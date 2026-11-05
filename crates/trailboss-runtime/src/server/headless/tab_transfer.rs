use super::HeadlessServer;
use crate::api::schema::{Method, ResponseResult};
use crate::server::tab_transfer::protocol::*;

impl HeadlessServer {
    pub(super) fn refresh_live_transfer_routes(&self) -> std::io::Result<()> {
        if let Some(control) = &self.transfer_control {
            crate::server::tab_transfer::refresh_owned_routes(
                control.registration(),
                &self.app.owned_live_routes(),
            )?;
        }
        Ok(())
    }
    pub(super) fn activate_tab_popup(&mut self, target: Option<String>) {
        if self.popup_owner_tab_id == target && self.app.state.popup_pane.is_some() {
            return;
        }
        if let Some(popup) = self.app.state.popup_pane.take() {
            let owner = self.popup_owner_tab_id.take().or_else(|| {
                self.default_shell_target().and_then(|target| {
                    self.app
                        .public_tab_id(target.workspace_index, target.tab_index)
                })
            });
            if let Some(owner) = owner {
                self.app.live_transfers.parked_popups.insert(owner, popup);
            } else {
                self.app.state.popup_pane = Some(popup);
                return;
            }
        }
        self.popup_owner_tab_id = None;
        if let Some(target) = target {
            if let Some(popup) = self.app.live_transfers.parked_popups.remove(&target) {
                self.app.state.popup_pane = Some(popup);
                self.popup_owner_tab_id = Some(target);
            }
        }
    }
    pub(super) fn intercept_live_transfer_api(&self, method: &Method) -> bool {
        matches!(method, Method::TabTransfer(params) if params.destination_session.is_some())
            || matches!(method, Method::TabTransferStatus(_))
    }
    pub(super) fn dispatch_live_transfer_api(&mut self, message: crate::api::ApiRequestMessage) {
        if let Method::TabTransferStatus(params) = &message.request.method {
            let operation_id = params.operation_id.clone();
            let failed = message.respond_to.clone();
            let failed_id = message.request.id.clone();
            if let Err(error) =
                crate::thread_spawn::spawn_named("trailboss-transfer-status", move || {
                    let response = match crate::server::tab_transfer::transfer_status(&operation_id)
                    {
                        Ok(status) => serde_json::to_string(&crate::api::schema::SuccessResponse {
                            id: message.request.id,
                            result: ResponseResult::TabTransferStatus { status },
                        }),
                        Err(error) => serde_json::to_string(&crate::api::schema::ErrorResponse {
                            id: message.request.id,
                            error: crate::api::schema::ErrorBody {
                                code: "transfer_unavailable".into(),
                                message: error.to_string(),
                            },
                        }),
                    };
                    if let Ok(response) = response {
                        let _ = message.respond_to.send(response);
                    }
                })
            {
                let _ = failed.send(serde_json::json!({"id": failed_id, "error": {"code": "transfer_unavailable", "message": error.to_string()}}).to_string());
            }
            return;
        }
        if let Some(control) = &self.transfer_control {
            crate::server::tab_transfer::begin_transfer(control.registration().clone(), message);
        } else {
            let response = serde_json::to_string(&crate::api::schema::ErrorResponse {
                id: message.request.id,
                error: crate::api::schema::ErrorBody {
                    code: "transfer_unavailable".into(),
                    message: "Core coordination unavailable".into(),
                },
            });
            if let Ok(response) = response {
                let _ = message.respond_to.send(response);
            }
        }
    }
    pub(super) fn handle_transfer_envelope(&mut self, envelope: CoreEnvelope) -> bool {
        if let CoreRequest::Forward {
            terminal_id,
            process,
            generation,
            request,
        } = &envelope.request
        {
            let prepared = self
                .app
                .validate_forward(terminal_id, *generation, process)
                .and_then(|()| {
                    self.app
                        .canonical_forward_request(terminal_id, request.clone())
                });
            let Some(control) = &self.transfer_control else {
                let _ = envelope
                    .respond_to
                    .send(Err("Core coordination unavailable".into()));
                return false;
            };
            let endpoint = control.registration().private_endpoint.clone();
            let admission = crate::server::tab_transfer::RouteAdmission {
                terminal_id: terminal_id.clone(),
                process: process.clone(),
                generation: *generation,
            };
            match prepared {
                Err(error) => {
                    let _ = envelope.respond_to.send(Err(error));
                }
                Ok(request) => {
                    let respond_to = envelope.respond_to;
                    let failed = respond_to.clone();
                    if let Err(error) =
                        crate::thread_spawn::spawn_named("trailboss-route-request", move || {
                            let result =
                                crate::api::request_guarded(&endpoint, &request, &admission)
                                    .map(|response| CorePacket {
                                        reply: CoreReply::Response(response),
                                        descriptors: Vec::new(),
                                    })
                                    .map_err(|error| error.to_string());
                            let _ = respond_to.send(result);
                        })
                    {
                        let _ = failed.send(Err(error.to_string()));
                    }
                }
            }
            return false;
        }
        let result = self.execute_transfer_request(envelope.request, envelope.descriptors);
        let changed = result.is_ok();
        let _ = envelope.respond_to.send(result);
        if changed {
            self.reconcile_client_shell_locations();
        }
        changed
    }
    fn execute_transfer_request(
        &mut self,
        request: CoreRequest,
        descriptors: Vec<std::os::fd::OwnedFd>,
    ) -> Result<CorePacket, String> {
        let registration = self
            .transfer_control
            .as_ref()
            .ok_or("Core coordination unavailable")?
            .registration()
            .clone();
        let mut outgoing = Vec::new();
        let reply = match request {
            CoreRequest::Inspect => CoreReply::Inspected(registration),
            CoreRequest::PrepareFrontDoor => {
                let server = self.api_server.as_mut().ok_or("API service unavailable")?;
                if self.transfer_private_api.is_none() {
                    self.transfer_private_api = Some(
                        server
                            .start_private_at(&registration.private_endpoint)
                            .map_err(|e| e.to_string())?,
                    );
                }
                outgoing.push(
                    server
                        .quiesce_public_acceptor(std::time::Duration::from_secs(3))
                        .map_err(|e| e.to_string())?,
                );
                CoreReply::FrontDoorReady(registration)
            }
            CoreRequest::CommitFrontDoor => {
                self.api_server
                    .as_mut()
                    .ok_or("API service unavailable")?
                    .commit_public_adoption();
                CoreReply::Acknowledged
            }
            CoreRequest::ResumeFrontDoor => {
                self.api_server
                    .as_mut()
                    .ok_or("API service unavailable")?
                    .resume_public_acceptor()
                    .map_err(|e| e.to_string())?;
                CoreReply::Acknowledged
            }
            CoreRequest::PrepareSource {
                operation_id,
                tab_id,
            } => {
                let include_popup = self.popup_owner_tab_id.as_deref() == Some(tab_id.as_str());
                let (bundle, fds) = self.app.prepare_live_source(
                    &operation_id,
                    &tab_id,
                    registration,
                    include_popup,
                )?;
                outgoing = fds;
                CoreReply::SourcePrepared(bundle)
            }
            CoreRequest::PrepareDestination {
                operation_id,
                bundle,
                workspace_id,
                workspace_label,
                insert_index,
            } => {
                let (transfer, panes) = self.app.prepare_live_destination(
                    &operation_id,
                    bundle,
                    descriptors,
                    workspace_id,
                    workspace_label,
                    insert_index,
                )?;
                CoreReply::DestinationPrepared { transfer, panes }
            }
            CoreRequest::CommitSource { operation_id } => {
                self.app.commit_live_source(&operation_id)?;
                CoreReply::Acknowledged
            }
            CoreRequest::CommitDestination {
                operation_id,
                generation,
            } => {
                self.app
                    .commit_live_destination(&operation_id, generation)?;
                CoreReply::Acknowledged
            }
            CoreRequest::Abort { operation_id } => {
                self.app.abort_live_transfer(&operation_id)?;
                CoreReply::Acknowledged
            }
            CoreRequest::OperationStatus { operation_id } => {
                self.app.live_transfer_operation_status(&operation_id)
            }
            CoreRequest::Forward { .. } => {
                return Err("forward requests require asynchronous admission".into())
            }
        };
        Ok(CorePacket {
            reply,
            descriptors: outgoing,
        })
    }
}
