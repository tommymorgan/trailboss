#[path = "support/console.rs"]
mod console;
#[path = "support/latency.rs"]
mod latency;
mod support;
#[test]
#[should_panic(expected = "population readiness failed for Pane002")]
fn empty_previously_unselected_third_pane_fails_before_measurement() {
    let core = support::Core::new();
    latency::seed_population(&core, Some("Pane002"));
}
fn acceptance(remote: bool) {
    let lines: Vec<_> = include_str!("../features/latency.feature")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(lines, [
        "Feature: Click-to-repaint latency",
        "Scenario Outline: Meet the local and remote click-to-repaint gate",
        "Given 15 populated panes are running in <mode> mode on the recorded acceptance setup",
        "When click-to-repaint-15-panes measures repeated workspace, tab and pane focus clicks through the completed visible repaint",
        "Then every measured click-to-repaint sample is at most 100ms including producer and transport delays and the displayed Surface and focus state match the requested selection",
        "Examples:", "| mode |", "| local |", "| remote from achy |",
    ]);
    let example = std::path::Path::new(env!("CARGO_BIN_EXE_trailboss"))
        .parent()
        .unwrap()
        .join("examples/click-to-repaint-15-panes");
    let output = std::env::temp_dir().join(format!(
        "trailboss-latency-{}-{}.json",
        std::process::id(),
        if remote { "remote" } else { "local" }
    ));
    latency::run(remote, &example, &output);
}
#[test]
#[ignore = "recorded 15-live-PTY acceptance; opt-in isolated benchmark"]
fn living_meet_local_click_to_repaint_gate() {
    acceptance(false);
}
#[test]
#[ignore = "requires real achy UI to junkman SSH; opt-in isolated benchmark"]
fn living_meet_remote_from_achy_click_to_repaint_gate() {
    acceptance(true);
}

fn completed_selection(client: &trailboss::Client, target: &serde_json::Value) -> bool {
    let Some(declarations) = &client.declarations else {
        return false;
    };
    if client.focused_pane_id.as_deref() != target["pane"].as_str()
        || client.focused_tab_id.as_deref() != target["tab"].as_str()
    {
        return false;
    }
    let mut state = trailboss::core::Core::default();
    state.update_session(
        client.workspace_id.clone(),
        client.focused_tab_id.clone(),
        true,
    );
    let mut bytes = Vec::new();
    trailboss::terminal::FrameRenderer::default()
        .draw_frame(
            &mut bytes,
            client.primary.as_ref(),
            declarations,
            &state,
            trailboss::terminal::Rect::viewport(80, 24),
        )
        .unwrap();
    let mut screen = ghostty_vt::Terminal::new(80, 24, 0).unwrap();
    screen.write(&bytes);
    let rows: Vec<String> = screen
        .screen_text_rows()
        .unwrap()
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| {
                    cell.graphemes
                        .first()
                        .and_then(|cp| char::from_u32(*cp))
                        .unwrap_or(' ')
                })
                .collect()
        })
        .collect();
    let Some(rect) = trailboss::terminal::surface_rect(
        &state,
        declarations,
        trailboss::terminal::Rect::viewport(80, 24),
    ) else {
        return false;
    };
    (0..usize::from(rect.height)).all(|y| {
        let expected = if y < 4 {
            format!("{}-row{}", target["label"].as_str().unwrap(), y)
        } else {
            String::new()
        };
        rows[y + usize::from(rect.y)]
            .chars()
            .skip(usize::from(rect.x))
            .take(usize::from(rect.width))
            .collect::<String>()
            .trim_end()
            == expected
    })
}

#[test]
fn producer_dispatch_to_completed_selected_surface_meets_latency_gate() {
    use std::time::{Duration, Instant};
    let core = support::Core::new();
    let populations = latency::seed(&core);
    let mut client = core.attach();
    client.input().unwrap().resize(80, 24).unwrap();
    let startup = Instant::now();
    while !completed_selection(&client, &populations[0])
        || !client.declarations.as_ref().is_some_and(|trees| {
            trees
                .iter()
                .any(|(plugin, _)| plugin == "tommymorgan.agent-tree")
        })
    {
        client.receive().unwrap_or_else(|error| {
            panic!(
                "startup: {error}; pane={:?}; primary={:?}",
                client.focused_pane_id,
                client.primary.as_ref().map(|surface| surface.text())
            )
        });
        assert!(startup.elapsed() < Duration::from_secs(8));
    }
    for (action, index) in [("open-workspace", 5), ("focus-tab", 8), ("focus-pane", 9)] {
        let target = &populations[index];
        let declaration_started = Instant::now();
        let (plugin, action_id, context) = loop {
            let declarations = client.declarations.as_ref().unwrap();
            if let Some(selection) =
                trailboss::sidebar::rows(declarations)
                    .enumerate()
                    .find_map(|(row, _)| {
                        let (plugin, declared, context) =
                            trailboss::sidebar::action_at(declarations, row)?;
                        (declared == action
                            && match action {
                                "open-workspace" => {
                                    context.workspace_id.as_deref() == target["workspace"].as_str()
                                }
                                "focus-tab" => context.tab_id.as_deref() == target["tab"].as_str(),
                                _ => context.pane_id.as_deref() == target["pane"].as_str(),
                            })
                        .then_some((
                            plugin.to_owned(),
                            declared.to_owned(),
                            context.clone(),
                        ))
                    })
            {
                break selection;
            }
            assert!(
                declaration_started.elapsed() < Duration::from_secs(8),
                "actual producer declaration missing selection action"
            );
            client.receive().unwrap();
        };
        let started = Instant::now();
        client
            .input()
            .unwrap()
            .dispatch(
                &mut trailboss::core::Core::default(),
                &client.boot_id,
                &plugin,
                &action_id,
                &context,
            )
            .unwrap();
        while !completed_selection(&client, target) {
            client.receive().unwrap();
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "selected Surface was never completely rendered"
            );
        }
        assert!(
            started.elapsed() <= Duration::from_millis(100),
            "{action} input dispatch to complete decoded selected Surface exceeded 100ms: {:?}",
            started.elapsed()
        );
    }
}
