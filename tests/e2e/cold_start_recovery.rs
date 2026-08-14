//! E2E tests for the `cold-start-session-recovery` capability (w04).
//!
//! After a reboot every tmux session is gone but an instance's `agent_slot`
//! rows survive in the store. AoE classifies such an instance as *recoverable*,
//! marks it in the home list, and lets the user rebuild + resume it by pressing
//! `R` on the focused row. Recovery recreates the tmux session, recreates one
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
//! flips the instance to `[recoverable]` and `R` triggers the rebuild.
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
    run_record_pane_as(h, tmux_pane, aoe_instance_id, "claude", session_id, cwd)
}

/// [`run_record_pane`] for a pane running `agent`, the way the hook reports a
/// non-Claude agent. A slot's recorded agent is what recovery relaunches it as,
/// so it is the interesting variable for an instance whose own tool differs.
fn run_record_pane_as(
    h: &TuiTestHarness,
    tmux_pane: &str,
    aoe_instance_id: &str,
    agent: &str,
    session_id: &str,
    cwd: &str,
) -> bool {
    let stdin_json = format!(
        "{{\"session_id\":\"{session_id}\",\"cwd\":\"{cwd}\",\"hook_event_name\":\"SessionStart\"}}"
    );
    let mut cmd = Command::new(h.binary_path());
    cmd.arg("__record-pane").arg("--agent").arg(agent);
    // This simulates a hook from outside the pane, so keep the capture's
    // pane-ownership check unanswerable rather than answerably wrong: point it
    // at a serverless socket dir (which MUST exist -- tmux silently falls back
    // to the real default socket when $TMUX_TMPDIR does not), and drop any
    // real $TMUX so it cannot reach the developer's own server.
    let no_server = h.home_path().join("no-tmux-server");
    std::fs::create_dir_all(&no_server).expect("create serverless tmpdir");
    let mut child = cmd
        .env_remove("TMUX")
        .env("TMUX_TMPDIR", &no_server)
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

/// Add + start an instance whose own tool is `tool`, with a long-lived stub for
/// `tool` so the started primary pane stays alive to be tracked.
///
/// The instance tool no longer has to match slot 0's recorded agent: recovery
/// builds each pane's command from the agent that pane's slot recorded, so
/// `add_and_start(h, title, "shell")` plus [`SlotSeed`]s recording `claude` gives
/// the shape a user gets by starting agents by hand inside a terminal session.
fn add_and_start(h: &TuiTestHarness, title: &str, tool: &str) -> String {
    add_and_start_with_command(h, title, tool, None)
}

/// [`add_and_start`], with `command` overriding the instance's agent binary the
/// way `session.agent_command_override` does for a real user.
///
/// This is what decides `expects_shell()`, and with it whether the session's
/// first pane is created holding itself open. The tool name alone cannot say:
/// the `shell` agent's binary is the literal string `shell`, which is not one of
/// the shells that predicate knows, so a shell-tool instance built without an
/// override does not have the shape a terminal session on a real machine has.
fn add_and_start_with_command(
    h: &TuiTestHarness,
    title: &str,
    tool: &str,
    command: Option<&str>,
) -> String {
    h.install_tool_stub(tool);
    let project = h.project_path();
    let mut add_args = vec!["add", project.to_str().unwrap(), "-t", title, "-c", tool];
    if let Some(command) = command {
        add_args.extend(["--cmd-override", command]);
    }
    let add = h.run_cli(&add_args);
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

fn pane_position(h: &TuiTestHarness, pane_id: &str) -> (u32, u32) {
    let position = h.tmux_display_message(pane_id, "#{pane_left},#{pane_top}");
    let (left, top) = position
        .split_once(',')
        .unwrap_or_else(|| panic!("invalid pane position for {pane_id}: {position:?}"));
    (
        left.parse()
            .unwrap_or_else(|_| panic!("invalid pane_left for {pane_id}: {left:?}")),
        top.parse()
            .unwrap_or_else(|_| panic!("invalid pane_top for {pane_id}: {top:?}")),
    )
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

/// Wait until a pane's screen shows `needle`.
///
/// Used to establish that a pane is displaying what a test is about to assert
/// against. Without it, an assertion that nothing acted on the pane's content
/// can pass simply because the content never appeared.
fn wait_for_pane_screen_contains(h: &TuiTestHarness, pane: &str, needle: &str) {
    let start = Instant::now();
    loop {
        let out = tmux(h, &["capture-pane", "-t", pane, "-p", "-J"]);
        let screen = String::from_utf8_lossy(&out.stdout).to_string();
        if screen.contains(needle) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "pane {pane} never showed {needle:?}; screen was {screen:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Wait until no persisted slot still holds a pre-recovery pane id, then return
/// the slots' panes in slot order.
///
/// Slot 0 rebinding is not the signal that recovery finished writing back:
/// recovery writes each slot as it launches it, so reading the whole set on slot
/// 0's signal can catch a rebuilt slot 0 beside a sibling that still holds the
/// pane it had before. That read is stable in isolation and races under a full
/// suite run, which is the worst way for it to be wrong.
fn wait_for_all_slots_rebound(db: &Path, instance_id: &str, old_panes: &[String]) -> Vec<String> {
    let start = Instant::now();
    loop {
        let panes = slot_panes(db, instance_id);
        let rebound =
            panes.len() == old_panes.len() && panes.iter().all(|pane| !old_panes.contains(pane));
        if rebound {
            return panes;
        }
        if start.elapsed() > Duration::from_secs(20) {
            return panes;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Wait until every persisted slot pane belongs to `live`, then return them in
/// slot order.
///
/// Recovery writes each slot back as it launches it, so slot 0 being rebound
/// says nothing about its siblings: reading the whole set on that signal can
/// catch a fresh slot 0 beside a slot still holding its pre-recovery pane id.
/// Membership in the live set is the property that actually distinguishes them.
fn wait_for_slots_within(db: &Path, instance_id: &str, live: &[String]) -> Vec<String> {
    let start = Instant::now();
    loop {
        let panes = slot_panes(db, instance_id);
        if !panes.is_empty() && panes.iter().all(|pane| live.contains(pane)) {
            return panes;
        }
        if start.elapsed() > Duration::from_secs(20) {
            return panes;
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
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let project = h.project_path().to_str().unwrap().to_string();

    let seeds: Vec<SlotSeed> = slots
        .iter()
        .map(|native| SlotSeed {
            agent: "claude",
            native,
            cwd: &project,
        })
        .collect();
    let old_panes = seed_tracked_panes(h, &instance_id, &session_name, &seeds);
    (instance_id, session_name, project, old_panes)
}

/// One pane to track: the agent the hook reports, the native session id it
/// captures, and the on-disk directory it runs in.
struct SlotSeed<'a> {
    agent: &'a str,
    native: &'a str,
    cwd: &'a str,
}

/// Track one pane per seed in a started instance: the first seed lands on the
/// primary pane, the rest on splits. Leaves the home-view TUI running (sized
/// large so the later recovery splits fit) and returns the persisted pane ids in
/// slot order.
fn seed_tracked_panes(
    h: &mut TuiTestHarness,
    instance_id: &str,
    session_name: &str,
    seeds: &[SlotSeed],
) -> Vec<String> {
    let db = db_path(h);

    // Room for the pre-kill splits that establish the tracked panes.
    h.resize_window(session_name, 220, 60);

    let primary = h.tmux_display_message(session_name, "#{pane_id}");
    run_record_pane_as(
        h,
        &primary,
        instance_id,
        seeds[0].agent,
        seeds[0].native,
        seeds[0].cwd,
    );
    for seed in &seeds[1..] {
        let pane = h.split_window_get_pane(session_name);
        run_record_pane_as(h, &pane, instance_id, seed.agent, seed.native, seed.cwd);
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
        &seeds.len().to_string(),
    );

    let old_panes = slot_panes(&db, instance_id);
    assert_eq!(
        old_panes.len(),
        seeds.len(),
        "precondition: one persisted slot per seeded pane"
    );
    old_panes
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

    h.send_keys("R");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);

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

#[test]
#[serial]
fn recover_preserves_nested_left_and_stacked_right_layout() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_nested_layout");
    let slots = [
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb2",
        "cccccccc-cccc-4ccc-8ccc-ccccccccccc3",
    ];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Nested Layout Recover", &slots);
    let db = db_path(&h);

    let primary = old_panes[0].clone();
    for pane in &old_panes[1..] {
        assert!(tmux(&h, &["kill-pane", "-t", pane]).status.success());
    }
    let right_out = tmux(
        &h,
        &[
            "split-window",
            "-h",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &primary,
        ],
    );
    assert!(right_out.status.success());
    let right = String::from_utf8_lossy(&right_out.stdout)
        .trim()
        .to_string();
    let bottom_out = tmux(
        &h,
        &["split-window", "-v", "-P", "-F", "#{pane_id}", "-t", &right],
    );
    assert!(bottom_out.status.success());
    let bottom = String::from_utf8_lossy(&bottom_out.stdout)
        .trim()
        .to_string();
    run_record_pane(&h, &right, &instance_id, slots[1], &_project);
    run_record_pane(&h, &bottom, &instance_id, slots[2], &_project);
    let live_layout = h.tmux_display_message(&session_name, "#{window_layout}");
    let start = Instant::now();
    while sqlite_query(
        &db,
        &format!("SELECT window_layout FROM instance_layout WHERE instance_id='{instance_id}';"),
    ) != live_layout
    {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "nested layout was not persisted"
        );
        std::thread::sleep(Duration::from_millis(150));
    }
    let saved = sqlite_query(
        &db,
        &format!("SELECT window_layout FROM instance_layout WHERE instance_id='{instance_id}';"),
    );
    assert!(
        saved.contains('{') && saved.contains('['),
        "precondition: saved layout must be nested, got {saved:?}"
    );

    let old_panes = slot_panes(&db, &instance_id);
    let old_positions: Vec<(u32, u32)> = old_panes
        .iter()
        .map(|pane| pane_position(&h, pane))
        .collect();
    cold_start(&h, &session_name);
    h.send_keys("R");
    assert_ne!(
        wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]),
        old_panes[0]
    );

    let panes = wait_for_pane_geometry(&h, &session_name, 3);
    let left = panes.iter().filter(|pane| pane[0] == 0).count();
    let right_left = panes.iter().map(|pane| pane[0]).max().unwrap();
    let right: Vec<&Vec<u32>> = panes.iter().filter(|pane| pane[0] == right_left).collect();
    assert_eq!(left, 1, "expected one full-height left pane: {panes:?}");
    assert_eq!(
        right.len(),
        2,
        "expected two stacked right panes: {panes:?}"
    );
    assert_ne!(
        right[0][1], right[1][1],
        "right panes must be vertically stacked"
    );

    let new_panes = slot_panes(&db, &instance_id);
    let new_positions = wait_for_slot_positions(&h, &new_panes, &old_positions);
    assert_eq!(
        new_positions, old_positions,
        "each durable slot must return to its original spatial position"
    );
}

#[test]
#[serial]
fn invalid_layout_snapshot_falls_back_and_recovers_every_pane() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_invalid_layout");
    let slots = [
        "dddddddd-dddd-4ddd-8ddd-ddddddddddd1",
        "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeee2",
    ];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Invalid Layout Recover", &slots);
    let db = db_path(&h);
    cold_start(&h, &session_name);
    sqlite_query(
        &db,
        &format!(
            "INSERT INTO instance_layout(instance_id, window_layout, captured_at)
             VALUES('{instance_id}', 'invalid', 1)
             ON CONFLICT(instance_id) DO UPDATE SET window_layout='invalid';"
        ),
    );
    h.send_keys("R");
    assert_ne!(
        wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]),
        old_panes[0]
    );
    assert_eq!(session_pane_ids(&h, &session_name).len(), slots.len());
    let new_panes = wait_for_all_slots_rebound(&db, &instance_id, &old_panes);
    for (pane, native) in new_panes.iter().zip(slot_natives(&db, &instance_id)) {
        wait_for_pane_start_command_contains(&h, pane, &format!("--resume {native}"));
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

    h.send_keys("R");

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

    h.send_keys("R");

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

// ---------------------------------------------------------------------------
// Requirement: Fresh restart extends to recoverable sessions
// Requirement: C keybinding restarts agent panes clean (recoverable branch)
//   4.1: `C` on a recoverable multi-pane instance rebuilds the session and
//        launches every pane with no resume flag.
// ---------------------------------------------------------------------------

/// Wait until `pane_id` has a start command mentioning `claude`, then return it.
/// Used by the clean-recovery tests, which assert on the ABSENCE of a resume flag
/// and therefore need the respawned command to have landed first.
fn wait_for_launched_claude_command(h: &TuiTestHarness, pane_id: &str) -> String {
    wait_for_pane_start_command_contains(h, pane_id, "claude");
    pane_start_command(h, pane_id)
}

#[test]
#[serial]
fn clean_recover_rebuilds_session_and_launches_every_pane_fresh() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_clean_recover");
    let slots = [
        "dddddddd-dddd-4ddd-8ddd-ddddddddddd0",
        "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeee1",
        "ffffffff-ffff-4fff-8fff-fffffffffff2",
    ];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Clean Recover", &slots);
    let db = db_path(&h);

    cold_start(&h, &session_name);

    // The status bar advertises the clean-recovery branch while the recoverable
    // instance is focused.
    h.assert_screen_contains("Clean Rec");

    h.send_keys("C");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(
        new_slot0, old_panes[0],
        "slot 0 tmux_pane must be rewritten to the rebuilt pane id (clean recovery did not run?)"
    );

    assert!(
        session_exists(&h, &session_name),
        "tmux session must be recreated by clean recovery"
    );
    let live_panes = session_pane_ids(&h, &session_name);
    assert_eq!(
        live_panes.len(),
        slots.len(),
        "clean recovery must rebuild one pane per slot, got {:?}",
        live_panes
    );

    // Durable slot rows keep their recorded native session ids: clean recovery
    // does not consult them, but blanking them would make the instance look
    // unrecoverable before the reconcile chain refreshes them.
    let new_natives = slot_natives(&db, &instance_id);
    let mut got_natives: Vec<&str> = new_natives.iter().map(String::as_str).collect();
    let mut want_natives: Vec<&str> = slots.to_vec();
    got_natives.sort_unstable();
    want_natives.sort_unstable();
    assert_eq!(
        got_natives, want_natives,
        "clean recovery must not discard the durable native session ids"
    );

    let new_panes = slot_panes(&db, &instance_id);
    assert_eq!(new_panes.len(), slots.len());
    for (i, pane) in new_panes.iter().enumerate() {
        assert_ne!(
            *pane, old_panes[i],
            "slot {i} tmux_pane must be updated to the new pane id"
        );
        assert!(
            live_panes.contains(pane),
            "slot {i} new pane {pane} must be live in the rebuilt session {:?}",
            live_panes
        );
        let cmd = wait_for_launched_claude_command(&h, pane);
        assert!(
            !cmd.contains("--resume"),
            "slot {i} must launch clean, got {:?}",
            cmd
        );
        for native_id in &slots {
            assert!(
                !cmd.contains(native_id),
                "slot {i} must not carry any persisted conversation id, got {:?}",
                cmd
            );
        }
    }
}

/// Poll a session's pane geometry until `expected` panes exist, then return their
/// `(left, top, width, height)` rows. Recovery creates the panes and applies the
/// saved layout in separate steps, so reading geometry once can catch the window
/// mid-rebuild under load.
fn wait_for_pane_geometry(h: &TuiTestHarness, session: &str, expected: usize) -> Vec<Vec<u32>> {
    let start = Instant::now();
    loop {
        let out = tmux(
            h,
            &[
                "list-panes",
                "-t",
                session,
                "-F",
                "#{pane_left},#{pane_top},#{pane_width},#{pane_height}",
            ],
        );
        let panes: Vec<Vec<u32>> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| line.split(',').map(|v| v.parse().ok()).collect())
            .collect();
        if panes.len() == expected {
            return panes;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "expected {expected} panes, got {panes:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Poll each slot's pane position until it matches `expected`, so the assertion
/// runs against settled geometry rather than a half-applied layout. Returns the
/// last observed positions either way, so a genuine mismatch still fails loudly.
fn wait_for_slot_positions(
    h: &TuiTestHarness,
    panes: &[String],
    expected: &[(u32, u32)],
) -> Vec<(u32, u32)> {
    let start = Instant::now();
    loop {
        let positions: Vec<(u32, u32)> = panes.iter().map(|p| pane_position(h, p)).collect();
        if positions == expected || start.elapsed() > Duration::from_secs(20) {
            return positions;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
#[serial]
fn clean_recover_preserves_nested_layout_while_launching_fresh() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_clean_nested");
    let slots = [
        "11111111-1111-4111-8111-111111111110",
        "22222222-2222-4222-8222-222222222221",
        "33333333-3333-4333-8333-333333333332",
    ];
    let (instance_id, session_name, project, old_panes) =
        seed_recoverable(&mut h, "Clean Nested Recover", &slots);
    let db = db_path(&h);

    // Rebuild the seeded panes as one left pane plus a vertically split right
    // column, so the saved snapshot is a nested layout rather than flat columns.
    let primary = old_panes[0].clone();
    for pane in &old_panes[1..] {
        assert!(tmux(&h, &["kill-pane", "-t", pane]).status.success());
    }
    let right_out = tmux(
        &h,
        &[
            "split-window",
            "-h",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &primary,
        ],
    );
    assert!(right_out.status.success());
    let right = String::from_utf8_lossy(&right_out.stdout)
        .trim()
        .to_string();
    let bottom_out = tmux(
        &h,
        &["split-window", "-v", "-P", "-F", "#{pane_id}", "-t", &right],
    );
    assert!(bottom_out.status.success());
    let bottom = String::from_utf8_lossy(&bottom_out.stdout)
        .trim()
        .to_string();
    run_record_pane(&h, &right, &instance_id, slots[1], &project);
    run_record_pane(&h, &bottom, &instance_id, slots[2], &project);

    let live_layout = h.tmux_display_message(&session_name, "#{window_layout}");
    let start = Instant::now();
    while sqlite_query(
        &db,
        &format!("SELECT window_layout FROM instance_layout WHERE instance_id='{instance_id}';"),
    ) != live_layout
    {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "nested layout was not persisted"
        );
        std::thread::sleep(Duration::from_millis(150));
    }

    let old_panes = slot_panes(&db, &instance_id);
    let old_positions: Vec<(u32, u32)> = old_panes
        .iter()
        .map(|pane| pane_position(&h, pane))
        .collect();

    cold_start(&h, &session_name);
    h.send_keys("C");
    assert_ne!(
        wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]),
        old_panes[0],
        "clean recovery did not run"
    );

    let panes = wait_for_pane_geometry(&h, &session_name, 3);
    let left = panes.iter().filter(|pane| pane[0] == 0).count();
    let right_left = panes.iter().map(|pane| pane[0]).max().unwrap();
    let right_column: Vec<&Vec<u32>> = panes.iter().filter(|pane| pane[0] == right_left).collect();
    assert_eq!(left, 1, "expected one full-height left pane: {panes:?}");
    assert_eq!(
        right_column.len(),
        2,
        "expected two stacked right panes: {panes:?}"
    );
    assert_ne!(
        right_column[0][1], right_column[1][1],
        "right panes must be vertically stacked"
    );

    let new_panes = slot_panes(&db, &instance_id);
    let new_positions = wait_for_slot_positions(&h, &new_panes, &old_positions);
    assert_eq!(
        new_positions, old_positions,
        "each durable slot must return to its original spatial cell under clean recovery"
    );

    for (i, pane) in new_panes.iter().enumerate() {
        let cmd = wait_for_launched_claude_command(&h, pane);
        assert!(
            !cmd.contains("--resume"),
            "slot {i} must launch clean while its geometry is restored, got {:?}",
            cmd
        );
    }
}

#[test]
#[serial]
fn clean_recover_one_pane_failure_does_not_abort_sibling() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_clean_isolation");
    let slots = [
        "55555555-5555-4555-8555-555555555550", // slot 0: launches normally
        "66666666-6666-4666-8666-666666666661", // slot 1: forced to error
    ];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Clean Recover Isolation", &slots);
    let db = db_path(&h);

    cold_start(&h, &session_name);

    // An agent token with a space is rejected by is_safe_command_token, so the
    // pane plan cannot be built and slot 1 yields an Error outcome -- a genuine
    // per-pane failure, not a degrade.
    sqlite_query(
        &db,
        &format!(
            "UPDATE agent_slot SET agent='bad agent' WHERE instance_id='{instance_id}' AND slot=1;"
        ),
    );

    h.send_keys("C");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "clean recovery did not run");

    let live_panes = session_pane_ids(&h, &session_name);
    assert_eq!(
        live_panes.len(),
        2,
        "a per-pane failure must not abort sibling pane creation, got {:?}",
        live_panes
    );

    let new_panes = slot_panes(&db, &instance_id);
    let cmd = wait_for_launched_claude_command(&h, &new_panes[0]);
    assert!(
        !cmd.contains("--resume"),
        "the healthy sibling must still launch clean, got {:?}",
        cmd
    );

    assert_ne!(new_panes[0], old_panes[0], "slot 0 must be written back");
    assert_ne!(new_panes[1], old_panes[1], "slot 1 must be written back");
}

// ---------------------------------------------------------------------------
// Requirement: Recovery launches each agent exactly once
//   Recovery used to launch the agent twice: once while rebuilding the session
//   and again per slot. The first launch carried the conversation id being
//   recovered, which a real agent refuses to reopen, so it exited at once and
//   took the single-pane session with it. Every other test here uses a stub that
//   sleeps forever and therefore cannot reach that path.
// ---------------------------------------------------------------------------

/// Recover `h`'s instance with `key` and assert the rebuild survived an agent
/// that refuses to run: the session still exists, and the slot's pane is a live
/// pane of it rather than one that vanished with the session.
fn assert_recovery_survives_exiting_agent(
    h: &TuiTestHarness,
    db: &Path,
    instance_id: &str,
    session_name: &str,
    old_panes: &[String],
    key: &str,
) {
    h.send_keys(key);
    let new_slot0 = wait_for_slot0_rebound(db, instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    // The slot is written back before the launched process has had time to exit,
    // so asserting immediately would pass even when the rebuild is about to take
    // the session down with it. Let the failure mode happen first.
    std::thread::sleep(Duration::from_secs(4));

    assert!(
        session_exists(h, session_name),
        "the rebuilt session must survive an agent that exits immediately; \
         screen was:\n{}",
        h.capture_screen()
    );
    let live_panes = session_pane_ids(h, session_name);
    assert!(
        live_panes.contains(&new_slot0),
        "slot 0 must point at a live pane of the rebuilt session, got {new_slot0} in {live_panes:?}"
    );
    assert!(
        !h.capture_screen().contains("can't find pane"),
        "recovery must reach the per-slot launch, screen was:\n{}",
        h.capture_screen()
    );
}

#[test]
#[serial]
fn resume_recovery_survives_an_agent_that_exits_immediately() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_exit_resume");
    let slots = ["77777777-7777-4777-8777-777777777770"];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Exit Resume", &slots);
    let db = db_path(&h);

    // From here the agent refuses to run, the way a real one does when asked to
    // reopen a conversation that is already open.
    h.install_exiting_tool_stub("claude", 1);
    cold_start(&h, &session_name);

    assert_recovery_survives_exiting_agent(&h, &db, &instance_id, &session_name, &old_panes, "R");
}

#[test]
#[serial]
fn clean_recovery_survives_an_agent_that_exits_immediately() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_exit_clean");
    let slots = ["88888888-8888-4888-8888-888888888880"];
    let (instance_id, session_name, _project, old_panes) =
        seed_recoverable(&mut h, "Exit Clean", &slots);
    let db = db_path(&h);

    h.install_exiting_tool_stub("claude", 1);
    cold_start(&h, &session_name);

    assert_recovery_survives_exiting_agent(&h, &db, &instance_id, &session_name, &old_panes, "C");

    let cmd = pane_start_command(&h, &slot_panes(&db, &instance_id)[0]);
    assert!(
        !cmd.contains("--resume"),
        "clean recovery must still launch without a resume flag, got {cmd:?}"
    );
}

// ---------------------------------------------------------------------------
// Requirement: A tracked pane relaunches as the agent its slot recorded
//   An instance whose own tool is a shell with an agent adopted into every slot:
//   the shape a user gets by starting agents by hand inside a terminal session.
//   Slot 0 used to be rebuilt from the instance's tool, so it came back running
//   the shell where an agent belonged, while slot 1 came back correctly.
// ---------------------------------------------------------------------------

/// A recovery settle short enough that a test can wait it out, for tests whose
/// subject is not the settle itself.
const SHORT_SETTLE: Duration = Duration::from_millis(300);

#[test]
#[serial]
fn recover_relaunches_an_adopted_slot_as_its_agent_not_the_instance_tool() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_adopted_shell");
    h.set_env(
        "AGENT_OF_EMPIRES_RECOVERY_SETTLE_MS",
        &SHORT_SETTLE.as_millis().to_string(),
    );
    h.install_tool_stub("claude");
    let other = h.home_path().join("other-dir");
    std::fs::create_dir_all(&other).expect("create the second slot's cwd");
    let other = other.to_str().unwrap().to_string();
    let project = h.project_path().to_str().unwrap().to_string();

    let instance_id = add_and_start(&h, "Adopted Shell", "shell");
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Adopted Shell");
    let db = db_path(&h);

    let left = "cf75f4aa-4812-4bfc-9f88-075d3824b5fa";
    let right = "55245107-81ba-4b3e-b740-247d27802d3f";
    let old_panes = seed_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[
            SlotSeed {
                agent: "claude",
                native: left,
                cwd: &project,
            },
            SlotSeed {
                agent: "claude",
                native: right,
                cwd: &other,
            },
        ],
    );

    cold_start(&h, &session_name);
    h.send_keys("R");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    let new_panes = wait_for_all_slots_rebound(&db, &instance_id, &old_panes);
    assert_eq!(new_panes.len(), 2, "both slots must be written back");

    // Slot 0 is the interesting one: the instance's own tool describes nothing
    // about the agent the user started in that pane.
    wait_for_pane_start_command_contains(&h, &new_panes[0], &format!("claude --resume {left}"));
    wait_for_pane_start_command_contains(&h, &new_panes[1], &format!("claude --resume {right}"));

    let live = session_pane_ids(&h, &session_name);
    assert!(
        live.contains(&new_panes[0]) && live.contains(&new_panes[1]),
        "both adopted agents must still be there after recovery, got {live:?}"
    );

    // Recovery writes the new pane ids back before it settles and checks which
    // slots came back, so asserting the absence of a report right after the
    // write-back would assert it against a check that has not run yet. Wait out
    // the (shortened) settle first so the absence means what it says.
    std::thread::sleep(SHORT_SETTLE * 4);
    assert_eq!(
        lost_events(&db, &instance_id),
        "",
        "a recovery where every pane is present must report no failure"
    );
}

