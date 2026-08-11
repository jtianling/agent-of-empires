//! RED e2e tests for model injection into every claude pane launch command.
//!
//! Covers `claude-model-continuity` / "每条 claude pane 启动命令都注入观测到的
//! 模型" and the `agent-pane-restart` requirement that the model flag comes out
//! of the one shared command builder rather than a per-keybinding special case.
//!
//! The respawn command is observed the way `multi_pane_restart` observes it:
//! `tmux respawn-pane -k -t <pane> <command>` records the launched command in
//! `#{pane_start_command}`, which survives the stubbed agent exiting. That is
//! the external, durable evidence of what a pane was restarted with.

use std::time::{Duration, Instant};

use serial_test::serial;

use crate::claude_model_support::{
    assistant_line, db_path, instance_models, pane_start_command, press_fresh_restart,
    press_resume_restart, press_resume_restart_no_attach, require_sqlite3, seed_instance,
    transcript, user_line, wait_for_pane_start_command_contains, wait_for_slot_model,
    write_transcript, SlotSeed,
};
use crate::harness::TuiTestHarness;

const OPUS: &str = "claude-opus-5";
const FABLE: &str = "claude-fable-5";

/// Seed a single-claude-pane instance whose transcript reports `model`, and wait
/// until that model is persisted on the slot. Returns the fixture plus the db
/// path, ready for a restart.
fn seed_observed_model(
    h: &mut TuiTestHarness,
    test_title: &str,
    native: &str,
    model: &str,
    extra_args: Option<&str>,
) -> crate::claude_model_support::Fixture {
    let fixture = seed_instance(
        h,
        test_title,
        "claude",
        extra_args,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    write_transcript(
        h,
        &fixture.cwd,
        native,
        &transcript(&[assistant_line(native, model, "observed model")]),
    );
    wait_for_slot_model(&db_path(h), &fixture.instance_id, native, model);
    fixture
}

// ---------------------------------------------------------------------------
// Requirement: 每条 claude pane 启动命令都注入观测到的模型
// ---------------------------------------------------------------------------

/// Scenario: resume 重启带上观测到的模型
#[test]
#[serial]
fn resume_restart_carries_the_observed_model() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_inject_resume");
    let native = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa10";
    let fixture = seed_observed_model(&mut h, "Model Inject Resume", native, FABLE, None);

    press_resume_restart(&h);

    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("--model {FABLE}"));
    let cmd = pane_start_command(&h, &fixture.panes[0]);
    assert!(
        cmd.contains(&format!("--resume {native}")),
        "a resume restart must keep its resume flag alongside the model flag, got: {cmd:?}"
    );
}

/// Scenario: fresh 重启带上观测到的模型
#[test]
#[serial]
fn fresh_restart_carries_the_model_without_a_resume_flag() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_inject_fresh");
    let native = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbb11";
    let fixture = seed_observed_model(&mut h, "Model Inject Fresh", native, OPUS, None);

    press_fresh_restart(&h);

    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("--model {OPUS}"));
    let cmd = pane_start_command(&h, &fixture.panes[0]);
    assert!(
        !cmd.contains("--resume"),
        "a fresh restart must carry no resume flag, got: {cmd:?}"
    );
}

/// Scenario: 非 primary claude pane 也带模型
///
/// The non-primary branch of the shared builder used to return `binary +
/// resume_flag` and nothing else, so a split pane never saw any of the
/// instance's launch configuration. The model must reach it too.
#[test]
#[serial]
fn non_primary_claude_pane_also_carries_the_model() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_inject_non_primary");
    let primary_native = "cccccccc-cccc-4ccc-8ccc-cccccccccc12";
    let second_native = "dddddddd-dddd-4ddd-8ddd-dddddddddd13";
    let fixture = seed_instance(
        &mut h,
        "Model Inject Non Primary",
        "claude",
        None,
        &[
            SlotSeed {
                agent: "claude",
                native: primary_native,
            },
            SlotSeed {
                agent: "claude",
                native: second_native,
            },
        ],
    );
    let db = db_path(&h);

    write_transcript(
        &h,
        &fixture.cwd,
        primary_native,
        &transcript(&[assistant_line(primary_native, OPUS, "primary")]),
    );
    write_transcript(
        &h,
        &fixture.cwd,
        second_native,
        &transcript(&[assistant_line(second_native, FABLE, "second")]),
    );
    wait_for_slot_model(&db, &fixture.instance_id, primary_native, OPUS);
    wait_for_slot_model(&db, &fixture.instance_id, second_native, FABLE);

    press_resume_restart(&h);

    // Each pane gets its OWN model, and the non-primary pane is not skipped.
    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("--model {OPUS}"));
    wait_for_pane_start_command_contains(&h, &fixture.panes[1], &format!("--model {FABLE}"));
}

