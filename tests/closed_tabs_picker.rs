#![cfg(unix)]
mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use std::time::{Duration, Instant};
use trailboss::{
    core::Core as NativeCore,
    terminal::{menus::Response, FrameRenderer, Rect},
};

fn rows(
    renderer: &mut FrameRenderer,
    native: &NativeCore,
    client: &trailboss::Client,
) -> Vec<String> {
    let controls = std::mem::take(&mut renderer.controls);
    *renderer = FrameRenderer::default();
    renderer.controls = controls;
    let mut bytes = Vec::new();
    renderer
        .draw_frame(
            &mut bytes,
            None,
            client.declarations.as_ref().unwrap(),
            native,
            Rect::viewport(100, 30),
        )
        .unwrap();
    let mut screen = ghostty_vt::Terminal::new(100, 30, 0).unwrap();
    screen.write(&bytes);
    screen
        .screen_text_rows()
        .unwrap()
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .flat_map(|cell| {
                    cell.graphemes
                        .iter()
                        .copied()
                        .chain(cell.graphemes.is_empty().then_some(32))
                })
                .filter_map(char::from_u32)
                .collect()
        })
        .collect()
}

fn key(
    renderer: &mut FrameRenderer,
    native: &NativeCore,
    client: &trailboss::Client,
    code: KeyCode,
) -> Response {
    renderer.controls.key(
        native,
        client.declarations.as_ref().unwrap(),
        Rect::viewport(100, 30),
        KeyEvent::new(code, KeyModifiers::NONE),
    )
}

fn click_text(
    renderer: &mut FrameRenderer,
    native: &NativeCore,
    client: &trailboss::Client,
    text: &str,
) -> Response {
    let painted = rows(renderer, native, client);
    let (y, line) = painted
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains(text))
        .unwrap_or_else(|| panic!("missing visible {text}: {painted:?}"));
    let x = line[..line.find(text).unwrap()].chars().count();
    renderer.controls.click(
        native,
        client.declarations.as_ref().unwrap(),
        Rect::viewport(100, 30),
        x as u16,
        y as u16,
        false,
    )
}

fn open_picker(
    renderer: &mut FrameRenderer,
    native: &NativeCore,
    client: &trailboss::Client,
    session_menu: bool,
) {
    let declarations = client.declarations.as_ref().unwrap();
    if session_menu {
        let surface =
            trailboss::terminal::surface_rect(native, declarations, Rect::viewport(100, 30))
                .unwrap();
        let x = surface.x - 2;
        renderer
            .controls
            .hover(native, declarations, Rect::viewport(100, 30), x, 29);
        assert!(matches!(
            renderer
                .controls
                .click(native, declarations, Rect::viewport(100, 30), x, 29, false),
            Response::Consumed
        ));
        assert!(matches!(
            click_text(renderer, native, client, "Recently closed tabs"),
            Response::Consumed
        ));
    } else {
        assert!(renderer.controls.open_palette(native, declarations, None));
        assert!(renderer.controls.paste("Recently closed tabs"));
        assert!(matches!(
            key(renderer, native, client, KeyCode::Enter),
            Response::Consumed
        ));
    }
}

