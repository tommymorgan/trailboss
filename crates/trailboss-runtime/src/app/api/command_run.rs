use crate::{
    api::schema::{ErrorBody, Method, Request, UiCommandRunParams},
    app::App,
    core_actions::CoreAction,
};
use serde_json::json;

fn error(code: &str, message: &str) -> ErrorBody {
    ErrorBody {
        code: code.into(),
        message: message.into(),
    }
}

impl App {
    pub(super) fn handle_ui_command_run(
        &mut self,
        id: String,
        params: UiCommandRunParams,
    ) -> String {
        match self.resolve_ui_command_run(&params) {
            Ok(method) => {
                self.handle_api_request_after_internal_events_drained(Request { id, method })
            }
            Err(error) => super::responses::encode_error_body(id, error),
        }
    }

    /// Resolve before dispatch so ordinary operation admission, reservations and
    /// deferred handlers apply identically to named and direct requests.
    pub(crate) fn resolve_ui_command_run(
        &mut self,
        params: &UiCommandRunParams,
    ) -> Result<Method, ErrorBody> {
        let catalog = self.ui_command_catalog()?;
        // A stable identity disambiguates titles, including a title that happens
        // to spell another command's ID. Only title-only lookup is ambiguous.
        let selected = if let Some(command) = catalog
            .iter()
            .find(|command| command.key.matches_stable_id(&params.command))
        {
            command
        } else {
            let mut matches = catalog
                .iter()
                .filter(|command| command.title == params.command);
            let selected = matches
                .next()
                .ok_or_else(|| error("command_not_found", "command not found"))?;
            if matches.next().is_some() {
                let choices = catalog
                    .iter()
                    .filter(|command| command.title == params.command)
                    .map(|command| format!("{} ({})", command.title, command.key.stable_id()))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(error(
                    "ambiguous_command",
                    &format!("ambiguous title: {choices}"),
                ));
            }
            selected
        };
        let descriptor = crate::core_actions::command_descriptor(&selected.key)
            .filter(|descriptor| descriptor.runnable_by_name())
            .ok_or_else(|| {
                error(
                    "command_unavailable",
                    "command adapter is unavailable through the API",
                )
            })?;
        let action = descriptor.action;
        let expected = usize::from(matches!(
            descriptor.interaction,
            crate::core_actions::CoreCommandInteraction::Name { .. }
        ));
        if params.answers.len() > expected {
            return Err(error(
                "extra_answer",
                "extra answer: command accepts no further answers",
            ));
        }
        let mut workspace = None;
        let mut tab = None;
        if !matches!(action, CoreAction::NewSpace | CoreAction::RestartIdleAgents) {
            let locator = params.pane_id.as_deref().ok_or_else(|| {
                error(
                    "missing_context",
                    "no Trailboss pane context; run from a Trailboss pane",
                )
            })?;
            let (ws, pane) = self.parse_pane_id(locator).ok_or_else(|| {
                error(
                    "pane_not_found",
                    &format!("Trailboss pane is unavailable: {locator}"),
                )
            })?;
            workspace = Some(self.public_workspace_id(ws));
            tab = self.state.workspaces[ws]
                .find_tab_index_for_pane(pane)
                .and_then(|t| self.public_tab_id(ws, t));
        }
        let (method, values) = crate::core_actions::prepare_named(
            action,
            workspace.as_deref(),
            tab.as_deref(),
            params.answers.first().map(String::as_str),
        )
        .map_err(|(code, message)| error(code, message))?;
        Ok(
            serde_json::from_value(json!({"method":method,"params":values}))
                .expect("named Core operation parameters"),
        )
    }
}
