//! Shared fixtures for the `preserve-claude-model-on-restart` e2e tests.
//!
//! Every test in the `claude_model_*` modules drives the real `aoe` binary
//! through the harness (private tmux socket, isolated `$HOME`, `$TMUX` and
//! `$TMUX_PANE` removed) and observes the outcome from outside the process:
//! the on-disk `aoe.db` via the `sqlite3` CLI, tmux's own
//! `#{pane_start_command}` for a respawned pane, and the rendered TUI screen.
//!
//! Nothing here reaches into AoE's internals, and no helper ever kills a tmux
//! server or sweeps sessions by pattern -- only the harness's own session and
//! the exact managed session a test created.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::harness::TuiTestHarness;

/// The model value claude writes for entries it synthesised itself; detection
/// must treat it as "no model" rather than as an observation.
pub const SYNTHETIC_MODEL: &str = "<synthetic>";

pub fn sqlite3_available() -> bool {
    Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Skip the calling test when the `sqlite3` CLI is missing. The store is a real
/// file on disk and is read from outside the binary.
macro_rules! require_sqlite3 {
    () => {
        if !$crate::claude_model_support::sqlite3_available() {
            eprintln!("Skipping test: sqlite3 CLI not available");
            return;
        }
    };
}
pub(crate) use require_sqlite3;

// ---------------------------------------------------------------------------
// Store access
// ---------------------------------------------------------------------------

pub fn config_dir(h: &TuiTestHarness) -> PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path().join(".config/agent-of-empires")
    } else {
        h.home_path().join(".agent-of-empires")
    }
}

pub fn db_path(h: &TuiTestHarness) -> PathBuf {
    config_dir(h).join("profiles/default/aoe.db")
}