/// Scenario: 探测值排在 extra_args 之后
///
/// The observed model wins by position, relying on claude taking the last
/// `--model`. AoE must not parse or rewrite `extra_args` to achieve that.
#[test]
#[serial]
fn observed_model_is_appended_after_extra_args() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_inject_after_extra");
    let native = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeee14";
    let fixture = seed_observed_model(
        &mut h,
        "Model After Extra Args",
        native,
        FABLE,
        Some("--model sonnet"),
    );

    press_resume_restart(&h);

    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("--model {FABLE}"));
    let cmd = pane_start_command(&h, &fixture.panes[0]);
    let configured = cmd
        .find("--model sonnet")
        .unwrap_or_else(|| panic!("extra_args must survive verbatim in the command, got: {cmd:?}"));
    let observed = cmd
        .find(&format!("--model {FABLE}"))
        .unwrap_or_else(|| panic!("the observed model must be present, got: {cmd:?}"));
    assert!(
        observed > configured,
        "the observed model must come after extra_args so claude's last-wins \
         rule picks it, got: {cmd:?}"
    );
}

// ---------------------------------------------------------------------------
// Requirement (agent-pane-restart): Agent launch command is reusable
//   Scenario: Model flag comes from the shared builder
// ---------------------------------------------------------------------------

/// `r` (restart without attaching) is a different key on a different code path
/// from `R`, but the same builder. If the model flag were bolted onto the
/// attaching path only, this is where the split would show.
#[test]
#[serial]
fn lowercase_restart_key_gets_the_same_model_flag() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_inject_lowercase_r");
    let native = "ffffffff-ffff-4fff-8fff-ffffffffff15";
    let fixture = seed_observed_model(&mut h, "Model Lowercase Restart", native, FABLE, None);

    press_resume_restart_no_attach(&h);

    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("--model {FABLE}"));
}

// ---------------------------------------------------------------------------
// Requirement: 模型探测仅适用于 claude pane
//   Scenario: 非 claude pane 不受影响 (command half)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn non_claude_pane_command_has_no_model_flag() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_inject_codex_none");
    let native = "019d1af9-a899-7df1-8f7d-a24412600016";
    let fixture = seed_instance(
        &mut h,
        "Model Codex Untouched",
        "codex",
        None,
        &[SlotSeed {
            agent: "codex",
            native,
        }],
    );

    // A claude-shaped transcript sits exactly where a claude pane's would.
    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[assistant_line(native, FABLE, "not this pane's model")]),
    );

    press_resume_restart(&h);

    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("resume {native}"));
    let cmd = pane_start_command(&h, &fixture.panes[0]);
    assert!(
        !cmd.contains("--model"),
        "a codex pane's command must be untouched by model continuity, got: {cmd:?}"
    );
}

// ---------------------------------------------------------------------------
// Requirement: 探测失败不改变启动行为 (command half)
// ---------------------------------------------------------------------------

/// Scenario: 新会话尚无 assistant 消息 -- the command must be exactly what it
/// was before this capability existed.
#[test]
#[serial]
fn pane_without_an_observed_model_gets_no_model_flag() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_inject_absent");
    let native = "99999999-9999-4999-8999-999999999917";
    let fixture = seed_instance(
        &mut h,
        "Model Absent",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );

    // A live conversation that has not been answered yet.
    write_transcript(
        &h,
        &fixture.cwd,
        native,
        &transcript(&[user_line(native, "no reply yet")]),
    );

    press_resume_restart(&h);

    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("--resume {native}"));
    let cmd = pane_start_command(&h, &fixture.panes[0]);
    assert!(
        !cmd.contains("--model"),
        "with no observed model the command must be unchanged, got: {cmd:?}"
    );
}

/// Scenario: 解析错误不影响重启
#[test]
#[serial]
fn malformed_transcript_does_not_break_the_restart() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_inject_malformed");
    let native = "88888888-8888-4888-8888-888888888818";
    let fixture = seed_instance(
        &mut h,
        "Model Malformed Restart",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );

    write_transcript(&h, &fixture.cwd, native, "}}not json{{\n\"dangling\n");

    press_resume_restart(&h);

    // The restart completes normally; a probe failure is never fatal.
    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("--resume {native}"));
    h.assert_screen_not_contains("Error");
}

// ---------------------------------------------------------------------------
// Requirement: 观测到的模型按 slot 持久化并周期刷新
//   Scenario: fresh restart 后模型仍然沿用
// ---------------------------------------------------------------------------

/// The model belongs to the seat, not to the conversation: opening a brand new
/// conversation in the pane must leave it in place.
#[test]
#[serial]
fn fresh_restart_keeps_the_observed_model_on_the_slot() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_slot_survives_fresh");
    let native = "77777777-7777-4777-8777-777777777719";
    let fixture = seed_observed_model(&mut h, "Model Survives Fresh", native, FABLE, None);
    let db = db_path(&h);

    press_fresh_restart(&h);
    wait_for_pane_start_command_contains(&h, &fixture.panes[0], &format!("--model {FABLE}"));

    // The slot keeps the model even though the conversation was replaced. The
    // lookup goes by instance rather than by native session id: a fresh restart
    // is entitled to rotate that id, and the model must outlive it.
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let models = instance_models(&db, &fixture.instance_id);
        assert!(
            models.iter().any(|model| model == FABLE),
            "a fresh restart must not clear the slot's model -- it is a property \
             of the seat, not of the conversation, got {models:?}"
        );
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
