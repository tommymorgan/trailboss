use super::{helper, protocol::*, storage, transport};
use crate::api::schema::{Method, TabTransferParams, TabTransferPhase, TabTransferStatus};
use std::{io, path::PathBuf};
#[derive(serde::Serialize, serde::Deserialize)]
struct Operation {
    status: TabTransferStatus,
    source: CoreRegistration,
    destination: CoreRegistration,
    params: TabTransferParams,
    generation: u64,
    routes: Vec<helper::Route>,
}
fn path(id: &str) -> io::Result<PathBuf> {
    if id.trim().is_empty() || id.len() > 256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nonempty operation ID of at most 256 bytes required",
        ));
    }
    let key = storage::key(id);
    let mut directory = storage::root()?;
    let mut chunks = key.as_bytes().chunks(120).peekable();
    while let Some(chunk) = chunks.next() {
        let component = std::str::from_utf8(chunk).map_err(io::Error::other)?;
        if chunks.peek().is_none() {
            return Ok(directory.join(format!("op-{component}.json")));
        }
        directory = directory.join(format!("ops-{component}"));
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        storage::validate(&directory, false)?;
    }
    Err(storage::denied())
}
fn save(op: &Operation) -> io::Result<()> {
    storage::atomic(&path(&op.status.operation_id)?, op)
}
fn reconcile(op: &mut Operation) -> io::Result<()> {
    let id = op.status.operation_id.clone();
    let source = transport::call(
        &op.source,
        CoreRequest::OperationStatus {
            operation_id: id.clone(),
        },
        Vec::new(),
    );
    let destination = transport::call(
        &op.destination,
        CoreRequest::OperationStatus {
            operation_id: id.clone(),
        },
        Vec::new(),
    );
    let committed = matches!(destination.as_ref().map(|p|&p.reply),Ok(CoreReply::OperationStatus{destination_committed:true,generation:Some(g),..}) if *g==op.generation);
    if committed {
        // Complete the same recorded commit only; this never starts another transfer.
        if storage::alive(op.source.pid, op.source.start)
            && !matches!(
                source.as_ref().map(|p| &p.reply),
                Ok(CoreReply::OperationStatus {
                    source_committed: true,
                    ..
                })
            )
        {
            transport::call(
                &op.source,
                CoreRequest::CommitSource { operation_id: id },
                Vec::new(),
            )?;
        }
        let _lock = storage::lock(&storage::root()?.join("routes.lock"))?;
        let mut routes = helper::routes()?;
        for route in &op.routes {
            routes.retain(|old| {
                !(old.origin.public_endpoint == route.origin.public_endpoint
                    && old.origin_pane == route.origin_pane)
            });
            routes.push(route.clone());
        }
        helper::save_routes(&routes)?;
        op.status.phase = TabTransferPhase::Committed;
        op.status.error = None;
        save(op)?;
    } else {
        let destination_aborted = matches!(
            destination.as_ref().map(|packet| &packet.reply),
            Ok(CoreReply::OperationStatus { aborted: true, .. })
        );
        let destination_prepared = matches!(
            destination.as_ref().map(|packet| &packet.reply),
            Ok(CoreReply::OperationStatus {
                destination_prepared: true,
                destination_committed: false,
                ..
            })
        );
        let source_recoverable = matches!(
            source.as_ref().map(|packet| &packet.reply),
            Ok(CoreReply::OperationStatus {
                source_prepared: true,
                source_committed: false,
                ..
            }) | Ok(CoreReply::OperationStatus {
                aborted: true,
                source_committed: false,
                ..
            })
        );
        if destination_aborted || (destination_prepared && source_recoverable) {
            // Abort destination atomically before permitting the source reader
            // to resume. A queued commit then fails against the abort tombstone.
            if !destination_aborted
                && !matches!(
                    transport::call(
                        &op.destination,
                        CoreRequest::Abort {
                            operation_id: id.clone(),
                        },
                        Vec::new()
                    )?
                    .reply,
                    CoreReply::Acknowledged
                )
            {
                return Err(storage::denied());
            }
            if !matches!(
                transport::call(
                    &op.source,
                    CoreRequest::Abort { operation_id: id },
                    Vec::new()
                )?
                .reply,
                CoreReply::Acknowledged
            ) {
                return Err(storage::denied());
            }
            op.status.phase = TabTransferPhase::Aborted;
            save(op)?;
        }
    }
    Ok(())
}
pub(crate) fn transfer_status(id: &str) -> io::Result<TabTransferStatus> {
    let _lock = storage::lock(&path(id)?.with_extension("lock"))?;
    let mut op: Operation = storage::read(&path(id)?)?;
    if op.status.phase == TabTransferPhase::Unresolved {
        let _ = reconcile(&mut op);
    }
    Ok(op.status)
}
pub(crate) fn stored_transfer_status(id: &str) -> io::Result<TabTransferStatus> {
    let op: Operation = storage::read(&path(id)?)?;
    Ok(op.status)
}
#[derive(Clone, Copy)]
enum TransferFailure {
    Definitive,
    Unresolved,
    Conflict,
}
pub(crate) fn begin_transfer(source: CoreRegistration, message: crate::api::ApiRequestMessage) {
    std::thread::spawn(move || {
        let id = message.request.id.clone();
        let operation_id = match &message.request.method {
            Method::TabTransfer(params) => params.operation_id.clone().unwrap_or_default(),
            _ => String::new(),
        };
        let mut failure = TransferFailure::Unresolved;
        let result = (|| -> io::Result<crate::api::schema::ResponseResult> {
            let Method::TabTransfer(params) = message.request.method else {
                failure = TransferFailure::Definitive;
                return Err(storage::denied());
            };
            let status = coordinate(source, params, &mut failure)?;
            failure = if status.phase == TabTransferPhase::Aborted {
                TransferFailure::Definitive
            } else {
                TransferFailure::Unresolved
            };
            if status.phase != TabTransferPhase::Committed {
                return Err(io::Error::other(status.error.unwrap_or_else(|| {
                    match status.phase {
                        TabTransferPhase::Aborted => "transfer aborted before ownership commit",
                        _ => "transfer outcome unresolved",
                    }
                    .into()
                })));
            }
            Ok(crate::api::schema::ResponseResult::TabTransfer {
                transfer: status.transfer.ok_or_else(storage::denied)?,
            })
        })();
        let response = match result {
            Ok(result) => serde_json::json!({"id":id,"result":result}),
            Err(error) => {
                let code = match failure {
                    TransferFailure::Definitive => "tab_transfer_failed",
                    TransferFailure::Unresolved => "transfer_unresolved",
                    TransferFailure::Conflict => "transfer_operation_conflict",
                };
                let message = match failure {
                    TransferFailure::Unresolved => format!(
                        "{error}; operation_id={operation_id}; outcome unresolved; query the same operation ID, do not repeat the move"
                    ),
                    _ => error.to_string(),
                };
                serde_json::json!({"id":id,"error":{"code":code,"message":message}})
            }
        };
        let _ = message.respond_to.send(response.to_string());
    });
}
fn coordinate(
    source: CoreRegistration,
    params: TabTransferParams,
    failure: &mut TransferFailure,
) -> io::Result<TabTransferStatus> {
    let id = params.operation_id.as_deref().ok_or_else(|| {
        *failure = TransferFailure::Definitive;
        io::Error::other("cross-session transfer requires operation_id")
    })?;
    let op_path = path(id).inspect_err(|error| {
        if error.kind() == io::ErrorKind::InvalidInput {
            *failure = TransferFailure::Definitive;
        }
    })?;
    let _lock = storage::lock(&op_path.with_extension("lock"))?;
    if op_path.try_exists()? {
        let mut op: Operation = storage::read(&op_path)?;
        if op.source.boot != source.boot || op.params != params {
            *failure = TransferFailure::Conflict;
            return Err(io::Error::other(
                "operation ID already belongs to another transfer",
            ));
        }
        if op.status.phase == TabTransferPhase::Unresolved {
            let _ = reconcile(&mut op);
        }
        return Ok(op.status);
    }
    *failure = TransferFailure::Definitive;
    if params.workspace_id.is_some() == params.destination_workspace_label.is_some()
        || params
            .destination_workspace_label
            .as_ref()
            .is_some_and(|s| s.trim().is_empty())
    {
        return Err(io::Error::other(
            "select exactly one destination workspace or nonempty new-space label",
        ));
    }
    let destination_name = params
        .destination_session
        .as_deref()
        .ok_or_else(storage::denied)?;
    crate::session::parse_target_name(destination_name).map_err(io::Error::other)?;
    let destination = storage::registration(destination_name).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("destination session {destination_name:?} is unavailable: {error}"),
        )
    })?;
    if destination.boot == source.boot {
        return Err(io::Error::other("destination must be another Core"));
    }
    for core in [&source, &destination] {
        let packet = transport::call(core, CoreRequest::Inspect, Vec::new())?;
        if !matches!(&packet.reply,CoreReply::Inspected(inspected) if inspected.boot==core.boot && inspected.version==COORDINATION_VERSION)
        {
            return Err(storage::denied());
        }
    }
    helper::ensure()?;
    helper::adopt(&source)?;
    helper::adopt(&destination)?;
    if let Some(workspace) = &params.workspace_id {
        use std::io::{BufRead, Write};
        let mut stream = transport::connect(&destination.private_endpoint)?;
        if transport::peer(&stream)? != (destination.pid, destination.start) {
            return Err(storage::denied());
        }
        stream.write_all(b"{\"id\":\"transfer:validate-workspace\",\"method\":\"workspace.list\",\"params\":{}}\n")?;
        let mut response = String::new();
        std::io::BufReader::new(stream).read_line(&mut response)?;
        let response: serde_json::Value =
            serde_json::from_str(&response).map_err(io::Error::other)?;
        if !response
            .pointer("/result/workspaces")
            .and_then(|v| v.as_array())
            .is_some_and(|workspaces| {
                workspaces
                    .iter()
                    .any(|w| w.get("workspace_id").and_then(|v| v.as_str()) == Some(workspace))
            })
        {
            return Err(io::Error::other("destination workspace does not exist"));
        }
    }
    let generation = storage::next_generation()?;
    let mut op = Operation {
        status: TabTransferStatus {
            operation_id: id.to_string(),
            phase: TabTransferPhase::Preparing,
            source_session: source.session.clone(),
            destination_session: destination.session.clone(),
            transfer: None,
            error: None,
        },
        source: source.clone(),
        destination: destination.clone(),
        params: params.clone(),
        generation,
        routes: Vec::new(),
    };
    *failure = TransferFailure::Unresolved;
    save(&op)?;
    let prepared = (|| -> io::Result<()> {
        let packet = transport::call(
            &source,
            CoreRequest::PrepareSource {
                operation_id: id.to_string(),
                tab_id: params.tab_id.clone(),
            },
            Vec::new(),
        )?;
        let CoreReply::SourcePrepared(bundle) = packet.reply else {
            return Err(storage::denied());
        };
        if packet.descriptors.len() != bundle.panes.len() {
            return Err(storage::denied());
        }
        let packet = transport::call(
            &destination,
            CoreRequest::PrepareDestination {
                operation_id: id.to_string(),
                bundle,
                workspace_id: params.workspace_id.clone(),
                workspace_label: params.destination_workspace_label.clone(),
                insert_index: params.insert_index,
            },
            packet.descriptors,
        )?;
        let CoreReply::DestinationPrepared { transfer, panes } = packet.reply else {
            return Err(storage::denied());
        };
        op.status.transfer = Some(transfer);
        let _routes_lock = storage::lock(&storage::root()?.join("routes.lock"))?;
        let existing = helper::routes()?;
        for target in panes {
            let previous: Vec<_> = existing
                .iter()
                .filter(|r| {
                    r.target.terminal_id == target.terminal_id && r.target.process == target.process
                })
                .collect();
            for route in previous {
                let mut route = route.clone();
                route.owner = destination.clone();
                route.target = target.clone();
                route.generation = generation;
                route.operation_id = id.to_string();
                op.routes.push(route)
            }
            op.routes.push(helper::Route {
                origin: source.clone(),
                origin_pane: target.previous_pane_id.clone(),
                owner: destination.clone(),
                target,
                generation,
                operation_id: id.to_string(),
            });
        }
        save(&op)?;
        Ok(())
    })();
    if let Err(error) = prepared {
        let a = transport::call(
            &destination,
            CoreRequest::Abort {
                operation_id: id.to_string(),
            },
            Vec::new(),
        );
        let b = transport::call(
            &source,
            CoreRequest::Abort {
                operation_id: id.to_string(),
            },
            Vec::new(),
        );
        op.status.phase = if a.is_ok() && b.is_ok() {
            TabTransferPhase::Aborted
        } else {
            TabTransferPhase::Unresolved
        };
        op.status.error = Some(error.to_string());
        save(&op)?;
        return Ok(op.status);
    }
    // Persist uncertainty before sending the first ownership-changing message.
    op.status.phase = TabTransferPhase::Unresolved;
    save(&op)?;
    let commit = transport::call(
        &destination,
        CoreRequest::CommitDestination {
            operation_id: id.to_string(),
            generation,
        },
        Vec::new(),
    );
    if let Err(error) = commit {
        op.status.error = Some(error.to_string());
        save(&op)?;
        let _ = reconcile(&mut op);
        return Ok(op.status);
    }
    if let Err(error) = reconcile(&mut op) {
        op.status.error = Some(error.to_string());
        save(&op)?;
    }
    Ok(op.status)
}