// ---------------------------------------------------------------------------
// Requirement: A tracked pane relaunches as the agent its slot recorded
//   The same shape as above, but for the instance a user actually has: one whose
//   command is a real shell. Recovery kills each pane's process tree from
//   outside tmux before respawning it, and the session's first pane used to be
//   created without remain-on-exit whenever the instance expected a shell -- so
//   that kill destroyed the pane and the respawn had nothing to target. With one
//   slot the destroyed pane is the session's only pane, so the session went with
//   it and recovery looked like it had done nothing at all.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn recover_relaunches_the_only_slot_of_a_shell_command_instance() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_shell_command");
    h.set_env(
        "AGENT_OF_EMPIRES_RECOVERY_SETTLE_MS",
        &SHORT_SETTLE.as_millis().to_string(),
    );
    h.install_tool_stub("claude");
    let project = h.project_path().to_str().unwrap().to_string();

    let instance_id = add_and_start_with_command(&h, "Shell Command", "shell", Some("/bin/sh"));
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Shell Command");
    let db = db_path(&h);

    let native = "3f6d4c21-9b7e-4a55-8c10-2d9e6b4f7a83";
    let old_panes = seed_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[SlotSeed {
            agent: "claude",
            native,
            cwd: &project,
        }],
    );

    cold_start(&h, &session_name);
    h.send_keys("R");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    wait_for_pane_start_command_contains(&h, &new_slot0, &format!("claude --resume {native}"));

    let live = session_pane_ids(&h, &session_name);
    assert_eq!(
        live,
        vec![new_slot0.clone()],
        "the instance's only slot must still be there after recovery"
    );

    std::thread::sleep(SHORT_SETTLE * 4);
    assert_eq!(
        lost_events(&db, &instance_id),
        "",
        "a recovery where every pane is present must report no failure"
    );
}

