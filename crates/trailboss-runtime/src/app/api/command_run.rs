use crate::{
    api::schema::{ErrorBody, Method, Request, UiCommandRunParams},
    app::App,
    core_actions::{CoreAction, NamedCommandRun},
};
use serde_json::json;

fn error(code: &str, message: &str) -> NamedCommandRun {
    ErrorBody {
        code: code.into(),
        message: message.into(),
    }
    .into()
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
            Err(NamedCommandRun::DiscoverTransfer { .. }) => super::responses::encode_error(
                id,
                "server_unavailable",
                "destination discovery requires a running Core",
            ),
            Err(NamedCommandRun::Error(error)) => super::responses::encode_error_body(id, error),
        }
    }

    /// Resolve before dispatch so ordinary operation admission, reservations and
    /// deferred handlers apply identically to named and direct requests.
    pub(crate) fn resolve_ui_command_run(
        &mut self,
        params: &UiCommandRunParams,
    ) -> Result<Method, NamedCommandRun> {
        let catalog = self.ui_command_catalog()?;
        // A stable identity disambiguates titles, including a title that happens
        // to spell another command's ID. Only title-only lookup is ambiguous.
        let selected = if let Some(command) = catalog
            .iter()
            .find(|command| command.key.matches_stable_id(&params.command))
        {
            command
        } else {
            let mut matches = catalog.iter().filter(|command| {
                command.title == params.command
                    && self.named_command_runnable(command, params.pane_id.is_some())
            });
            let selected = matches
                .next()
                .or_else(|| {
                    // A sole unavailable title still yields its actionable dispatch
                    // error, but cannot make an executable title ambiguous.
                    catalog
                        .iter()
                        .find(|command| command.title == params.command)
                })
                .ok_or_else(|| error("command_not_found", "command not found"))?;
            if matches.next().is_some() {
                let choices = catalog
                    .iter()
                    .filter(|command| {
                        command.title == params.command
                            && self.named_command_runnable(command, params.pane_id.is_some())
                    })
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
        if let crate::api::schema::UiCommandKey::Plugin {
            plugin_id,
            action_id,
        } = &selected.key
        {
            self.named_plugin_command_available(selected)?;
            if params.answers.len() > 1 {
                return Err(error(
                    "extra_answer",
                    "extra answer: command accepts no further answers",
                ));
            }
            let mut context = crate::api::schema::PluginInvocationContext {
                // The palette replacement path preserves absent fields instead
                // of merging Core-global focus. Legacy API invocation is unchanged.
                invocation_source: Some("plugin_ui".into()),
                ..Default::default()
            };
            if let Some(locator) = params.pane_id.as_deref() {
                let (ws, pane) = self.parse_pane_id(locator).ok_or_else(|| {
                    error(
                        "pane_not_found",
                        &format!("Trailboss pane is unavailable: {locator}"),
                    )
                })?;
                context.workspace_id = Some(self.public_workspace_id(ws));
                context.tab_id = self.state.workspaces[ws]
                    .find_tab_index_for_pane(pane)
                    .and_then(|tab| self.public_tab_id(ws, tab));
                context.pane_id = self.public_pane_id(ws, pane);
            } else if super::plugins::commands::plugin_requires_context(selected) {
                return Err(error(
                    "missing_context",
                    "no Trailboss pane context; run from a Trailboss pane",
                ));
            }
            return Ok(Method::PluginActionInvoke(
                crate::api::schema::PluginActionInvokeParams {
                    plugin_id: Some(plugin_id.clone()),
                    action_id: action_id.clone(),
                    arguments: params.answers.first().cloned(),
                    context: Some(context),
                },
            ));
        }
        let descriptor = crate::core_actions::command_descriptor(&selected.key)
            .filter(|descriptor| descriptor.runnable_by_name())
            .ok_or_else(|| {
                error(
                    "command_unavailable",
                    "command adapter is unavailable through the API",
                )
            })?;
        let action = descriptor.action;
        if action == CoreAction::RecentlyClosedTabs {
            let labels: Vec<_> = self
                .state
                .closed_tabs
                .iter()
                .map(|entry| {
                    crate::core_actions::closed_tab_choice_label(
                        &entry.label,
                        &entry.workspace_label,
                        entry.closed_at,
                    )
                })
                .collect();
            let index = crate::core_actions::select_command_choice(
                "Recently closed tabs",
                params.answers.first(),
                &labels,
            )
            .map_err(|(code, message)| error(code, &message))?;
            if params.answers.len() > 1 {
                return Err(error(
                    "extra_answer",
                    "extra answer: command accepts no further answers",
                ));
            }
            let entry = &self.state.closed_tabs[index];
            return Ok(Method::TabClosedRestore(
                crate::api::schema::ClosedTabRestoreParams {
                    closed_tab_id: entry.id.clone(),
                },
            ));
        }
        let expected = usize::from(matches!(
            descriptor.interaction,
            crate::core_actions::CoreCommandInteraction::Name { .. }
        ));
        if action != CoreAction::MoveToSession && params.answers.len() > expected {
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
        if action == CoreAction::MoveToSession {
            return Err(NamedCommandRun::DiscoverTransfer {
                tab_id: tab.expect("caller tab resolved"),
                answers: params.answers.clone(),
            });
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
