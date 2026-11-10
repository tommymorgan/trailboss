use super::*;
use crate::protocol::endpoint::TAB_TRANSFER_DESTINATIONS_RESPONSE_KIND;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(serde::Deserialize)]
struct DiscoveryRequest {
    request_id: String,
    boot_id: String,
}

#[derive(serde::Serialize)]
struct DiscoveryResponse<'a> {
    request_id: &'a str,
    boot_id: &'a str,
    destinations: &'a [crate::server::tab_transfer::TransferDestination],
    error: Option<String>,
}

fn response(
    request: &DiscoveryRequest,
    destinations: &[crate::server::tab_transfer::TransferDestination],
    error: Option<String>,
) -> String {
    serde_json::to_string(&DiscoveryResponse {
        request_id: &request.request_id,
        boot_id: &request.boot_id,
        destinations,
        error,
    })
    .expect("destination discovery contains only JSON-compatible values")
}

fn failure(request: &DiscoveryRequest, error: impl Into<String>) -> String {
    response(request, &[], Some(error.into()))
}

impl HeadlessServer {
    pub(super) fn discover_named_transfer(
        &self,
        message: api::ApiRequestMessage,
        tab_id: String,
        answers: Vec<String>,
    ) {
        let failed = message.respond_to.clone();
        let failed_id = message.request.id.clone();
        let source = crate::session::active_name()
            .unwrap_or_else(|| crate::session::DEFAULT_SESSION_NAME.to_owned());
        let sender = self.server_event_tx.clone();
        let result = crate::thread_spawn::spawn_named(
            "trailboss-command-destinations",
            move || {
                let resolved = crate::server::tab_transfer::list_destinations(&source)
                    .map_err(|error| ("discovery_failed", error.to_string()))
                    .and_then(|destinations| {
                        let labels: Vec<_> = destinations
                            .iter()
                            .map(|destination| {
                                crate::core_actions::transfer_destination_label(
                                    &destination.session,
                                    destination.workspace_label.as_deref(),
                                )
                            })
                            .collect();
                        let index = crate::core_actions::select_command_choice(
                            "Move to session",
                            answers.first(),
                            &labels,
                        )?;
                        let destination = &destinations[index];
                        let destination_workspace_label = if destination.workspace_id.is_none() {
                            let name = answers.get(1).ok_or_else(|| (
                                "missing_answer", "New destination space name: text answer required".to_owned(),
                            ))?;
                            if name.trim().is_empty() {
                                return Err(("invalid_name", "New destination space name: A destination space name is required".to_owned()));
                            }
                            Some(name.clone())
                        } else {
                            None
                        };
                        let expected = if destination.workspace_id.is_none() { 2 } else { 1 };
                        if answers.len() > expected {
                            return Err(("extra_answer", "extra answer: selected destination accepts no further answers".to_owned()));
                        }
                        Ok(api::schema::TabTransferParams {
                                tab_id,
                                workspace_id: destination.workspace_id.clone(),
                                destination_session: Some(destination.session.clone()),
                                destination_workspace_label,
                                operation_id: Some(crate::core_actions::named_transfer_operation_id(&message.request.id)),
                                insert_index: None,
                        })
                    });
                match resolved {
                    Ok(params) => {
                        let _ = sender.blocking_send(ServerEvent::NamedTransferPrepared {
                            message: Box::new(message),
                            params,
                        });
                    }
                    Err((code, diagnostic)) => {
                        let _ = message.respond_to.send(
                            serde_json::json!({
                                "id": message.request.id,
                                "error": {"code":code,"message":diagnostic},
                            })
                            .to_string(),
                        );
                    }
                }
            },
        );
        if let Err(error) = result {
            let _ = failed.send(
                serde_json::json!({
                    "id": failed_id,
                    "error": {"code":"discovery_failed","message":error.to_string()},
                })
                .to_string(),
            );
        }
    }

    pub(super) fn handle_tab_transfer_destinations(&mut self, client_id: u64, data: String) {
        let Some(client) = self.clients.get(&client_id) else {
            return;
        };
        if !matches!(client.mode, ClientConnectionMode::ClientShell) {
            return;
        }
        let request = match serde_json::from_str::<DiscoveryRequest>(&data) {
            Ok(request) if request.request_id.len() <= 256 && request.boot_id.len() <= 256 => {
                request
            }
            _ => {
                let invalid = DiscoveryRequest {
                    request_id: String::new(),
                    boot_id: self.client_shell_boot_id.clone(),
                };
                self.send_tab_transfer_discovery(
                    client_id,
                    failure(
                        &invalid,
                        "invalid_request: expected bounded request_id and boot_id",
                    ),
                );
                return;
            }
        };
        let error = if !client.shell_tab_transfer {
            Some("unsupported_capability: tab_transfer was not negotiated")
        } else if request.boot_id != self.client_shell_boot_id {
            Some("stale_boot: discovery targeted an earlier Core boot")
        } else if client.shell_tab_transfer_discovery_in_flight {
            Some("discovery_busy: another destination discovery is in progress")
        } else {
            None
        };
        if let Some(error) = error {
            self.send_tab_transfer_discovery(client_id, failure(&request, error));
            return;
        }
        let source = crate::session::active_name()
            .unwrap_or_else(|| crate::session::DEFAULT_SESSION_NAME.to_owned());
        let events = self.server_event_tx.clone();
        let boot_id = request.boot_id.clone();
        let failed = failure(
            &request,
            "server_unavailable: could not start destination discovery",
        );
        match crate::thread_spawn::spawn_named("trailboss-transfer-discovery", move || {
            let data = match crate::server::tab_transfer::list_destinations(&source) {
                Ok(destinations) => {
                    let encoded = response(&request, &destinations, None);
                    if encoded.len() > MAX_RESPONSE_BYTES {
                        failure(
                            &request,
                            "response_too_large: destination list exceeds publication limit",
                        )
                    } else {
                        encoded
                    }
                }
                Err(error) => failure(&request, format!("discovery_failed: {error}")),
            };
            let _ = events.blocking_send(ServerEvent::ClientShellTabTransferDestinationsReady {
                client_id,
                boot_id,
                data,
            });
        }) {
            Ok(_) => {
                if let Some(client) = self.clients.get_mut(&client_id) {
                    client.shell_tab_transfer_discovery_in_flight = true;
                }
            }
            Err(_) => self.send_tab_transfer_discovery(client_id, failed),
        }
    }

    pub(super) fn finish_tab_transfer_destinations(
        &mut self,
        client_id: u64,
        boot_id: String,
        data: String,
    ) {
        if boot_id != self.client_shell_boot_id {
            return;
        }
        let Some(client) = self.clients.get_mut(&client_id) else {
            return;
        };
        if !matches!(client.mode, ClientConnectionMode::ClientShell) || !client.shell_tab_transfer {
            return;
        }
        client.shell_tab_transfer_discovery_in_flight = false;
        self.send_tab_transfer_discovery(client_id, data);
    }

    fn send_tab_transfer_discovery(&mut self, client_id: u64, data: String) {
        self.send_to_client(
            client_id,
            ServerMessage::EndpointControl {
                kind: TAB_TRANSFER_DESTINATIONS_RESPONSE_KIND.to_owned(),
                data,
            },
        );
    }
}