// ---------------------------------------------------------------------------
// Requirement: Recovery reports slots that did not come back
//   A pane that is created, respawned and then disappears is invisible to the
//   launch outcomes, so recovery used to hand back fewer panes than the user had
//   and still report success.
// ---------------------------------------------------------------------------

/// The `lost` events recorded for an instance, one `slot|detail` line each.
fn lost_events(db: &Path, instance_id: &str) -> String {
    sqlite_query(
        db,
        &format!(
            "SELECT slot || '|' || detail FROM events \
             WHERE instance_id='{instance_id}' AND kind='lost';"
        ),
    )
}

#[test]
#[serial]
fn recovery_reports_a_slot_whose_pane_disappears() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("cold_start_lost_pane");
    // Whether the relaunched pane has died by the time recovery decides which
    // slots came back depends on process scheduling, so under load the default
    // window can close first. Widen it for this test alone: recovery's settle is
    // a blocking wait, and lengthening it for every test perturbs the timing of
    // the whole suite.
    h.set_env("AGENT_OF_EMPIRES_RECOVERY_SETTLE_MS", "3000");
    let instance_id = add_and_start(&h, "Lost Pane", "claude");
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, "Lost Pane");
    let db = db_path(&h);
    let project = h.project_path().to_str().unwrap().to_string();

    // Slot 1 is a shell pane adopted next to the agent. Its recorded binary
    // exits at once here, so the pane closes moments after recovery respawns it
    // -- surviving its own relaunch, then vanishing.
    h.install_exiting_tool_stub("shell", 0);

    let old_panes = seed_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[
            SlotSeed {
                agent: "claude",
                native: "99999999-9999-4999-8999-999999999990",
                cwd: &project,
            },
            SlotSeed {
                agent: "shell",
                native: "99999999-9999-4999-8999-999999999991",
                cwd: &project,
            },
        ],
    );

    cold_start(&h, &session_name);
    h.send_keys("R");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    wait_for_count(
        &h,
        &db,
        &format!("SELECT count(*) FROM events WHERE instance_id='{instance_id}' AND kind='lost';"),
        "1",
    );

    let reported = lost_events(&db, &instance_id);
    assert!(
        reported.starts_with("1|") && reported.contains("shell") && reported.contains(&project),
        "the report must name the slot by the agent and directory it recorded, got {reported:?}"
    );

    let new_panes = slot_panes(&db, &instance_id);
    let live = session_pane_ids(&h, &session_name);
    assert!(
        live.contains(&new_panes[0]),
        "the surviving sibling must still be there, got {live:?}"
    );
    assert!(
        !live.contains(&new_panes[1]),
        "the reported slot's pane really is gone, got {live:?}"
    );
    assert_eq!(
        live.len(),
        1,
        "a missing pane must not be relaunched or recreated, got {live:?}"
    );
}

