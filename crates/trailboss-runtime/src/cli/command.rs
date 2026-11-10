use crate::api::schema::{Method, Request, UiCommandInfo, UiCommandListParams, UiCommandRunParams};

/// Start of the opaque selector/answer tail after leading launch options.
/// Global extractors must not interpret any token in this command's payload.
pub fn command_run_payload_start(args: &[String]) -> Option<usize> {
    let mut index = 1;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "command" if args.get(index + 1).is_some_and(|arg| arg == "run") => {
                return Some(index + 2);
            }
            "--session"
            | "--remote"
            | "--remote-keybindings"
            | "--remote-executable"
            | "--machine" => {
                index += 2;
            }
            option if option.starts_with('-') && option != "--" => index += 1,
            _ => return None,
        }
    }
    None
}

pub(super) fn run(args: &[String]) -> std::io::Result<i32> {
    if args.len() == 1 && args[0] == "list" {
        let response = super::send_request(&Request {
            id: "cli:command:list".into(),
            method: Method::UiCommandList(UiCommandListParams {
                pane_id: std::env::var("HERDR_PANE_ID").ok(),
            }),
        })?;
        if response.get("error").is_some() {
            return super::print_response(&response);
        }
        let commands: Vec<UiCommandInfo> =
            serde_json::from_value(response["result"]["commands"].clone())
                .map_err(std::io::Error::other)?;
        for command in commands {
            println!(
                "{}\t{}\t{}",
                command.title,
                command.key.stable_id(),
                command.argument_hint.as_deref().unwrap_or("")
            );
        }
        return Ok(0);
    }
    let Some((operation, payload)) = args.split_first() else {
        eprintln!("usage: trailboss command run <title-or-id> [answer ...]");
        return Ok(2);
    };
    if operation != "run" {
        eprintln!("unknown command operation: {operation}");
        return Ok(2);
    }
    // Consume one option terminator, wherever it precedes the positional tail.
    // Subsequent `--` tokens are literal selector/answer text.
    let mut separator_seen = false;
    let mut positional = payload.iter().filter(|arg| {
        if !separator_seen && arg.as_str() == "--" {
            separator_seen = true;
            false
        } else {
            true
        }
    });
    let Some(command) = positional.next() else {
        eprintln!("usage: trailboss command run <title-or-id> [answer ...]");
        return Ok(2);
    };
    let response = super::send_request(&Request {
        id: "cli:command".into(),
        method: Method::UiCommandRun(UiCommandRunParams {
            command: command.clone(),
            answers: positional.cloned().collect(),
            pane_id: std::env::var("HERDR_PANE_ID").ok(),
        }),
    })?;
    super::print_response(&response)
}