pub fn sqlite_query(db: &Path, sql: &str) -> String {
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

/// Run SQL that is allowed to fail; returns `(success, combined output)`.
pub fn sqlite_try(db: &Path, sql: &str) -> (bool, String) {
    let output = Command::new("sqlite3")
        .arg("-cmd")
        .arg(".timeout 5000")
        .arg(db)
        .arg(sql)
        .output()
        .expect("failed to run sqlite3");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// The persisted model for the slot holding `native_session_id`.
///
/// Slots are addressed by their native session id rather than by slot number:
/// which pane lands in which slot is the reconciler's business, while the
/// session id is what the test itself seeded.
///
/// Fails with an explicit message while the column does not exist yet, which is
/// the RED state this change starts from.
pub fn slot_model(db: &Path, instance_id: &str, native_session_id: &str) -> String {
    let (ok, out) = sqlite_try(
        db,
        &format!(
            "SELECT model FROM agent_slot \
             WHERE instance_id='{instance_id}' AND native_session_id='{native_session_id}';"
        ),
    );
    assert!(
        ok,
        "reading agent_slot.model for {instance_id}/{native_session_id} failed \
         (the column does not exist yet?): {out}"
    );
    out.trim().to_string()
}

/// Every persisted model for an instance, in slot order. Used where a restart
/// may have rotated the slot's native session id out from under a lookup keyed
/// on it.
pub fn instance_models(db: &Path, instance_id: &str) -> Vec<String> {
    let (ok, out) = sqlite_try(
        db,
        &format!("SELECT model FROM agent_slot WHERE instance_id='{instance_id}' ORDER BY slot;"),
    );
    assert!(
        ok,
        "reading agent_slot.model for {instance_id} failed \
         (the column does not exist yet?): {out}"
    );
    out.lines().map(|line| line.trim().to_string()).collect()
}

/// Poll until the slot's persisted model equals `expected`.
pub fn wait_for_slot_model(db: &Path, instance_id: &str, native_session_id: &str, expected: &str) {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() <= Duration::from_secs(15) {
        last = slot_model(db, instance_id, native_session_id);
        if last == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!(
        "Timed out waiting for agent_slot.model of {instance_id}/{native_session_id} \
         to become {expected:?} (last={last:?})"
    );
}

/// Assert the slot's model stays at `expected` for `hold`, so a test can pin
/// "nothing changed it" rather than merely "it was right once".
pub fn assert_slot_model_stays(
    db: &Path,
    instance_id: &str,
    native_session_id: &str,
    expected: &str,
    hold: Duration,
    why: &str,
) {
    let start = Instant::now();
    while start.elapsed() <= hold {
        let got = slot_model(db, instance_id, native_session_id);
        assert_eq!(
            got, expected,
            "{why}: agent_slot.model of {instance_id}/{native_session_id} changed to {got:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn wait_for_count(h: &TuiTestHarness, db: &Path, sql: &str, expected: &str) {
    let start = Instant::now();
    loop {
        let got = sqlite_query(db, sql);
        if got == expected {
            return;
        }
        if start.elapsed() > Duration::from_secs(15) {
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

// ---------------------------------------------------------------------------
// Transcript fixtures
// ---------------------------------------------------------------------------

/// The project directory names claude could use for `cwd`: the path with every
/// `/` replaced by `-`.
///
/// Both the raw and the canonicalised form are produced. On macOS the harness's
/// temp cwd is reached through the `/var` -> `/private/var` symlink, so an
/// implementation that canonicalises before deriving the directory name and one
/// that does not land in different places. Writing the transcript to both keeps
/// these tests pinned on the observed model rather than on a path-normalisation
/// choice the spec leaves open.
fn project_dir_names(cwd: &str) -> Vec<String> {
    let mut names = vec![cwd.replace('/', "-")];
    if let Ok(canonical) = std::fs::canonicalize(cwd) {
        let canonical = canonical.to_string_lossy().replace('/', "-");
        if !names.contains(&canonical) {
            names.push(canonical);
        }
    }
    names
}

pub fn transcript_files(h: &TuiTestHarness, cwd: &str, session_id: &str) -> Vec<PathBuf> {
    project_dir_names(cwd)
        .into_iter()
        .map(|dir| {
            h.home_path()
                .join(".claude")
                .join("projects")
                .join(dir)
                .join(format!("{session_id}.jsonl"))
        })
        .collect()
}

/// Write `content` as the claude transcript for `(cwd, session_id)` inside the
/// harness's isolated `$HOME`. Never touches the real `~/.claude`.
pub fn write_transcript(
    h: &TuiTestHarness,
    cwd: &str,
    session_id: &str,
    content: &str,
) -> Vec<PathBuf> {
    let files = transcript_files(h, cwd, session_id);
    for file in &files {
        std::fs::create_dir_all(file.parent().expect("transcript parent"))
            .expect("create transcript dir");
        std::fs::write(file, content).expect("write transcript");
    }
    files
}

pub fn remove_transcript(h: &TuiTestHarness, cwd: &str, session_id: &str) {
    for file in transcript_files(h, cwd, session_id) {
        let _ = std::fs::remove_file(file);
    }
}

fn entry(kind: &str, model: Option<&str>, sidechain: bool, text: &str, session_id: &str) -> String {
    let message = match model {
        Some(model) => serde_json::json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [{ "type": "text", "text": text }],
        }),
        None => serde_json::json!({
            "role": "user",
            "content": [{ "type": "text", "text": text }],
        }),
    };
    serde_json::json!({
        "type": kind,
        "isSidechain": sidechain,
        "sessionId": session_id,
        "uuid": format!("{kind}-{text:.16}"),
        "timestamp": "2026-08-11T00:00:00.000Z",
        "message": message,
    })
    .to_string()
}

pub fn assistant_line(session_id: &str, model: &str, text: &str) -> String {
    entry("assistant", Some(model), false, text, session_id)
}

pub fn sidechain_assistant_line(session_id: &str, model: &str, text: &str) -> String {
    entry("assistant", Some(model), true, text, session_id)
}

pub fn user_line(session_id: &str, text: &str) -> String {
    entry("user", None, false, text, session_id)
}

pub fn transcript(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Pad with trailing spaces so a rewritten transcript keeps its original byte
/// length. Used to prove a fingerprint of (mtime, length) alone is what makes
/// reconcile skip a file, not the content happening to be identical.
pub fn pad_to_len(mut content: String, target: usize) -> String {
    assert!(
        content.len() <= target,
        "cannot pad {} bytes down to {}",
        content.len(),
        target
    );
    while content.len() < target {
        content.push(' ');
    }
    content
}

/// Copy `src` to a sibling reference file preserving its mtime, so it can be
/// restored after an in-place rewrite.
pub fn snapshot_mtime(src: &Path) -> PathBuf {
    let reference = src.with_extension("jsonl.mtime-ref");
    let out = Command::new("cp")
        .arg("-p")
        .arg(src)
        .arg(&reference)
        .output()
        .expect("failed to run cp -p");
    assert!(
        out.status.success(),
        "cp -p failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    reference
}

pub fn restore_mtime(reference: &Path, target: &Path) {
    let out = Command::new("touch")
        .arg("-r")
        .arg(reference)
        .arg(target)
        .output()
        .expect("failed to run touch -r");
    assert!(
        out.status.success(),
        "touch -r failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------------
// Instance + slot fixtures
// ---------------------------------------------------------------------------

/// One pane to track: the agent the capture hook reports and the native session
/// id it captures.
pub struct SlotSeed<'a> {
    pub agent: &'a str,
    pub native: &'a str,
}

pub struct Fixture {
    pub instance_id: String,
    pub session_name: String,
    /// The cwd every seeded pane was captured with, and therefore the cwd the
    /// transcript path is derived from.
    pub cwd: String,
    /// tmux pane ids in slot order (slot 0 is the primary agent pane).
    pub panes: Vec<String>,
}

/// Invoke the hidden `aoe __record-pane` capture subcommand exactly as the hook
/// would: hook JSON on stdin, `$TMUX_PANE` / `$AOE_INSTANCE_ID` in the env.
///
/// The capture is pointed at a serverless (but existing) `TMUX_TMPDIR` with
/// `$TMUX` removed, so its pane-ownership check cannot reach any real tmux
/// server -- least of all the developer's.
pub fn run_record_pane(
    h: &TuiTestHarness,
    tmux_pane: &str,
    instance_id: &str,
    agent: &str,
    session_id: &str,
    cwd: &str,
) -> bool {
    let stdin_json = format!(
        "{{\"session_id\":\"{session_id}\",\"cwd\":\"{cwd}\",\"hook_event_name\":\"SessionStart\"}}"
    );
    let no_server = h.home_path().join("no-tmux-server");
    std::fs::create_dir_all(&no_server).expect("create serverless tmpdir");

    let mut child = Command::new(h.binary_path())
        .arg("__record-pane")
        .arg("--agent")
        .arg(agent)
        .env_remove("TMUX")
        .env("TMUX_TMPDIR", &no_server)
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

pub fn instance_id_for(h: &TuiTestHarness, title: &str) -> String {
    let sessions_path = config_dir(h).join("profiles/default/sessions.json");
    let content = std::fs::read_to_string(&sessions_path).expect("read sessions.json");
    let sessions: serde_json::Value = serde_json::from_str(&content).expect("parse sessions.json");
    sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|s| s["title"].as_str() == Some(title))
        .and_then(|s| s["id"].as_str())
        .unwrap_or_else(|| panic!("missing session {title}"))
        .to_string()
}

/// Register and start an instance whose primary tool is `tool`, optionally with
/// `extra_args`. A long-lived stub for `tool` is installed so the primary pane
/// survives long enough to be tracked and respawned.
pub fn add_and_start(
    h: &TuiTestHarness,
    title: &str,
    tool: &str,
    extra_args: Option<&str>,
) -> String {
    h.install_tool_stub(tool);
    let project = h.project_path();
    let extra = extra_args.map(|extra| format!("--extra-args={extra}"));
    let mut args = vec!["add", project.to_str().unwrap(), "-t", title, "-c", tool];
    if let Some(extra) = extra.as_deref() {
        args.push(extra);
    }
    let add = h.run_cli(&args);
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

    instance_id_for(h, title)
}

/// Start an instance, track one pane per seed through the real capture +
/// reconcile chain, and leave the home-view TUI running (which is what drives
/// the reconcile passes these tests rely on).
pub fn seed_instance(
    h: &mut TuiTestHarness,
    title: &str,
    tool: &str,
    extra_args: Option<&str>,
    seeds: &[SlotSeed],
) -> Fixture {
    let instance_id = add_and_start(h, title, tool, extra_args);
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let cwd = h.project_path().to_str().unwrap().to_string();

    h.resize_window(&session_name, 220, 60);

    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    let mut panes = vec![primary.clone()];
    assert!(
        run_record_pane(
            h,
            &primary,
            &instance_id,
            seeds[0].agent,
            seeds[0].native,
            &cwd
        ),
        "capture for the primary pane should succeed"
    );
    for seed in &seeds[1..] {
        let pane = h.split_window_get_pane(&session_name);
        assert!(
            run_record_pane(h, &pane, &instance_id, seed.agent, seed.native, &cwd),
            "capture for a split pane should succeed"
        );
        panes.push(pane);
    }

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.resize_window(h.session_name(), 220, 60);

    let db = db_path(h);
    wait_for_count(
        h,
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
        &seeds.len().to_string(),
    );

    Fixture {
        instance_id,
        session_name,
        cwd,
        panes,
    }
}

// ---------------------------------------------------------------------------
// Restart observation
// ---------------------------------------------------------------------------

/// The command tmux recorded for a pane at respawn time. `respawn-pane -k -t
/// <pane> <command>` stores it in `#{pane_start_command}`, where it survives the
/// stubbed agent binary exiting -- the external, durable evidence of what the
/// pane was restarted with.
pub fn pane_start_command(h: &TuiTestHarness, pane_id: &str) -> String {
    h.tmux_display_message(pane_id, "#{pane_start_command}")
}

pub fn wait_for_pane_start_command_contains(h: &TuiTestHarness, pane_id: &str, needle: &str) {
    let start = Instant::now();
    loop {
        let last = pane_start_command(h, pane_id);
        if last.contains(needle) {
            return;
        }
        if start.elapsed() > Duration::from_secs(15) {
            panic!(
                "Timed out waiting for pane {pane_id} start command to contain {needle:?} \
                 (last={last:?}).\n\n--- Screen ---\n{}",
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// `R`: resume restart of the selected instance's agent panes.
pub fn press_resume_restart(h: &TuiTestHarness) {
    h.send_keys("R");
}

/// `C`: fresh restart (new conversation) of the selected instance's agent panes.
pub fn press_fresh_restart(h: &TuiTestHarness) {
    h.send_keys("C");
}

/// `r`: resume restart without attaching.
pub fn press_resume_restart_no_attach(h: &TuiTestHarness) {
    h.send_keys("r");
}