// ===========================================================================
// INDEPENDENT ACCEPTANCE (tester) -- written from the spec, not from the
// implementer's own tests. Covers the three gaps the implementer flagged:
//   1. multi-slot shell-command instance, layout preserved
//   2. remain-on-exit after relaunch describes the pane's own agent
//   3. the C (clean) path, not just R
// ===========================================================================

const AT_SETTLE: Duration = Duration::from_millis(300);

/// Pane-level `remain-on-exit` as tmux reports it, or `""` when the option is
/// not set on the pane at all. The distinction matters: "not set" is exactly
/// the state a relaunch leaves behind when it declines to write the value.
fn pane_remain_on_exit(h: &TuiTestHarness, pane: &str) -> String {
    let out = tmux(h, &["show-options", "-p", "-t", pane, "remain-on-exit"]);
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string()
}

/// Persisted `agent_slot.agent` values for an instance, in ascending slot order
/// (aligned element-for-element with [`slot_panes`]).
fn slot_agents(db: &Path, instance_id: &str) -> Vec<String> {
    sqlite_query(
        db,
        &format!("SELECT agent FROM agent_slot WHERE instance_id='{instance_id}' ORDER BY slot;"),
    )
    .lines()
    .filter(|l| !l.trim().is_empty())
    .map(str::to_string)
    .collect()
}

