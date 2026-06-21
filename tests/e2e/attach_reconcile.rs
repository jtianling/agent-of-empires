//! RED probe for the `background-reconcile` change.
//!
//! The reconciler that snapshots `pane_live` captures into the durable
//! `agent_slot` table is, on current `main`, driven ONLY by the TUI status
//! poller -- and the poller only ticks from the home (session list) view. While
//! the user is attached to a session the TUI main loop is blocked on a
//! synchronous `tmux attach-session`, so the poller stops ticking and reconcile
//! is suspended. `agent_slot` therefore stops advancing for the entire time the
//! user stays inside an agent, which is AoE's normal usage.
//!
//! This test reproduces the spec scenario "Reconcile continues while attached to
//! a session": a managed session is attached (a real tmux client views it) and
//! NO AoE home-view TUI is running, so the status-poller reconcile driver is
//! absent -- exactly the "poller is not ticking" precondition of the bug. The
//! only remaining possible reconcile driver is the long-lived notification
//! monitor (`aoe tmux monitor-notifications`).
//!
//! On current `main` the monitor does NOT reconcile, so a capture produced in
//! this state never reaches `agent_slot` and this test is RED (it times out).
//! Plan B adds a throttled `reconcile_all` to the monitor loop, which turns it
//! GREEN.
//!
//! The monitor is bootstrapped exactly as `ensure_notification_monitor` does:
//! spawn the real `aoe` monitor subcommand, then publish its pid into the
//! `@aoe_notification_monitor_pid` server option so the monitor's ownership
//! guard keeps it alive. All tmux + store state lives on the harness's isolated
//! private socket and temp HOME; nothing touches the real profile or the default
//! tmux socket.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serial_test::serial;

use crate::harness::TuiTestHarness;

macro_rules! require_sqlite3 {
    () => {
        if !sqlite3_available() {
            eprintln!("Skipping test: sqlite3 CLI not available");
            return;
        }
    };
}

fn sqlite3_available() -> bool {
    Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn db_path(h: &TuiTestHarness) -> PathBuf {
    let profile_dir = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default")
    } else {
        h.home_path().join(".agent-of-empires/profiles/default")
    };
    profile_dir.join("aoe.db")
}

