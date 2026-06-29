//! E2E tests for the `cold-start-session-recovery` capability (w04).
//!
//! After a reboot every tmux session is gone but an instance's `agent_slot`
//! rows survive in the store. AoE classifies such an instance as *recoverable*,
//! marks it in the home list, and lets the user rebuild + resume it by pressing
//! `V` on the focused row. Recovery recreates the tmux session, recreates one
//! pane per persisted slot (slot 0 as the primary `@aoe_agent_pane`, the rest
//! split off), resume-launches each pane from its `agent_slot.native_session_id`
//! via the same per-pane resume core the `R` flow uses, and writes the new pane
//! ids back into `agent_slot.tmux_pane`.
//!
//! ## How recovery is observed from outside the process
//!
//! `resume_launch_pane` ends in `tmux respawn-pane -k -c <cwd> -t <pane>
//! <command>` (see `src/tmux/session.rs`). tmux records that command string in
//! `#{pane_start_command}` for the pane, which survives even after the (stubbed)
//! agent binary exits. Each test therefore asserts on
//! `display-message -p '#{pane_start_command}'` per recovered pane id -- the
//! external, durable signal that the pane was resume-launched with
//! `--resume <id>` (or, on degrade, with a bare `claude` and no resume flag).
//!
//! Slots are seeded through the real capture+reconcile path exactly as the `R`
//! tests do: `aoe __record-pane` writes `pane_live` rows and the home-view
//! status poller reconciles them into `agent_slot`. The capture JSON carries a
//! real on-disk `cwd` so the recovery `split-window`/`respawn-pane` `-c <cwd>`
//! invocations succeed deterministically. Cold start is then simulated by
//! killing the managed tmux session while the home-view TUI stays up; the poller
//! flips the instance to `[recoverable]` and `V` triggers the rebuild.
//!
//! Everything lives on the harness's isolated private tmux socket and temp HOME;
//! the agent binaries are never really run (only the command strings matter), so
//! no real `~/.claude` is touched.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
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
/// would: pipe hook stdin JSON, set `$TMUX_PANE`/`$AOE_INSTANCE_ID`. `cwd` is a
/// real on-disk directory so the later recovery split/respawn `-c <cwd>` works.
fn run_record_pane(
    h: &TuiTestHarness,
    tmux_pane: &str,
    aoe_instance_id: &str,
    session_id: &str,
    cwd: &str,
) -> bool {
    let stdin_json = format!(
        "{{\"session_id\":\"{session_id}\",\"cwd\":\"{cwd}\",\"hook_event_name\":\"SessionStart\"}}"
    );
    let mut child = Command::new(h.binary_path())
        .arg("__record-pane")
        .env("HOME", h.home_path())
        .env("XDG_CONFIG_HOME", h.home_path().join(".config"))
        .env("AGENT_OF_EMPIRES_PROFILE", "default")
        .env("TMUX_PANE", tmux_pane)
        .env("AOE_INSTANCE_ID", aoe_instance_id)
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

/// Add + start an instance whose primary agent is `tool`. The instance tool must
/// match slot 0's recorded agent: recovery rebuilds the primary pane's resume
/// command from `self.tool` (`get_tool_command()`), so a `--cmd-override` that
/// swaps in a shell would suppress the `--resume <id>` these tests assert on. A
/// long-lived stub for `tool` keeps the started primary pane alive to be tracked.
fn add_and_start(h: &TuiTestHarness, title: &str, tool: &str) -> String {
    h.install_tool_stub(tool);
    let project = h.project_path();
    let add = h.run_cli(&["add", project.to_str().unwrap(), "-t", title, "-c", tool]);
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

fn wait_for_count(h: &TuiTestHarness, db: &Path, sql: &str, expected: &str) {
    let start = Instant::now();
    loop {
        let got = sqlite_query(db, sql);
        if got == expected {
            return;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!(
                "Timed out waiting for `{}` to equal {} (last={}).\n\n--- Screen ---\n{}",
                sql,
                expected,
                got,
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Run a tmux subcommand against the harness's private socket.
fn tmux(h: &TuiTestHarness, args: &[&str]) -> Output {
    Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(args)
        .output()
        .expect("failed to run tmux")
}

fn session_exists(h: &TuiTestHarness, session: &str) -> bool {
    tmux(h, &["has-session", "-t", session]).status.success()
}

/// Pane ids of a session in pane-index order; empty if the session is gone.
fn session_pane_ids(h: &TuiTestHarness, session: &str) -> Vec<String> {
    let out = tmux(h, &["list-panes", "-t", session, "-F", "#{pane_id}"]);
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// Persisted `agent_slot.tmux_pane` values for an instance, in ascending slot
/// order.
fn slot_panes(db: &Path, instance_id: &str) -> Vec<String> {
    let out = sqlite_query(
        db,
        &format!(
            "SELECT tmux_pane FROM agent_slot WHERE instance_id='{instance_id}' ORDER BY slot;"
        ),
    );
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// Persisted `agent_slot.native_session_id` values for an instance, in ascending
/// slot order (aligned element-for-element with [`slot_panes`]).
fn slot_natives(db: &Path, instance_id: &str) -> Vec<String> {
    let out = sqlite_query(
        db,
        &format!(
            "SELECT native_session_id FROM agent_slot WHERE instance_id='{instance_id}' ORDER BY slot;"
        ),
    );
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn pane_start_command(h: &TuiTestHarness, pane_id: &str) -> String {
    h.tmux_display_message(pane_id, "#{pane_start_command}")
}

/// Poll a pane's start command until it contains `needle`, or panic with the
/// last seen value and a screen dump. Recovery is synchronous in the V handler
/// but the respawn command may take a tick to surface in tmux.
fn wait_for_pane_start_command_contains(h: &TuiTestHarness, pane_id: &str, needle: &str) {
    let start = Instant::now();
    loop {
        let last = pane_start_command(h, pane_id);
        if last.contains(needle) {
            return;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!(
                "Timed out waiting for pane {} start command to contain {:?} (last={:?}).\
                 \n\n--- Screen ---\n{}",
                pane_id,
                needle,
                last,
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Poll until slot 0's persisted `tmux_pane` differs from `old`, signalling that
/// recovery has rebuilt the panes and written the new ids back. Returns the new
/// value (or the last seen one on timeout, so the caller can report it).
fn wait_for_slot0_rebound(db: &Path, instance_id: &str, old: &str) -> String {
    let sql =
        format!("SELECT tmux_pane FROM agent_slot WHERE instance_id='{instance_id}' AND slot=0;");
    let start = Instant::now();
    loop {
        let got = sqlite_query(db, &sql);
        if got != old && !got.is_empty() {
            return got;
        }
        if start.elapsed() > Duration::from_secs(20) {
            return got;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Seed a started instance with `slots.len()` tracked agent panes, each captured
/// and reconciled into `agent_slot`, then return `(instance_id, session_name,
/// project_cwd, old_pane_ids)`. The home-view TUI is left running (sized large so
/// the later recovery splits fit) and the managed session is alive on return.
fn seed_recoverable(
    h: &mut TuiTestHarness,
    title: &str,
    slots: &[&str],
) -> (String, String, String, Vec<String>) {
    let instance_id = add_and_start(h, title, "claude");
    let db = db_path(h);
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let project = h.project_path().to_str().unwrap().to_string();

    // Room for the pre-kill splits that establish the tracked panes.
    h.resize_window(&session_name, 220, 60);

    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    run_record_pane(h, &primary, &instance_id, slots[0], &project);
    for sess in &slots[1..] {
        let pane = h.split_window_get_pane(&session_name);
        run_record_pane(h, &pane, &instance_id, sess, &project);
    }

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    // The recovered session inherits the home TUI's terminal size, so make it
    // large enough for up to four `-h` splits.
    h.resize_window(h.session_name(), 220, 60);

    wait_for_count(
        h,
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
        &slots.len().to_string(),
    );

    let old_panes = slot_panes(&db, &instance_id);
    assert_eq!(
        old_panes.len(),
        slots.len(),
        "precondition: one persisted slot per seeded pane"
    );
    (instance_id, session_name, project, old_panes)
}

/// Kill the managed session (simulating a reboot) and wait for the home view to
/// classify the instance as recoverable.
fn cold_start(h: &TuiTestHarness, session_name: &str) {
    h.kill_tmux_target(session_name);
    assert!(
        !session_exists(h, session_name),
        "managed session must be dead after kill (cold-start precondition)"
    );
    h.wait_for("[recoverable]");
}

// ---------------------------------------------------------------------------
// Requirement: Session rebuild from persisted slots
// Requirement: Pane id write-back after recovery
//   6.2: N persisted slots -> session recreated with N panes, each pane resumes
//        from its native_session_id, each slot's tmux_pane updated to the new id.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn recover_rebuilds_session_with_n_panes_resumed_and_writes_back() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_recover_n");
    let slots = [
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa0",
        "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1",
        "cccccccc-cccc-4ccc-8ccc-ccccccccccc2",
    ];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Cold Start Recover", &slots);
    let db = db_path(&h);

    cold_start(&h, &session_name);

    // The status bar advertises the recovery key while the recoverable instance
    // is focused.
    h.assert_screen_contains("Recover");

    h.send_keys("V");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);

    eprintln!("DEBUG old_panes={:?}", old_panes);
    eprintln!(
        "DEBUG agent_slot rows:\n{}",
        sqlite_query(
            &db,
            &format!(
                "SELECT slot, tmux_pane, native_session_id, agent FROM agent_slot WHERE instance_id='{instance_id}' ORDER BY slot;"
            ),
        )
    );
    eprintln!(
        "DEBUG live panes (idx id cmd):\n{}",
        String::from_utf8_lossy(
            &tmux(
                &h,
                &[
                    "list-panes",
                    "-t",
                    &session_name,
                    "-F",
                    "#{pane_index} #{pane_id} [#{pane_start_command}]",
                ],
            )
            .stdout
        )
    );
    eprintln!(
        "DEBUG instance error note / screen:\n{}",
        h.capture_screen()
    );

    assert_ne!(
        new_slot0, old_panes[0],
        "slot 0 tmux_pane must be rewritten to the rebuilt pane id (recovery did not run?)"
    );

    // Session was recreated with exactly N panes.
    assert!(
        session_exists(&h, &session_name),
        "tmux session must be recreated by recovery"
    );
    let live_panes = session_pane_ids(&h, &session_name);
    assert_eq!(
        live_panes.len(),
        slots.len(),
        "recovered session must have one pane per slot, got {:?}",
        live_panes
    );

    let new_panes = slot_panes(&db, &instance_id);
    assert_eq!(new_panes.len(), slots.len());

    // The seed's live reconcile assigns slot numbers by ascending pane index,
    // which need not match the order this test recorded the sessions in (for 3+
    // panes a right-split lands at a lower index than an earlier one). So assert
    // against each slot's OWN persisted native_session_id, read back by slot and
    // aligned with `new_panes`, rather than the positional `slots[i]`. Recovery
    // is correct as long as every slot's rebuilt pane resumes that slot's own
    // session and all seeded sessions survive exactly once.
    let new_natives = slot_natives(&db, &instance_id);
    assert_eq!(new_natives.len(), slots.len());
    let mut got_natives: Vec<&str> = new_natives.iter().map(String::as_str).collect();
    let mut want_natives: Vec<&str> = slots.to_vec();
    got_natives.sort_unstable();
    want_natives.sort_unstable();
    assert_eq!(
        got_natives, want_natives,
        "every seeded session must survive recovery exactly once"
    );

    for (i, native_id) in new_natives.iter().enumerate() {
        // Write-back: each slot points at a brand-new pane id.
        assert_ne!(
            new_panes[i], old_panes[i],
            "slot {i} tmux_pane must be updated to the new pane id"
        );
        assert!(
            live_panes.contains(&new_panes[i]),
            "slot {i} new pane {} must be a live pane in the rebuilt session {:?}",
            new_panes[i],
            live_panes
        );
        // Each recovered pane resumes from ITS slot's own native_session_id.
        wait_for_pane_start_command_contains(&h, &new_panes[i], &format!("--resume {native_id}"));
        let cmd = pane_start_command(&h, &new_panes[i]);
        assert!(
            cmd.contains("claude"),
            "slot {i} resume command should launch claude, got {:?}",
            cmd
        );
    }
}

// ---------------------------------------------------------------------------
// Requirement: Per-pane degrade and isolation on recovery
//   6.3a: a slot with an empty native_session_id degrades to a fresh launch for
//         that pane while the sibling still resumes.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn recover_degrades_empty_native_id_to_fresh_while_sibling_resumes() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_recover_degrade");
    let slots = [
        "11111111-1111-4111-8111-111111111110", // slot 0: keeps a valid id
        "22222222-2222-4222-8222-222222222221", // slot 1: will be cleared
    ];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Cold Start Degrade", &slots);
    let db = db_path(&h);

    cold_start(&h, &session_name);

    // Model a slot whose native_session_id is unusable. Safe to mutate now: the
    // session is dead, so reconcile skips this instance and cannot overwrite it.
    sqlite_query(
        &db,
        &format!("UPDATE agent_slot SET native_session_id='' WHERE instance_id='{instance_id}' AND slot=1;"),
    );

    h.send_keys("V");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    let live_panes = session_pane_ids(&h, &session_name);
    assert_eq!(
        live_panes.len(),
        2,
        "both panes must be rebuilt, got {:?}",
        live_panes
    );
    let new_panes = slot_panes(&db, &instance_id);

    // Slot 0 resumes from its valid id.
    wait_for_pane_start_command_contains(&h, &new_panes[0], &format!("--resume {}", slots[0]));

    // Slot 1 degraded to a fresh claude launch -- no resume flag.
    wait_for_pane_start_command_contains(&h, &new_panes[1], "claude");
    let degraded = pane_start_command(&h, &new_panes[1]);
    assert!(
        !degraded.contains("--resume"),
        "slot 1 with an empty native_session_id must launch fresh (no --resume), got {:?}",
        degraded
    );

    // Both slots still got their new pane ids written back.
    assert_ne!(new_panes[0], old_panes[0]);
    assert_ne!(new_panes[1], old_panes[1]);
}

// ---------------------------------------------------------------------------
// Requirement: Per-pane degrade and isolation on recovery
//   6.3b: one pane's resume failing (here forced via an unsafe agent token that
//         build_pane_resume_command refuses) does not abort recovery of the
//         remaining panes; the sibling still resumes and both slots are written
//         back.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn recover_one_pane_failure_does_not_abort_sibling() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_recover_isolation");
    let slots = [
        "33333333-3333-4333-8333-333333333330", // slot 0: resumes normally
        "44444444-4444-4444-8444-444444444441", // slot 1: forced to error
    ];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Cold Start Isolation", &slots);
    let db = db_path(&h);

    cold_start(&h, &session_name);

    // An agent token with a space is rejected by is_safe_command_token, so
    // build_pane_resume_command returns None and resume_launch_pane yields an
    // Error outcome for slot 1 -- a genuine per-pane failure, not a degrade.
    sqlite_query(
        &db,
        &format!(
            "UPDATE agent_slot SET agent='bad agent' WHERE instance_id='{instance_id}' AND slot=1;"
        ),
    );

    h.send_keys("V");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    // The failure did not abort the rebuild: both panes exist...
    let live_panes = session_pane_ids(&h, &session_name);
    assert_eq!(
        live_panes.len(),
        2,
        "a per-pane failure must not abort sibling pane creation, got {:?}",
        live_panes
    );

    // ...the healthy sibling (slot 0) still resumed from its own id...
    let new_panes = slot_panes(&db, &instance_id);
    wait_for_pane_start_command_contains(&h, &new_panes[0], &format!("--resume {}", slots[0]));

    // ...and write-back still happened for BOTH slots, proving the loop ran past
    // the failing slot rather than aborting.
    assert_ne!(new_panes[0], old_panes[0], "slot 0 must be written back");
    assert_ne!(new_panes[1], old_panes[1], "slot 1 must be written back");
}
