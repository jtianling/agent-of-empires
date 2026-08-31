//! E2E test for the `status-detection` shell-fallback requirement: a tracked
//! agent pane whose process dies falls back to a plain shell through the
//! pane-died hook, and the TUI must report the instance as an error naming
//! the fallen pane instead of showing a healthy session -- the silent state
//! the 2026-08-31 codex-update incident produced. A restart then clears the
//! error through the existing restart error reset.
//!
//! The test drives the real `aoe` binary through the harness (private tmux
//! socket, isolated `$HOME`), seeds a codex-recorded slot through the real
//! capture + reconcile chain, kills the stubbed agent so the pane-died hook
//! fires, and observes the rendered TUI screen.

use std::time::Duration;

use serial_test::serial;

use crate::claude_model_support::{require_sqlite3, seed_instance, SlotSeed};
use crate::harness::TuiTestHarness;

#[test]
#[serial]
fn fallen_agent_pane_surfaces_as_error_and_restart_clears_it() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("fallen_agent");
    // A codex instance whose primary pane runs the long-lived stub and is
    // tracked as a codex `agent_slot` row.
    let fixture = seed_instance(
        &mut h,
        "Fallen Agent",
        "codex",
        None,
        &[SlotSeed {
            agent: "codex",
            native: "019d1af9-a899-7df1-8f7d-a244126e5ded",
        }],
    );

    // Kill the stubbed agent. The pane-died hook replaces the dead pane with
    // a plain shell, so the slot still records codex while the pane runs a
    // shell -- the fallen state under test.
    h.send_keys_to_target(&fixture.panes[0], "C-c");

    // The status poller reports the instance as an error naming the fallen
    // pane and the restart keys; the preview renders `last_error`.
    h.wait_for_timeout("dropped to shell", Duration::from_secs(20));
    h.assert_screen_contains(&fixture.panes[0]);
    h.assert_screen_contains("r/R");

    // `c` (fresh restart, stay on home) relaunches the stub agent; the
    // restart path's existing error reset clears the fallen-agent error.
    h.send_keys("c");
    h.wait_for_absent("dropped to shell", Duration::from_secs(20));
}
