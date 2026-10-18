//! Structured chat-submit and turn-abort for running OMP agents, delivered
//! over the reporter control socket instead of the PTY.

use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::api::schema::{
    AgentChatSubmitParams, AgentChatSubmitResult, AgentTurnAbortParams, AgentTurnAbortResult,
    ResponseResult,
};

use super::restart_idle::{
    control_roundtrip, exact_session_path, prepare_control_target, ControlReply,
};
use super::responses::{encode_error, encode_error_body, encode_success};
use super::App;

/// Reporter-side chat input cap; must not exceed the socket frame we send.
const CHAT_TEXT_LIMIT: usize = 128 * 1024;
/// Chat frames carry user text, so they get a larger cap than control frames.
const CHAT_FRAME_LIMIT: usize = 256 * 1024;
const IDEMPOTENCY_KEY_LIMIT: usize = 256;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct ControlCheck {
    target: String,
    shell_pid: u32,
    session_path: String,
}

impl App {
    pub(super) fn start_agent_chat_submit(
        &mut self,
        id: String,
        respond_to: Sender<String>,
        params: AgentChatSubmitParams,
    ) {
        let check = match self.control_check(&id, &params.target) {
            Ok(check) => check,
            Err(response) => return drop(respond_to.send(response)),
        };
        if let Err(reason) = validate_chat(&params) {
            return drop(respond_to.send(encode_error(id, "invalid_params", reason)));
        }
        let spawned = crate::thread_spawn::spawn_named("trailboss-agent-chat", move || {
            let response = match chat_submit_roundtrip(&check, &params) {
                Ok(result) => encode_success(id, ResponseResult::AgentChatSubmitted { result }),
                Err((code, message)) => encode_error(id, &code, message),
            };
            let _ = respond_to.send(response);
        });
        if spawned.is_err() {
            tracing::error!("failed to spawn trailboss-agent-chat thread");
        }
    }

    pub(super) fn start_agent_turn_abort(
        &mut self,
        id: String,
        respond_to: Sender<String>,
        params: AgentTurnAbortParams,
    ) {
        let check = match self.control_check(&id, &params.target) {
            Ok(check) => check,
            Err(response) => return drop(respond_to.send(response)),
        };
        let spawned = crate::thread_spawn::spawn_named("trailboss-agent-abort", move || {
            let response = match turn_abort_roundtrip(&check) {
                Ok(result) => encode_success(id, ResponseResult::AgentTurnAborted { result }),
                Err((code, message)) => encode_error(id, &code, message),
            };
            let _ = respond_to.send(response);
        });
        if spawned.is_err() {
            tracing::error!("failed to spawn trailboss-agent-abort thread");
        }
    }

    /// Shared admission: resolve the agent, its saved OMP session, and the
    /// pane's shell pid before any control traffic is attempted. The Err side
    /// is a fully encoded error response ready to send.
    fn control_check(&mut self, id: &str, target: &str) -> Result<ControlCheck, String> {
        self.reconcile_managed_agent_target(target);
        let agent = match self.agent_info_for_target(target) {
            Ok(agent) => agent,
            Err(err) => {
                return Err(encode_error_body(
                    id.to_owned(),
                    self.agent_target_error_body(err),
                ))
            }
        };
        if !cfg!(target_os = "linux") {
            return Err(encode_error(
                id.to_owned(),
                "structured_unavailable",
                "unsupported_platform",
            ));
        }
        let Some(session_path) = exact_session_path(&agent) else {
            return Err(encode_error(
                id.to_owned(),
                "structured_unavailable",
                "unresolved_session_path",
            ));
        };
        let shell_pid = self.agent_shell_pid(&agent).ok_or_else(|| {
            encode_error(id.to_owned(), "structured_unavailable", "pane_unavailable")
        })?;
        Ok(ControlCheck {
            target: target.to_owned(),
            shell_pid,
            session_path,
        })
    }

    fn agent_shell_pid(&self, agent: &crate::api::schema::AgentInfo) -> Option<u32> {
        let resolved = self.resolve_terminal_target(&agent.terminal_id).ok()?;
        let runtime = self.lookup_runtime_sender(resolved.ws_idx, resolved.pane_id)?;
        runtime.child_pid()
    }
}

fn validate_chat(params: &AgentChatSubmitParams) -> Result<(), String> {
    if params.text.trim().is_empty() {
        return Err("text must not be empty".into());
    }
    if params.text.len() > CHAT_TEXT_LIMIT {
        return Err(format!("text exceeds {CHAT_TEXT_LIMIT} bytes"));
    }
    if params.idempotency_key.trim().is_empty() {
        return Err("idempotency_key must not be empty".into());
    }
    if params.idempotency_key.len() > IDEMPOTENCY_KEY_LIMIT {
        return Err(format!("idempotency_key exceeds {IDEMPOTENCY_KEY_LIMIT} bytes"));
    }
    if params
        .idempotency_key
        .chars()
        .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("idempotency_key must not contain control or whitespace characters".into());
    }
    Ok(())
}

fn chat_submit_roundtrip(
    check: &ControlCheck,
    params: &AgentChatSubmitParams,
) -> Result<AgentChatSubmitResult, (String, String)> {
    let target =
        prepare_control_target(check.shell_pid, &check.session_path).map_err(control_error)?;
    let request = serde_json::json!({
        "operation": "chat_submit",
        "pid": target.omp.pid,
        "pane_id": target.pane_identity,
        "session_path": check.session_path,
        "text": params.text,
        "idempotency_key": params.idempotency_key,
    });
    let reply = control_roundtrip(&target, request, CHAT_FRAME_LIMIT, CONTROL_TIMEOUT)
        .map_err(control_error)?;
    chat_result_from_reply(check, &reply)
}