/// Assert that the set of seeded conversation ids survived recovery exactly
/// once. Slot numbers are assigned by ascending pane index, not by the order a
/// test seeded them, so only the set is meaningful across the boundary.
fn assert_same_natives(db: &Path, instance_id: &str, seeded: &[&str]) {
    let mut got: Vec<String> = slot_natives(db, instance_id);
    let mut want: Vec<String> = seeded.iter().map(|s| s.to_string()).collect();
    got.sort();
    want.sort();
    assert_eq!(
        got, want,
        "every seeded conversation must survive recovery exactly once"
    );
}

/// A shell-command instance (`expects_shell()` true) with `agents.len()` tracked
/// slots, seeded and cold-started. Returns `(instance_id, session, db, old_panes)`.
fn at_seed_shell_instance(
    h: &mut TuiTestHarness,
    title: &str,
    agents: &[&str],
    natives: &[&str],
) -> (String, String, PathBuf, Vec<String>) {
    h.set_env(
        "AGENT_OF_EMPIRES_RECOVERY_SETTLE_MS",
        &AT_SETTLE.as_millis().to_string(),
    );
    h.install_tool_stub("claude");
    let project = h.project_path().to_str().unwrap().to_string();
    let instance_id = add_and_start_with_command(h, title, "shell", Some("/bin/sh"));
    for agent in agents {
        if *agent != "shell" {
            h.install_tool_stub(agent);
        }
    }
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let db = db_path(h);
    let seeds: Vec<SlotSeed> = agents
        .iter()
        .zip(natives.iter())
        .map(|(agent, native)| SlotSeed {
            agent,
            native,
            cwd: &project,
        })
        .collect();
    let old_panes = seed_tracked_panes(h, &instance_id, &session_name, &seeds);
    (instance_id, session_name, db, old_panes)
}

/// AT-1: three slots on an instance whose command is a real shell. Every slot
/// must come back as the agent it recorded, in the position it held.
#[test]
#[serial]
fn at_shell_command_instance_recovers_all_three_slots_and_layout() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_shell_three_slots");
    let natives = [
        "a1a1a1a1-1111-4111-8111-111111111111",
        "b2b2b2b2-2222-4222-8222-222222222222",
        "c3c3c3c3-3333-4333-8333-333333333333",
    ];
    let (instance_id, session_name, db, old_panes) = at_seed_shell_instance(
        &mut h,
        "AT Shell Three",
        &["claude", "claude", "claude"],
        &natives,
    );
    let old_positions: Vec<(u32, u32)> = old_panes.iter().map(|p| pane_position(&h, p)).collect();

    cold_start(&h, &session_name);
    h.send_keys("R");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    let new_panes = wait_for_all_slots_rebound(&db, &instance_id, &old_panes);
    assert_eq!(new_panes.len(), 3, "every slot must be written back");

    wait_for_pane_geometry(&h, &session_name, 3);
    let live = session_pane_ids(&h, &session_name);
    assert_eq!(
        live.len(),
        3,
        "a shell-command instance must not lose panes to the relaunch, got {live:?}"
    );
    assert_same_natives(&db, &instance_id, &natives);
    let new_natives = slot_natives(&db, &instance_id);
    for (i, pane) in new_panes.iter().enumerate() {
        assert!(
            live.contains(pane),
            "slot {i} pane {pane} is gone: {live:?}"
        );
        wait_for_pane_start_command_contains(
            &h,
            pane,
            &format!("claude --resume {}", new_natives[i]),
        );
    }

    let new_positions = wait_for_slot_positions(&h, &new_panes, &old_positions);
    assert_eq!(
        new_positions, old_positions,
        "each slot must return to the position it held before the cold start"
    );

    std::thread::sleep(AT_SETTLE * 5);
    assert_eq!(
        lost_events(&db, &instance_id),
        "",
        "no slot went missing, so nothing may be reported lost"
    );
}

