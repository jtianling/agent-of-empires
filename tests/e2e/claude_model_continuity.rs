//! RED e2e tests for the `claude-model-continuity` capability: detecting the
//! model a claude pane is actually running, and persisting it per slot.
//!
//! `/model` is pure session state inside claude -- it is written nowhere except
//! the transcript's assistant entries. AoE therefore reads the tail of
//! `<home>/.claude/projects/<project-dir>/<native_session_id>.jsonl` and stores
//! what it finds on `agent_slot.model`.
//!
//! Everything here is observed from outside the binary: transcripts are seeded
//! as real files inside the harness's isolated `$HOME`, reconcile is driven by
//! the real home-view status poller, and the result is read back from the
//! on-disk `aoe.db` via the `sqlite3` CLI.
//!
//! These are RED until the change lands: `agent_slot` has no `model` column, so
//! every read of it fails outright.
//!
//! Command injection lives in `claude_model_injection`, and the store column
//! itself in `claude_model_schema`.

use std::time::Duration;

use serial_test::serial;

use crate::claude_model_support::{
    assert_slot_model_stays, assistant_line, db_path, pad_to_len, remove_transcript,
    require_sqlite3, restore_mtime, seed_instance, sidechain_assistant_line, slot_model,
    snapshot_mtime, transcript, user_line, wait_for_slot_model, write_transcript, SlotSeed,
    SYNTHETIC_MODEL,
};
use crate::harness::TuiTestHarness;

const OPUS: &str = "claude-opus-5";
const FABLE: &str = "claude-fable-5";

// ---------------------------------------------------------------------------
// Requirement: 模型探测仅适用于 claude pane
//   Scenario: 非 claude pane 不受影响
//   Scenario: claude pane 参与探测 (covered by the detection tests below)
// ---------------------------------------------------------------------------

/// A codex pane must not be probed even when a claude-shaped transcript happens
/// to sit at the path a claude pane with the same cwd and session id would use.
/// The agent recorded on the slot, not the presence of a file, decides.
#[test]
#[serial]
fn non_claude_pane_is_never_probed() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_non_claude_pane");
    let native = "019d1af9-a899-7df1-8f7d-a244126e5ded";
    let fixture = seed_instance(
        &mut h,
        "Model Non Claude",
        "codex",
        None,
        &[SlotSeed {
            agent: "codex",
            native,
        }],
    );
    let db = db_path(&h);

    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[assistant_line(native, FABLE, "hello")]),
    );

    assert_slot_model_stays(
        &db,
        &fixture.instance_id,
        native,
        "",
        Duration::from_secs(6),
        "a non-claude pane must never be probed for a model",
    );
}

// ---------------------------------------------------------------------------
// Requirement: 当前模型从 claude transcript 尾部探测
// ---------------------------------------------------------------------------

/// Scenario: 取最后一条有效 assistant 条目的模型
#[test]
#[serial]
fn model_is_the_last_valid_assistant_entry() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_last_assistant");
    let native = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa0";
    let fixture = seed_instance(
        &mut h,
        "Model Last Assistant",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );

    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[
            user_line(native, "switch me"),
            assistant_line(native, OPUS, "on opus"),
            user_line(native, "and again"),
            assistant_line(native, FABLE, "on fable"),
        ]),
    );

    wait_for_slot_model(&db_path(&h), &fixture.instance_id, native, FABLE);
}

/// Scenario: 子代理条目被跳过
#[test]
#[serial]
fn sidechain_assistant_entries_are_skipped() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_skip_sidechain");
    let native = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1";
    let fixture = seed_instance(
        &mut h,
        "Model Skip Sidechain",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    let db = db_path(&h);

    // The last assistant entry belongs to a subagent; the pane's own model is
    // the one before it.
    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[
            assistant_line(native, FABLE, "main agent"),
            sidechain_assistant_line(native, OPUS, "subagent"),
        ]),
    );

    wait_for_slot_model(&db, &fixture.instance_id, native, FABLE);
    assert_ne!(
        slot_model(&db, &fixture.instance_id, native),
        OPUS,
        "a sidechain entry must never supply the pane's model"
    );
}

