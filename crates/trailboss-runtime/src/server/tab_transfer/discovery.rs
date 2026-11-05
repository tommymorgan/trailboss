use super::{
    helper,
    protocol::{CoreRegistration, CoreReply, CoreRequest, COORDINATION_VERSION},
    storage, transport,
};
use std::{
    io::{self, BufRead, Read, Write},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TransferDestination {
    pub session: String,
    pub workspace_id: Option<String>,
    pub workspace_label: Option<String>,
}
/// Worker-only read. Two-second discovery budget; each authenticated Core
/// receives at most 300ms of socket inactivity time. No transfer preparation,
/// listener adoption, focus change, or replacement Core is started here.
pub(crate) fn list_destinations(source_session: &str) -> io::Result<Vec<TransferDestination>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut cores = Vec::new();
    for entry in std::fs::read_dir(storage::root()?)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("core-") || !name.ends_with(".json") {
            continue;
        }
        let Ok(core) = storage::read::<CoreRegistration>(&entry.path()) else {
            continue;
        };
        if core.session == source_session {
            continue;
        }
        let Ok(registered) = storage::registration(&core.session) else {
            continue;
        };
        if registered.boot == core.boot {
            cores.push(registered)
        }
    }
    cores.sort_by(|a, b| a.session.cmp(&b.session));
    let mut destinations = Vec::new();
    for core in cores {
        let inspected =
            transport::call_timeout(&core, CoreRequest::Inspect, Vec::new(), budget(deadline)?);
        if !matches!(inspected.map(|packet|packet.reply),Ok(CoreReply::Inspected(registered)) if registered.boot==core.boot&&registered.version==COORDINATION_VERSION)
        {
            continue;
        }
        let timeout = budget(deadline)?;
        let queried = (|| -> io::Result<Vec<crate::api::schema::WorkspaceInfo>> {
            let mut stream = transport::connect_timeout(&core.public_endpoint, timeout)?;
            helper::validate_public_peer(&stream, &core, budget(deadline)?)?;
            stream.write_all(
                b"{\"id\":\"transfer:destinations\",\"method\":\"workspace.list\",\"params\":{}}\n",
            )?;
            let mut line = String::new();
            io::BufReader::new(stream)
                .take(4 * 1024 * 1024)
                .read_line(&mut line)?;
            let response: crate::api::schema::SuccessResponse =
                serde_json::from_str(&line).map_err(io::Error::other)?;
            let crate::api::schema::ResponseResult::WorkspaceList { workspaces } = response.result
            else {
                return Err(storage::denied());
            };
            Ok(workspaces)
        })();
        let Ok(workspaces) = queried else { continue };
        if !storage::registration(&core.session)
            .is_ok_and(|registered| registered.boot == core.boot)
        {
            continue;
        }
        if workspaces.is_empty() {
            destinations.push(TransferDestination {
                session: core.session,
                workspace_id: None,
                workspace_label: None,
            })
        } else {
            for workspace in workspaces {
                destinations.push(TransferDestination {
                    session: core.session.clone(),
                    workspace_id: Some(workspace.workspace_id),
                    workspace_label: Some(workspace.label),
                })
            }
        }
    }
    if Instant::now() > deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "destination discovery deadline exceeded",
        ));
    }
    Ok(destinations)
}
fn budget(deadline: Instant) -> io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "destination discovery deadline exceeded",
        ));
    }
    Ok(remaining.min(Duration::from_millis(300)))
}
