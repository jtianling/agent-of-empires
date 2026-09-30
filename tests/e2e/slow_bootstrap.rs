//! A launch wrapper that bootstraps its agent slowly (the Codex xats wrapper
//! loads a login shell and pre-registers before starting codex) shows a shell
//! in the pane for a while. That is not an agent that exited: only a pane the
//! pane-died hook respawned into a shell is.

use std::time::{Duration, Instant};

use serial_test::serial;

use crate::harness::TuiTestHarness;

/// Spins in the shell itself (so tmux reports the pane as `sh`) for longer
/// than the start grace period, then becomes a non-shell process.
const SLOW_BOOTSTRAP_STUB: &str = "#!/bin/sh\n\
end=$(( $(date +%s) + 10 ))\n\
while [ \"$(date +%s)\" -lt \"$end\" ]; do :; done\n\
exec sleep 100000\n";

#[test]
#[serial]
fn a_slow_bootstrap_is_not_reported_as_a_fallen_agent() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("slow_bootstrap");
    h.install_stub_script("codex", SLOW_BOOTSTRAP_STUB);
    let title = "Slow Bootstrap";
    let project = h.project_path();
    let add = h.run_cli(&["add", project.to_str().unwrap(), "-t", title, "-c", "codex"]);
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for(title);
    h.send_keys("c");

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let screen = h.capture_screen();
        assert!(
            !screen.contains("dropped to shell") && !screen.contains("Status:  Error"),
            "a bootstrapping wrapper was reported as a fallen agent\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
}