/// AT-2: the relaunch must leave remain-on-exit describing the agent that now
/// runs in the pane -- on for an agent pane, off for a shell pane. Holding the
/// pane open across the kill is a means, not the end state.
#[test]
#[serial]
fn at_relaunched_pane_remain_on_exit_matches_its_own_agent() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_remain_on_exit");
    let natives = [
        "d4d4d4d4-4444-4444-8444-444444444444",
        "e5e5e5e5-5555-4555-8555-555555555555",
    ];
    let (instance_id, session_name, db, old_panes) =
        at_seed_shell_instance(&mut h, "AT Remain", &["claude", "shell"], &natives);

    cold_start(&h, &session_name);
    h.send_keys("R");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    let new_panes = wait_for_all_slots_rebound(&db, &instance_id, &old_panes);
    assert_eq!(new_panes.len(), 2);
    wait_for_pane_geometry(&h, &session_name, 2);

    // Slot numbers follow pane index, not seed order, so read back which slot
    // ended up recording which agent instead of assuming.
    let agents = slot_agents(&db, &instance_id);
    let mut sorted = agents.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["claude".to_string(), "shell".to_string()]);

    for (i, agent) in agents.iter().enumerate() {
        let pane = &new_panes[i];
        let (needle, want) = match agent.as_str() {
            "claude" => ("claude --resume", "on"),
            _ => ("shell", "off"),
        };
        wait_for_pane_start_command_contains(&h, pane, needle);
        assert_eq!(
            pane_remain_on_exit(&h, pane),
            want,
            "slot {i} runs {agent}, so its remain-on-exit must be {want}: an agent pane \
             is held open when its process exits, a shell pane closes with the user's \
             shell instead of being stuck open by the setting the relaunch used to \
             survive its own kill"
        );
    }

    std::thread::sleep(AT_SETTLE * 5);
    assert_eq!(lost_events(&db, &instance_id), "", "no slot went missing");
}

/// AT-3: the C (clean) path on the shape where losing the pane loses the whole
/// session -- a shell-command instance with a single slot. The implementer
/// covered this shape on R only.
#[test]
#[serial]
fn at_clean_recovery_of_a_single_slot_shell_command_instance() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_clean_single_shell");
    let natives = ["f6f6f6f6-6666-4666-8666-666666666666"];
    let (instance_id, session_name, db, old_panes) =
        at_seed_shell_instance(&mut h, "AT Clean Single", &["claude"], &natives);

    cold_start(&h, &session_name);
    h.assert_screen_contains("Clean Rec");
    h.send_keys("C");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "clean recovery did not run");

    assert!(
        session_exists(&h, &session_name),
        "clean recovery must leave the session alive"
    );
    let live = session_pane_ids(&h, &session_name);
    assert_eq!(
        live,
        vec![new_slot0.clone()],
        "the only slot must still be there after a clean recovery, got {live:?}"
    );

    let cmd = wait_for_launched_claude_command(&h, &new_slot0);
    assert!(
        !cmd.contains("--resume") && !cmd.contains(natives[0]),
        "clean recovery must launch fresh, got {cmd:?}"
    );
    assert!(
        !cmd.contains("/bin/sh"),
        "the instance's shell override must not replace the slot's agent, got {cmd:?}"
    );

    std::thread::sleep(AT_SETTLE * 5);
    assert_eq!(lost_events(&db, &instance_id), "", "no slot went missing");
}

/// AT-4: the C path with more than one slot on the same shape.
#[test]
#[serial]
fn at_clean_recovery_of_a_multi_slot_shell_command_instance() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_clean_multi_shell");
    let natives = [
        "17171717-7777-4777-8777-777777777777",
        "18181818-8888-4888-8888-888888888888",
    ];
    let (instance_id, session_name, db, old_panes) =
        at_seed_shell_instance(&mut h, "AT Clean Multi", &["claude", "claude"], &natives);

    cold_start(&h, &session_name);
    h.send_keys("C");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "clean recovery did not run");

    wait_for_pane_geometry(&h, &session_name, 2);
    let live = session_pane_ids(&h, &session_name);
    assert_eq!(
        live.len(),
        2,
        "clean recovery must rebuild one live pane per slot, got {live:?}"
    );

    // Slot 0 rebinding does not mean every slot has been written back: recovery
    // writes each slot as it launches it, so reading the whole set on slot 0's
    // signal can catch a new slot 0 next to a stale sibling.
    let new_panes = wait_for_slots_within(&db, &instance_id, &live);
    assert_eq!(new_panes.len(), 2);
    for (i, pane) in new_panes.iter().enumerate() {
        assert!(
            live.contains(pane),
            "slot {i} pane {pane} is gone: {live:?}"
        );
        let cmd = wait_for_launched_claude_command(&h, pane);
        assert!(
            !cmd.contains("--resume"),
            "slot {i} must launch fresh: {cmd:?}"
        );
        for native in &natives {
            assert!(
                !cmd.contains(native),
                "slot {i} must carry no persisted conversation id, got {cmd:?}"
            );
        }
        assert!(
            !cmd.contains("/bin/sh"),
            "the instance's shell override must not replace the slot's agent, got {cmd:?}"
        );
    }

    std::thread::sleep(AT_SETTLE * 5);
    assert_eq!(lost_events(&db, &instance_id), "", "no slot went missing");
}

// ===========================================================================
// INDEPENDENT ACCEPTANCE (tester), batch 2: auto-confirm / CAT.
// The author's own coverage here is unit-level only; nothing exercised the
// keystroke path end to end, and sending a keystroke is not a read-only act.
// ===========================================================================

const CAT_FLAG: &str = "--dangerously-load-development-channels";

/// Turn Cross Agent Team on for an already-created session, the way
/// `xats_identity.rs` does: there is no CLI flag, so it is written into the
/// store before the TUI loads it.
fn enable_cross_agent_team(h: &TuiTestHarness, title: &str) {
    let path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    };
    let content = std::fs::read_to_string(&path).expect("read sessions.json");
    let mut sessions: serde_json::Value = serde_json::from_str(&content).expect("parse sessions");
    let session = sessions
        .as_array_mut()
        .expect("sessions array")
        .iter_mut()
        .find(|s| s["title"] == title)
        .expect("created session");
    // The pane owns this flag and the instance field is derived from it, so
    // setting only the instance field is undone the moment the record is read
    // back.
    session["cross_agent_team"] = serde_json::Value::Bool(true);
    session["primary_pane"]["cross_agent_team"] = serde_json::Value::Bool(true);
    let instance_id = session["id"].as_str().expect("session id").to_string();
    std::fs::write(&path, serde_json::to_string_pretty(&sessions).unwrap())
        .expect("enable Cross Agent Team");

    // The launch that already happened provisioned slot 0 with the flag off,
    // and a stored slot config wins over the instance record when a pane is
    // rebuilt -- so turning it on after the fact has to reach the slot too.
    sqlite_query(
        &db_path(h),
        &format!("UPDATE agent_slot SET cross_agent_team=1 WHERE instance_id='{instance_id}';"),
    );
}