/// Scenario: 合成条目被跳过
#[test]
#[serial]
fn synthetic_model_entries_are_skipped() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_skip_synthetic");
    let native = "cccccccc-cccc-4ccc-8ccc-ccccccccccc2";
    let fixture = seed_instance(
        &mut h,
        "Model Skip Synthetic",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    let db = db_path(&h);

    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[
            assistant_line(native, OPUS, "real answer"),
            assistant_line(native, SYNTHETIC_MODEL, "interrupted by user"),
        ]),
    );

    wait_for_slot_model(&db, &fixture.instance_id, native, OPUS);
    assert_ne!(
        slot_model(&db, &fixture.instance_id, native),
        SYNTHETIC_MODEL,
        "the synthetic placeholder must never be stored as a model"
    );
}

/// Scenario: 超长行不阻碍探测
///
/// Real transcripts carry single lines well past 256 KiB, which is exactly the
/// size at which a too-small tail window slices through the middle of the entry
/// that holds the answer.
#[test]
#[serial]
fn oversized_transcript_line_still_yields_the_model() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_oversized_line");
    let native = "dddddddd-dddd-4ddd-8ddd-ddddddddddd3";
    let fixture = seed_instance(
        &mut h,
        "Model Oversized Line",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );

    let huge = "x".repeat(300 * 1024);
    let line = assistant_line(native, FABLE, &huge);
    assert!(
        line.len() > 256 * 1024,
        "precondition: the entry must exceed 256 KiB, got {} bytes",
        line.len()
    );
    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[assistant_line(native, OPUS, "small"), line]),
    );

    wait_for_slot_model(&db_path(&h), &fixture.instance_id, native, FABLE);
}

/// Scenario: 变体标记不被补回
///
/// A `[1m]` session records itself as the plain model id. Detection must store
/// exactly what the transcript says and invent nothing.
#[test]
#[serial]
fn context_variant_marker_is_not_restored() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_no_variant");
    let native = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeee4";
    let fixture = seed_instance(
        &mut h,
        "Model No Variant",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    let db = db_path(&h);

    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[assistant_line(native, OPUS, "started as opus-5[1m]")]),
    );

    wait_for_slot_model(&db, &fixture.instance_id, native, OPUS);
    let stored = slot_model(&db, &fixture.instance_id, native);
    assert!(
        !stored.contains("[1m]"),
        "the context-window variant marker must not be inferred, got {stored:?}"
    );
}

// ---------------------------------------------------------------------------
// Requirement: 观测到的模型按 slot 持久化并周期刷新
// ---------------------------------------------------------------------------

/// Scenario: 同一 instance 的两个 pane 保存不同模型
#[test]
#[serial]
fn two_panes_of_one_instance_keep_distinct_models() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_per_slot");
    let first = "11111111-1111-4111-8111-111111111110";
    let second = "22222222-2222-4222-8222-222222222221";
    let fixture = seed_instance(
        &mut h,
        "Model Per Slot",
        "claude",
        None,
        &[
            SlotSeed {
                agent: "claude",
                native: first,
            },
            SlotSeed {
                agent: "claude",
                native: second,
            },
        ],
    );
    let db = db_path(&h);

    write_transcript(
        &h,
        &fixture.cwd,
        first,
        &transcript(&[assistant_line(first, OPUS, "pane one")]),
    );
    write_transcript(
        &h,
        &fixture.cwd,
        second,
        &transcript(&[assistant_line(second, FABLE, "pane two")]),
    );

    wait_for_slot_model(&db, &fixture.instance_id, first, OPUS);
    wait_for_slot_model(&db, &fixture.instance_id, second, FABLE);
}

/// Scenario: 模型跨进程重启存活
/// (also `agent-session-store` / Model survives process restart)
#[test]
#[serial]
fn model_survives_process_restart() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_survives_restart");
    let native = "33333333-3333-4333-8333-333333333332";
    let fixture = seed_instance(
        &mut h,
        "Model Survives Restart",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    let db = db_path(&h);

    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[assistant_line(native, FABLE, "observed")]),
    );
    wait_for_slot_model(&db, &fixture.instance_id, native, FABLE);

    // "Close and reopen AoE": a fresh process against the same profile.
    let reopen = h.run_cli(&["list"]);
    assert!(
        reopen.status.success(),
        "reopening aoe failed: {}",
        String::from_utf8_lossy(&reopen.stderr)
    );

    assert_eq!(
        slot_model(&db, &fixture.instance_id, native),
        FABLE,
        "the persisted model must be read back unchanged after a process restart"
    );
}