#[test]
fn native_recently_closed_picker_restores_selected_entry_from_both_entrypoints_and_inputs() {
    for session_menu in [true, false] {
        for mouse in [false, true] {
            let core = support::Core::new();
            let mut client = core.attach();
            let mut native = NativeCore::default();
            let mut entries = Vec::new();
            let beta_directory = core.home.join("beta-project");
            std::fs::create_dir(&beta_directory).unwrap();
            for space in ["Alpha", "Beta"] {
                let cwd = if space == "Beta" {
                    &beta_directory
                } else {
                    &core.home
                };
                let created = core.request(
                    "workspace.create",
                    json!({"label":space,"cwd":cwd,"focus":true}),
                );
                let workspace = created["workspace"]["workspace_id"].clone();
                let tab = created["root_pane"]["tab_id"].clone();
                core.request("tab.rename", json!({"tab_id":tab,"label":"Shared editor"}));
                core.request("tab.create", json!({"workspace_id":workspace,"label":"Survivor","cwd":core.home,"focus":true}));
                core.request("tab.close", json!({"tab_id":tab}));
                entries.push(workspace);
            }
            let before = core.request("tab.closed.list", json!({}));
            let catalog = core.request("ui.command.list", json!({}));
            assert!(
                catalog["commands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|command| command["title"] == "Recently closed tabs"),
                "recently closed picker is not discoverable: {catalog}"
            );
            for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
                core.request("plugin.link", json!({"path":format!("{}/plugins/{plugin}",env!("CARGO_MANIFEST_DIR")),"enabled":true}));
            }
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                if client.declarations.as_ref().is_some_and(|declarations| {
                    native
                        .fragment(declarations, "sidebar.session-menu")
                        .is_some()
                        && native.fragment(declarations, "command-palette").is_some()
                        && declarations.iter().any(|(owner, declaration)| {
                            owner == "tommymorgan.stock-ui" && declaration.composition.is_some()
                        })
                }) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "native entrypoints did not publish"
                );
                client.receive().unwrap();
            }
            native.update_session(
                client.workspace_id.clone(),
                client.focused_tab_id.clone(),
                true,
            );
            let mut input = client.input().unwrap();
            let mut renderer = FrameRenderer::default();
            open_picker(&mut renderer, &native, &client, session_menu);
            let painted = rows(&mut renderer, &native, &client);
            let beta = painted
                .iter()
                .position(|line| line.contains("Shared editor · Beta"))
                .expect("newest entry has no original space");
            let alpha = painted
                .iter()
                .position(|line| line.contains("Shared editor · Alpha"))
                .expect("older entry has no original space");
            assert!(
                beta < alpha,
                "picker reordered closure history: {painted:?}"
            );
            for index in [alpha, beta] {
                assert!(
                    painted[index].contains("UTC") && painted[index].contains(':'),
                    "missing visible close time: {:?}",
                    painted[index]
                );
            }
            key(&mut renderer, &native, &client, KeyCode::Esc);
            renderer.controls.reset();
            assert_eq!(
                core.request("tab.closed.list", json!({})),
                before,
                "dismissal changed history"
            );
            open_picker(&mut renderer, &native, &client, session_menu);
            assert!(renderer.controls.paste("Alpha"),"history is not searchable: session_menu={session_menu}, mouse={mouse}, screen={:?}",rows(&mut renderer,&native,&client));
            let selected = if mouse {
                click_text(&mut renderer, &native, &client, "Shared editor")
            } else {
                key(&mut renderer, &native, &client, KeyCode::Enter)
            };
            let Response::Invoke(selected) = selected else {
                panic!("history selection did not invoke restore");
            };
            input
                .dispatch(
                    &mut native,
                    &client.boot_id,
                    &selected.plugin,
                    &selected.action,
                    &selected.context,
                )
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                if client.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.tabs.iter().any(|tab| {
                        tab.label == "Shared editor"
                            && Some(tab.tab_id.as_str()) == client.focused_tab_id.as_deref()
                            && Some(tab.workspace_id.as_str()) == entries[0].as_str()
                    })
                }) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "selected history entry was not restored and focused"
                );
                client.receive().unwrap_or_else(|error| panic!("restore did not focus: {error}; action={}; history={}; snapshot={:?}; native={native:?}",selected.action,core.request("tab.closed.list",json!({})),client.snapshot));
                if let Some(response) = client.endpoint_response.take() {
                    native.closed_tab_restore_response(&response, &client.boot_id, Instant::now());
                }
            }
            let remaining = core.request("tab.closed.list", json!({}));
            assert_eq!(
                remaining["closed_tabs"],
                json!([before["closed_tabs"][0].clone()]),
                "selection consumed or restored another entry"
            );
            std::fs::remove_dir(&beta_directory).unwrap();
            let tabs_before_failure = core.request("tab.list", json!({}));
            renderer.controls.reset();
            open_picker(&mut renderer, &native, &client, session_menu);
            assert!(renderer.controls.paste("Beta"));
            let Response::Invoke(unavailable) =
                key(&mut renderer, &native, &client, KeyCode::Enter)
            else {
                panic!("unavailable entry cannot be selected");
            };
            input
                .dispatch(
                    &mut native,
                    &client.boot_id,
                    &unavailable.plugin,
                    &unavailable.action,
                    &unavailable.context,
                )
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            while native.endpoint_notice().is_none() {
                assert!(
                    Instant::now() < deadline,
                    "restore failure did not reach native feedback"
                );
                client.receive().unwrap();
                if let Some(response) = client.endpoint_response.take() {
                    native.closed_tab_restore_response(&response, &client.boot_id, Instant::now());
                }
            }
            assert!(native.endpoint_notice().unwrap().contains("beta-project"));
            assert!(
                rows(&mut renderer, &native, &client)
                    .iter()
                    .any(|line| line.contains("Cannot restore closed tab")),
                "restore failure was not painted"
            );
            assert_eq!(core.request("tab.closed.list", json!({})), remaining);
            assert_eq!(core.request("tab.list", json!({})), tabs_before_failure);
            std::fs::create_dir(&beta_directory).unwrap();
            core.request(
                "tab.closed.restore",
                json!({"closed_tab_id":remaining["closed_tabs"][0]["closed_tab_id"]}),
            );
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                renderer.controls.reset();
                open_picker(&mut renderer, &native, &client, session_menu);
                if rows(&mut renderer, &native, &client)
                    .iter()
                    .any(|line| line.contains("No recently closed tabs"))
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "empty history was not shown explicitly"
                );
                client.receive().unwrap();
            }
        }
    }
}