/// A Cross Agent Team Claude instance recovered with a second slot that records
/// a different agent.
///
/// Two things are being watched at once, and they are the two the author had
/// only unit coverage for. The adopted pane must be decorated for the agent it
/// actually runs, and -- because that pane shows text auto-confirm's marker
/// table matches -- it must receive no keystroke at all. The second is what the
/// original defect was: a pane that was not launched by this flow was sent
/// Enter, and the shell in it ran what the user had typed.
#[test]
#[serial]
fn at_cat_recovery_decorates_each_pane_and_types_into_no_other() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_cat_multi_pane");
    let title = "AT CAT Multi";
    let instance_id = add_and_start(&h, title, "claude");
    enable_cross_agent_team(&h, title);

    // The adopted pane: it prints a line auto-confirm's marker table matches,
    // then records every line its stdin receives. An Enter that reaches it
    // shows up as a recorded line -- the original defect, made observable.
    //
    // Copilot rather than Codex, because Codex's Cross Agent Team bootstrap
    // checks a live app-server port and pre-registers the pane before it execs
    // the agent. Neither exists here, so the stub would never run and the
    // "nothing was typed" assertion would hold vacuously -- the pane it speaks
    // for would not exist. Copilot is decorated by neither integration, so it
    // reaches the stub, which is what makes the assertion mean anything.
    let typed = h.home_path().join("adopted-received-input.txt");
    h.install_stub_script(
        "copilot",
        &format!(
            "#!/bin/sh\n\
             : > '{0}'\n\
             printf '  WARNING: Loading development channels\\n'\n\
             printf '  \\342\\235\\257 1. I am using this for local development\\n'\n\
             while IFS= read -r line; do echo \"GOT[$line]\" >> '{0}'; done\n\
             sleep 2147483647\n",
            typed.display()
        ),
    );

    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let db = db_path(&h);
    let project = h.project_path().to_str().unwrap().to_string();

    let old_panes = seed_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[
            SlotSeed {
                agent: "claude",
                native: "aaaa1111-1111-4111-8111-111111111111",
                cwd: &project,
            },
            SlotSeed {
                agent: "copilot",
                native: "bbbb2222-2222-4222-8222-222222222222",
                cwd: &project,
            },
        ],
    );

    cold_start(&h, &session_name);
    h.send_keys("R");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    assert_ne!(new_slot0, old_panes[0], "recovery did not run");

    let new_panes = wait_for_all_slots_rebound(&db, &instance_id, &old_panes);
    assert_eq!(new_panes.len(), 2);
    wait_for_pane_geometry(&h, &session_name, 2);

    // Slot order follows pane index, so read back which slot recorded which
    // agent instead of assuming.
    let agents = slot_agents(&db, &instance_id);
    for (i, agent) in agents.iter().enumerate() {
        let pane = &new_panes[i];
        wait_for_pane_start_command_contains(&h, pane, agent);
        let cmd = pane_start_command(&h, pane);
        match agent.as_str() {
            "claude" => assert!(
                cmd.contains(CAT_FLAG),
                "the instance's own Claude pane must keep its Cross Agent Team \
                 flag, got {cmd:?}"
            ),
            other => {
                assert!(
                    !cmd.contains(CAT_FLAG),
                    "an adopted {other} pane must not be handed Claude's flag, \
                     got {cmd:?}"
                );
                assert!(
                    cmd.contains(other),
                    "the adopted pane must run its own binary, got {cmd:?}"
                );
            }
        }
    }

    // "Received nothing" is only evidence if the thing that would have recorded
    // it ran at all. The Codex bootstrap checks a port and pre-registers before
    // it execs this stub, so any of those failing would leave no file -- and an
    // absent file read as an empty one would pass this test without the pane
    // ever having existed to be typed into.
    let stub_started = {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if typed.exists() {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    assert!(
        stub_started,
        "the adopted pane's agent never started, so this test could prove nothing \
         about what was typed into it"
    );

    // And it must be showing the text that makes it a tempting target: without
    // this, "nothing was typed" could just mean nothing matched.
    let adopted_pane = new_panes
        .iter()
        .zip(agents.iter())
        .find(|(_, agent)| agent.as_str() != "claude")
        .map(|(pane, _)| pane.clone())
        .expect("an adopted non-Claude pane");
    wait_for_pane_screen_contains(&h, &adopted_pane, "I am using this for local development");

    // Auto-confirm runs synchronously inside recovery and is bounded by its own
    // 12s deadline; wait past it so "nothing was typed" is a settled fact.
    std::thread::sleep(Duration::from_secs(14));
    let received = std::fs::read_to_string(&typed).expect("the stub's record file");
    assert_eq!(
        received, "",
        "the adopted pane shows text the marker table matches, but it is not a \
         Claude pane this flow launched -- it must receive no keystroke. got {received:?}"
    );
}

// ---------------------------------------------------------------------------
// AT-B3: independent acceptance tests (batch 3, tester).
// Focus: shell-launched codex recovery (slot path and no-slot path), the
// default-command guard, and identity-key injection for adopted codex slots.
// ---------------------------------------------------------------------------

/// AT-B3-1: the dispatched headline form, slot path. A shell-command instance
/// whose tracked slots record a shell and a codex conversation must bring the
/// codex pane back with `codex resume <its own thread id>`, and remain-on-exit
/// must describe each pane's own agent afterwards.
#[test]
#[serial]
fn at_b3_shell_instance_recovers_codex_slot_as_codex() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_b3_shell_codex_slot");
    let natives = [
        "5be11000-0000-4000-8000-000000000000",
        "019d1af9-b3b3-4333-8333-333333333333",
    ];
    let (instance_id, session_name, db, old_panes) =
        at_seed_shell_instance(&mut h, "AT B3 Shell Codex", &["shell", "codex"], &natives);
    assert_eq!(old_panes.len(), 2);

    cold_start(&h, &session_name);
    h.send_keys("R");

    let _ = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    let new_panes = wait_for_all_slots_rebound(&db, &instance_id, &old_panes);
    let agents = slot_agents(&db, &instance_id);
    let new_natives = slot_natives(&db, &instance_id);

    let codex_at = agents
        .iter()
        .position(|a| a == "codex")
        .expect("a slot must still record codex after recovery");
    let shell_at = agents
        .iter()
        .position(|a| a == "shell")
        .expect("a slot must still record shell after recovery");

    wait_for_pane_start_command_contains(
        &h,
        &new_panes[codex_at],
        &format!("codex resume {}", new_natives[codex_at]),
    );
    let codex_cmd = pane_start_command(&h, &new_panes[codex_at]);
    assert!(
        !codex_cmd.contains("claude"),
        "the codex slot must not be relaunched as claude, got {codex_cmd:?}"
    );
    let shell_cmd = pane_start_command(&h, &new_panes[shell_at]);
    assert!(
        !shell_cmd.contains("codex") && !shell_cmd.contains("claude"),
        "the shell slot must come back as a shell, got {shell_cmd:?}"
    );

    assert_eq!(
        pane_remain_on_exit(&h, &new_panes[codex_at]),
        "on",
        "a relaunched codex pane must keep remain-on-exit on"
    );
    assert_eq!(
        pane_remain_on_exit(&h, &new_panes[shell_at]),
        "off",
        "a relaunched shell pane must have remain-on-exit off"
    );
}