/// Scenario: 未变化的 transcript 不被重读
///
/// "Not re-read" is observable from outside: rewrite the file in place with a
/// different model while keeping its length and mtime, and a fingerprint-gated
/// reconcile will keep reporting the old value. Without the fingerprint the
/// rewrite would be picked up within a tick.
#[test]
#[serial]
fn unchanged_transcript_is_not_reread() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_fingerprint_skip");
    let native = "44444444-4444-4444-8444-444444444443";
    let fixture = seed_instance(
        &mut h,
        "Model Fingerprint Skip",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    let db = db_path(&h);

    let original = transcript(&[assistant_line(native, OPUS, "first observation")]);
    let files = write_transcript(&h, &fixture.cwd, native, &original);
    wait_for_slot_model(&db, &fixture.instance_id, native, OPUS);

    // Same byte length, same mtime, different model.
    let rewritten = pad_to_len(
        transcript(&[assistant_line(native, FABLE, "silently swapped")]),
        original.len(),
    );
    assert_eq!(
        rewritten.len(),
        original.len(),
        "precondition: the rewrite must not change the file length"
    );
    for file in &files {
        let reference = snapshot_mtime(file);
        std::fs::write(file, &rewritten).expect("rewrite transcript in place");
        restore_mtime(&reference, file);
        let _ = std::fs::remove_file(&reference);
    }

    assert_slot_model_stays(
        &db,
        &fixture.instance_id,
        native,
        OPUS,
        Duration::from_secs(6),
        "a transcript whose mtime and length are unchanged must not be re-read",
    );
}

// ---------------------------------------------------------------------------
// Requirement: 探测失败不改变启动行为 (detection half)
// ---------------------------------------------------------------------------

/// Scenario: 新会话尚无 assistant 消息
#[test]
#[serial]
fn transcript_without_assistant_entries_leaves_model_empty() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_no_assistant_yet");
    let native = "55555555-5555-4555-8555-555555555554";
    let fixture = seed_instance(
        &mut h,
        "Model No Assistant Yet",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );

    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[user_line(native, "first message, no reply yet")]),
    );

    assert_slot_model_stays(
        &db_path(&h),
        &fixture.instance_id,
        native,
        "",
        Duration::from_secs(6),
        "a transcript with no assistant entry yields no model",
    );
}

/// Scenario: transcript 缺失时保留旧值
#[test]
#[serial]
fn missing_transcript_keeps_the_last_known_model() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_missing_transcript");
    let native = "66666666-6666-4666-8666-666666666665";
    let fixture = seed_instance(
        &mut h,
        "Model Missing Transcript",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    let db = db_path(&h);

    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[assistant_line(native, OPUS, "before deletion")]),
    );
    wait_for_slot_model(&db, &fixture.instance_id, native, OPUS);

    remove_transcript(&h, &fixture.cwd, native);

    assert_slot_model_stays(
        &db,
        &fixture.instance_id,
        native,
        OPUS,
        Duration::from_secs(6),
        "a failed probe must preserve the last known model, never clear it",
    );
}

/// A transcript whose tail is not valid JSON yields no observation, and must
/// not clear a model that was already observed.
#[test]
#[serial]
fn malformed_transcript_tail_keeps_the_last_known_model() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_malformed_tail");
    let native = "77777777-7777-4777-8777-777777777776";
    let fixture = seed_instance(
        &mut h,
        "Model Malformed Tail",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    let db = db_path(&h);

    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[assistant_line(native, FABLE, "valid entry")]),
    );
    wait_for_slot_model(&db, &fixture.instance_id, native, FABLE);

    write_transcript(&h, &fixture.cwd, native, "{not json at all\n{\"type\":\n");

    assert_slot_model_stays(
        &db,
        &fixture.instance_id,
        native,
        FABLE,
        Duration::from_secs(6),
        "a parse failure must preserve the last known model",
    );
}
