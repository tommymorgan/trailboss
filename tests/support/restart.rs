use super::support::Core;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

impl Core {
    pub fn restart(&mut self) -> Vec<u8> {
        let old_pid = self.child.id();
        self.request("server.stop", serde_json::json!({}));
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "Core stop failed: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "normal Core stop hung");
            std::thread::sleep(Duration::from_millis(10));
        }
        let saved = std::fs::read(
            self.home
                .join(format!("trailboss/sessions/{}/session.json", self.name)),
        )
        .expect("Core stop did not retain its actual persistence");
        let replacement = Core::launch(self.home.clone(), self.name.clone());
        assert_ne!(replacement.child.id(), old_pid);
        // The exited owner must not remove the root now owned by its replacement.
        self.home = PathBuf::new();
        *self = replacement;
        saved
    }
}
