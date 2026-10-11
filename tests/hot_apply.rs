#[path = "support/replacement.rs"]
mod replacement;
mod support;
use std::time::{Duration, Instant};
use support::Core;

fn render(client: &trailboss::Client, screen: &mut ghostty_vt::Terminal) -> Vec<String> {
    let mut bytes = Vec::new();
    trailboss::terminal::FrameRenderer::default()
        .draw_frame(
            &mut bytes,
            None,
            client.declarations.as_ref().unwrap(),
            &trailboss::core::Core::default(),
            trailboss::terminal::Rect::viewport(80, 24),
        )
        .unwrap();
    screen.write(&bytes);
    screen
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
        .collect()
}
fn wait(
    client: &mut trailboss::Client,
    screen: &mut ghostty_vt::Terminal,
    marker: &str,
) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Err(error) = client.receive() {
            let rows = client.declarations.as_ref().map(|_| render(client, screen));
            panic!("replacement {marker} not visible before receive deadline ({error}); last rendered rows: {rows:?}");
        }
        if client.declarations.is_some() {
            let rows = render(client, screen);
            // Neutral declarations can precede the initial ClientShellSnapshot.
            // Observe its boot identity before recording the restart baseline.
            if !client.boot_id.is_empty() && rows.iter().any(|row| row.starts_with(marker)) {
                return rows;
            }
            assert!(
                Instant::now() < deadline,
                "replacement {marker} not visible; last rendered rows: {rows:?}"
            );
        }
        assert!(
            Instant::now() < deadline,
            "no declaration visible before deadline"
        );
    }
}
#[test]
fn running_producer_replaces_whole_tree_on_the_same_client_socket() {
    let core = Core::new();
    let plugin = replacement::enable(&core);
    let mut client = core.attach();
    let mut screen = ghostty_vt::Terminal::new(80, 24, 0).unwrap();
    let rows = wait(&mut client, &mut screen, "OLDTREE");
    assert_eq!(rows[0].trim_end(), "OLDTREE");
    assert!(rows[1..].iter().all(|row| row.trim().is_empty()));
    let boot = client.boot_id.clone();

    replacement::replace(&plugin);
    let rows = wait(&mut client, &mut screen, "NEW");
    replacement::assert_replacement(&rows);
    replacement::assert_same_producer_and_socket(&core, &plugin);
    assert_eq!(client.boot_id, boot, "Core restarted");
}