/// AT-B3-2: what the Cross Agent Team flag does on a shell instance -- nothing.
/// `is_cross_agent_team()` gates on the INSTANCE tool being claude/codex, so a
/// shell instance's adopted codex slot resumes its conversation but gets no
/// xats bootstrap and no identity key, and none is minted for the slot. This
/// pins that semantics; the acceptance report carries what it means for
/// hand-launched agents in shell sessions.
#[test]
#[serial]
fn at_b3_cat_flag_on_a_shell_instance_is_structurally_inert() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_b3_cat_codex_key");
    h.set_env(
        "AGENT_OF_EMPIRES_RECOVERY_SETTLE_MS",
        &AT_SETTLE.as_millis().to_string(),
    );
    h.install_tool_stub("claude");
    h.install_tool_stub("codex");
    let project = h.project_path().to_str().unwrap().to_string();
    let title = "AT B3 CAT Codex";

    let add = h.run_cli(&[
        "add",
        &project,
        "-t",
        title,
        "-c",
        "shell",
        "--cmd-override",
        "/bin/sh",
    ]);
    assert!(
        add.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let sessions_path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    };
    let content = std::fs::read_to_string(&sessions_path).expect("read sessions.json");
    let mut sessions: serde_json::Value = serde_json::from_str(&content).unwrap();
    let instance_id = {
        let session = sessions
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|s| s["title"] == title)
            .expect("created session");
        session["cross_agent_team"] = serde_json::Value::Bool(true);
        session["id"].as_str().unwrap().to_string()
    };
    std::fs::write(
        &sessions_path,
        serde_json::to_string_pretty(&sessions).unwrap(),
    )
    .expect("enable Cross Agent Team");
    let start = h.run_cli_in_tmux(&["session", "start", title]);
    assert!(
        start.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let db = db_path(&h);
    let native = "019d1af9-c4c4-4444-8444-444444444444";
    let seeds = [SlotSeed {
        agent: "codex",
        native,
        cwd: &project,
    }];
    let old_panes = seed_tracked_panes(&mut h, &instance_id, &session_name, &seeds);

    cold_start(&h, &session_name);
    h.send_keys("R");

    let new_slot0 = wait_for_slot0_rebound(&db, &instance_id, &old_panes[0]);
    wait_for_pane_start_command_contains(&h, &new_slot0, &format!("resume {native}"));
    let cmd = pane_start_command(&h, &new_slot0);

    // Shipped semantics, pinned: `is_cross_agent_team()` requires the INSTANCE
    // tool to be claude/codex, so a shell instance's flag is structurally inert.
    // The adopted codex slot resumes its conversation but receives neither the
    // xats bootstrap nor an identity key, and no key is minted for the slot.
    // (The new-session dialog hides the CAT field for shell, so this state is
    // only reachable by editing the store -- which is also how the flag default
    // could reach it.) The acceptance report carries the implication.
    assert!(
        !cmd.contains("--remote"),
        "shipped: a shell instance takes no CAT integration, got {cmd:?}"
    );
    assert!(
        !cmd.contains("XATS_IDENTITY_KEY"),
        "shipped: a shell instance injects no identity key, got {cmd:?}"
    );
    let persisted = sqlite_query(
        &db,
        &format!(
            "SELECT xats_identity_key FROM agent_slot \
             WHERE instance_id='{instance_id}' AND slot=0;"
        ),
    );
    assert_eq!(
        persisted, "",
        "shipped: no key is minted for an adopted slot of a shell instance"
    );
}

/// AT-B3-3: R on a live shell-override instance with no tracked slots -- the
/// exact shape of every real shell session on the reporting machine (command
/// `/bin/zsh`, hooks not yet installed, so no slots).
///
/// Two things must hold. The session must survive the restart: a shell pane is
/// created with remain-on-exit off, and until `43bbd087` the no-slot path
/// killed the pane's process tree before respawning, so the single-pane
/// session died with its pane and R destroyed what it was asked to restart.
/// And the pane must come back as its override: the instance carries a
/// command override, so the restart relaunches that override and never reads
/// the pane -- even when the pane demonstrably runs codex.
#[test]
#[serial]
fn at_b3_r_on_a_live_no_slot_shell_instance_relaunches_its_override() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_b3_override_skips");
    let instance_id = add_and_start_with_command(&h, "AT B3 Override", "shell", Some("/bin/sh"));
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "AT B3 Override");

    let Some(codex_bin) = h.install_native_stub("codex") else {
        eprintln!("Skipping test: no C compiler to build a native stub");
        return;
    };

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    // A launch provisions slot 0 to hold that pane's config, so the untracked
    // state this test is about has to be constructed rather than assumed.
    sqlite_query(
        &db,
        &format!("DELETE FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        sqlite_query(
            &db,
            &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
        ),
        "0",
        "precondition: no tracked slots"
    );

    // The hand-off: the pane now runs codex, and nothing recorded it.
    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    let respawn = tmux(
        &h,
        &[
            "respawn-pane",
            "-k",
            "-t",
            &primary,
            codex_bin.to_str().unwrap(),
        ],
    );
    assert!(respawn.status.success());
    let start = Instant::now();
    loop {
        if h.tmux_display_message(&primary, "#{pane_current_command}") == "codex" {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the pane never came up running codex"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    h.send_keys("R");

    wait_for_pane_start_command_contains(&h, &primary, "sh");
    assert!(
        session_exists(&h, &session_name),
        "the restart must not destroy the session it restarts"
    );
    let cmd = pane_start_command(&h, &primary);
    assert!(
        !cmd.contains("codex"),
        "a command-override instance relaunches its override and never reads \
         the pane; got {cmd:?}"
    );
    // The restart must leave the shell pane able to die normally again: the
    // hold is a means, not the end state.
    assert_eq!(
        pane_remain_on_exit(&h, &primary),
        "off",
        "a relaunched shell pane must have remain-on-exit off"
    );
}

/// AT-B3-4: the same guard on a codex instance. `aoe add -c codex` stores the
/// default command `codex` (set_default_command), which is not an override --
/// `has_command_override()` is false -- yet the guard tests `command.is_empty()`
/// and so skips the pane read here too. A codex instance whose pane was handed
/// to another agent is relaunched as codex. Pinned as shipped; flagged in the
/// report as an imprecise predicate.
#[test]
#[serial]
fn at_b3_r_on_a_default_command_codex_instance_does_not_read_the_pane() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("at_b3_codex_default_cmd");
    let instance_id = add_and_start_with_command(&h, "AT B3 Codex Default", "codex", None);
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "AT B3 Codex Default");

    let Some(claude_bin) = h.install_native_stub("claude") else {
        eprintln!("Skipping test: no C compiler to build a native stub");
        return;
    };

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    // A launch provisions slot 0 to hold that pane's config, so the untracked
    // state this test is about has to be constructed rather than assumed.
    sqlite_query(
        &db,
        &format!("DELETE FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        sqlite_query(
            &db,
            &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
        ),
        "0",
        "precondition: no tracked slots"
    );

    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    let respawn = tmux(
        &h,
        &[
            "respawn-pane",
            "-k",
            "-t",
            &primary,
            claude_bin.to_str().unwrap(),
        ],
    );
    assert!(respawn.status.success());
    let start = Instant::now();
    loop {
        if h.tmux_display_message(&primary, "#{pane_current_command}") == "claude" {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the pane never came up running the native claude stub"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    h.send_keys("R");

    wait_for_pane_start_command_contains(&h, &primary, "codex");
    let cmd = pane_start_command(&h, &primary);
    assert!(
        !cmd.contains("claude"),
        "shipped behavior: the default-command guard never reads the pane of a \
         codex instance; got {cmd:?}"
    );
}