fn chat_result_from_reply(
    check: &ControlCheck,
    reply: &ControlReply,
) -> Result<AgentChatSubmitResult, (String, String)> {
    match reply.status.as_str() {
        "accepted" => Ok(AgentChatSubmitResult {
            target: check.target.clone(),
            turn_id: reply
                .turn_id
                .clone()
                .ok_or_else(|| control_error("missing_turn_id".into()))?,
            deduplicated: reply.reason == "duplicate",
        }),
        _ => Err(skip_error(&reply)),
    }
}

fn turn_abort_roundtrip(check: &ControlCheck) -> Result<AgentTurnAbortResult, (String, String)> {
    let target =
        prepare_control_target(check.shell_pid, &check.session_path).map_err(control_error)?;
    let request = serde_json::json!({
        "operation": "abort",
        "pid": target.omp.pid,
        "pane_id": target.pane_identity,
        "session_path": check.session_path,
    });
    let reply = control_roundtrip(&target, request, CHAT_FRAME_LIMIT, CONTROL_TIMEOUT)
        .map_err(control_error)?;
    match reply.status.as_str() {
        "accepted" => Ok(AgentTurnAbortResult {
            target: check.target.clone(),
            aborted: true,
        }),
        // An idle agent has no turn to abort; report that as a completed no-op
        // so callers can retry freely.
        "skipped" if reply.reason == "agent-not-working" => Ok(AgentTurnAbortResult {
            target: check.target.clone(),
            aborted: false,
        }),
        _ => Err(skip_error(&reply)),
    }
}

/// Maps transport/preparation failures onto stable error codes. Missing or
/// unreachable control sockets mean the installed reporter cannot do
/// structured chat (v11 has no socket; v12 has restart only).
fn control_error(reason: String) -> (String, String) {
    if reason == "unsupported_control_endpoint" || reason.starts_with("control_connect_failed") {
        return (
            "structured_unavailable".into(),
            format!("reporter does not support structured control ({reason})"),
        );
    }
    if reason == "process_identity_changed" {
        return ("agent_process_changed".into(), reason);
    }
    ("control_unavailable".into(), reason)
}

fn skip_error(reply: &ControlReply) -> (String, String) {
    if matches!(reply.reason.as_str(), "invalid-request" | "unsupported-operation") {
        return (
            "structured_unavailable".into(),
            format!("agent reporter rejected the operation ({})", reply.reason),
        );
    }
    ("control_skipped".into(), reply.reason.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: &str, reason: &str, turn_id: Option<&str>) -> ControlReply {
        ControlReply {
            status: status.into(),
            reason: reason.into(),
            turn_id: turn_id.map(str::to_owned),
        }
    }

    fn check() -> ControlCheck {
        ControlCheck {
            target: "agent-1".into(),
            shell_pid: 1,
            session_path: "/s".into(),
        }
    }

    fn params(text: &str, key: &str) -> AgentChatSubmitParams {
        AgentChatSubmitParams {
            target: "agent-1".into(),
            text: text.into(),
            idempotency_key: key.into(),
        }
    }

    #[test]
    fn chat_params_validate_text_and_idempotency_key() {
        assert_eq!(validate_chat(&params("hello", "k1")), Ok(()));
        assert!(validate_chat(&params("  ", "k1")).is_err());
        assert!(validate_chat(&params("x", "has space")).is_err());
        assert!(validate_chat(&params("x", "k\u{7}")).is_err());
        assert!(validate_chat(&params(&"x".repeat(CHAT_TEXT_LIMIT + 1), "k")).is_err());
        assert!(validate_chat(&params("x", &"k".repeat(IDEMPOTENCY_KEY_LIMIT + 1))).is_err());
    }

    #[test]
    fn missing_socket_and_invalid_request_map_to_structured_unavailable() {
        assert_eq!(
            control_error("unsupported_control_endpoint".into()).0,
            "structured_unavailable"
        );
        assert_eq!(
            control_error("control_connect_failed: os error 2".into()).0,
            "structured_unavailable"
        );
        assert_eq!(
            skip_error(&reply("skipped", "invalid-request", None)).0,
            "structured_unavailable"
        );
        assert_eq!(
            skip_error(&reply("skipped", "unsupported-operation", None)).0,
            "structured_unavailable"
        );
    }

    #[test]
    fn accepted_reply_carries_turn_id_and_dedupe_flag() {
        let duplicate =
            chat_result_from_reply(&check(), &reply("accepted", "duplicate", Some("turn-9")))
                .expect("accepted reply maps to a result");
        assert_eq!(duplicate.turn_id, "turn-9");
        assert!(duplicate.deduplicated);

        let fresh = chat_result_from_reply(&check(), &reply("accepted", "", Some("turn-1")))
            .expect("accepted reply maps to a result");
        assert_eq!(fresh.turn_id, "turn-1");
        assert!(!fresh.deduplicated);

        assert!(chat_result_from_reply(&check(), &reply("accepted", "", None)).is_err());
    }

    #[test]
    fn abort_on_idle_is_completed_noop_and_other_skips_error() {
        let idle = reply("skipped", "agent-not-working", None);
        assert_eq!(skip_error(&idle).0, "control_skipped");
        let rejected = reply("skipped", "invalid-request", None);
        assert_eq!(skip_error(&rejected).0, "structured_unavailable");
    }
}
