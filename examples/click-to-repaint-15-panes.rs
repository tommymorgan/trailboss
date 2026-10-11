#[path = "../tests/support/console.rs"]
mod console;
#[path = "../tests/support/latency.rs"]
mod latency;
#[path = "../tests/support/mod.rs"]
mod support;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--observe") {
        latency::observer_file(std::path::Path::new(&args[2]));
    } else {
        assert!(
            args.len() == 3 && ["local", "remote"].contains(&args[1].as_str()),
            "usage: click-to-repaint-15-panes local|remote OUTPUT_JSON"
        );
        latency::run(
            args[1] == "remote",
            &std::env::current_exe().unwrap(),
            std::path::Path::new(&args[2]),
        );
    }
}