fn sqlite_query(db: &Path, sql: &str) -> String {
    let output = Command::new("sqlite3")
        .arg("-cmd")
        .arg(".timeout 5000")
        .arg(db)
        .arg(sql)
        .output()
        .expect("failed to run sqlite3");
    assert!(
        output.status.success(),
        "sqlite3 query failed for {:?}: {}\nstdout: {}",
        sql,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Invoke the hidden `aoe __record-pane` capture subcommand exactly as the hook
/// would: pipe hook stdin JSON, set `$TMUX_PANE`/`$AOE_INSTANCE_ID`.
fn run_record_pane(
    h: &TuiTestHarness,
    tmux_pane: &str,
    instance_id: &str,
    session_id: &str,
) -> bool {
    let stdin_json = format!(
        "{{\"session_id\":\"{session_id}\",\"cwd\":\"/work\",\"hook_event_name\":\"SessionStart\"}}"
    );
    let mut child = Command::new(h.binary_path())
        .arg("__record-pane")
        .env("HOME", h.home_path())
        .env("XDG_CONFIG_HOME", h.home_path().join(".config"))
        .env("AGENT_OF_EMPIRES_PROFILE", "default")
        .env("TMUX_PANE", tmux_pane)
        .env("AOE_INSTANCE_ID", instance_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn aoe __record-pane");
    child
        .stdin
        .as_mut()
        .expect("record-pane stdin")
        .write_all(stdin_json.as_bytes())
        .expect("write record-pane stdin");
    child
        .wait_with_output()
        .expect("wait for aoe __record-pane")
        .status
        .success()
}

/// Register a session and start its tmux process. Returns the instance id.
fn add_and_start(h: &TuiTestHarness, title: &str) -> String {
    let project = h.project_path();
    let add = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        title,
        "--cmd-override",
        "sh",
    ]);
    assert!(
        add.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let start = h.run_cli_in_tmux(&["session", "start", title]);
    assert!(
        start.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let sessions_path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    };
    let content = std::fs::read_to_string(&sessions_path).expect("read sessions.json");
    let sessions: serde_json::Value = serde_json::from_str(&content).unwrap();
    sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str() == Some(title))
        .and_then(|s| s["id"].as_str())
        .unwrap_or_else(|| panic!("missing session {}", title))
        .to_string()
}

/// Attach a real tmux client to a session on the harness's private socket.
///
/// Mirrors `TuiTestHarness::attach_client_to_session` but without its
/// `spawned()` guard: this test intentionally does NOT run a home-view TUI (that
/// would restart the status poller and mask the bug under test), so the harness
/// method's precondition does not hold here. The client just makes "a session is
/// attached" literally true; it is inert with respect to reconcile.
fn attach_client(h: &TuiTestHarness, target: &str) -> Child {
    let socket = h.tmux_socket_path();
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("script");
        c.arg("-q")
            .arg("/dev/null")
            .arg("tmux")
            .arg("-S")
            .arg(socket)
            .arg("attach-session")
            .arg("-t")
            .arg(target);
        c
    } else {
        let mut c = Command::new("script");
        c.arg("-q")
            .arg("-c")
            .arg(format!(
                "tmux -S '{}' attach-session -t '{}'",
                socket.display(),
                target
            ))
            .arg("/dev/null");
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to attach tmux client")
}

/// Start the long-lived notification monitor against the harness's isolated
/// private socket, and publish its pid into the ownership-guard server option so
/// the monitor stays alive (mirrors `ensure_notification_monitor`).
fn spawn_monitor(h: &TuiTestHarness) -> Child {
    let tmux_env = format!("{},1,0", h.tmux_socket_path().display());
    let child = Command::new(h.binary_path())
        .args(["tmux", "monitor-notifications", "--profile", "default"])
        .env("HOME", h.home_path())
        .env("XDG_CONFIG_HOME", h.home_path().join(".config"))
        .env("AGENT_OF_EMPIRES_PROFILE", "default")
        .env("TMUX", &tmux_env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn notification monitor");

    // The monitor's loop exits unless `@aoe_notification_monitor_pid` equals its
    // own pid within a 2s startup deadline. Publish it immediately on the same
    // private socket the monitor queries via $TMUX.
    let _ = Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args([
            "set-option",
            "-gq",
            "@aoe_notification_monitor_pid",
            &child.id().to_string(),
        ])
        .output();

    child
}

/// Poll a count query until it equals `expected` or the timeout elapses. Returns
/// `(reached, last_seen)` instead of panicking so the caller can tear down its
/// spawned processes before asserting.
fn poll_count_eq(db: &Path, sql: &str, expected: &str, timeout: Duration) -> (bool, String) {
    let start = Instant::now();
    loop {
        let got = sqlite_query(db, sql);
        if got == expected {
            return (true, got);
        }
        if start.elapsed() > timeout {
            return (false, got);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Kills the spawned monitor + attach client and the isolated private tmux
/// server on drop, so the test cleans up even if an assertion panics. (This test
/// deliberately does not run `spawn_tui`, so the harness Drop -- which only
/// sweeps the default socket -- would otherwise leave the private server and its
/// managed session running.)
struct Cleanup<'a> {
    h: &'a TuiTestHarness,
    monitor: Option<Child>,
    attach: Option<Child>,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        if let Some(mut c) = self.monitor.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(mut c) = self.attach.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = Command::new("tmux")
            .arg("-S")
            .arg(self.h.tmux_socket_path())
            .arg("kill-server")
            .output();
    }
}

// ---------------------------------------------------------------------------
// Requirement: Reconcile continues while attached (the status poller is idle)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn reconcile_advances_agent_slot_from_monitor_while_poller_idle() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("attach_reconcile");
    let mut cleanup = Cleanup {
        h: &h,
        monitor: None,
        attach: None,
    };

    let instance_id = add_and_start(&h, "Attach Reconcile");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Attach Reconcile");
    let pane_id = h.tmux_display_message(&session_name, "#{pane_id}");

    // Attach a real tmux client to the managed session. No AoE home-view TUI is
    // running, so the status-poller reconcile driver is absent: this is the
    // "poller is not ticking" state the bug occurs in.
    cleanup.attach = Some(attach_client(&h, &session_name));

    // A capture lands in pane_live while no reconcile driver is active yet.
    assert!(
        run_record_pane(&h, &pane_id, &instance_id, "attach-sess"),
        "capture for the attached pane should succeed"
    );

    // Precondition: nothing has snapshotted the capture into agent_slot yet.
    let before = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        before, "0",
        "precondition: no agent_slot row before reconcile"
    );

    // Bring up the notification monitor -- the only possible reconcile driver in
    // this state. Under Plan B it reconciles on a throttled interval.
    cleanup.monitor = Some(spawn_monitor(&h));

    // The capture must reach agent_slot within a bounded time WITHOUT returning
    // to the home view. RED on current main (the monitor does not reconcile);
    // GREEN once Plan B adds reconcile to the monitor loop.
    let (reached, last) = poll_count_eq(
        &db,
        &format!(
            "SELECT count(*) FROM agent_slot \
             WHERE instance_id='{instance_id}' AND native_session_id='attach-sess';"
        ),
        "1",
        Duration::from_secs(12),
    );

    assert!(
        reached,
        "agent_slot was not advanced by the notification monitor while the status \
         poller was idle (last count={last}). This is the background-reconcile \
         gating bug: reconcile is suspended whenever the poller is not ticking \
         (e.g. while attached to a session)."
    );
}
