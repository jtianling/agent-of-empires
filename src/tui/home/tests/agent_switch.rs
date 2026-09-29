use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serial_test::serial;

use super::{create_test_env_with_groups, create_test_env_with_sessions, key};
use crate::session::{RestartMode, SandboxInfo, Status};
use crate::tui::app::{Action, PostRestart};

#[test]
#[serial]
fn shift_a_opens_the_dialog_to_attach_and_a_to_stay() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);

    env.view
        .handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    let (_, post) = env.view.switch_agent_dialog.as_ref().expect("dialog opens");
    assert_eq!(*post, PostRestart::Attach);

    env.view.handle_key(key(KeyCode::Esc));
    assert!(env.view.switch_agent_dialog.is_none());

    env.view.handle_key(key(KeyCode::Char('a')));
    let (_, post) = env.view.switch_agent_dialog.as_ref().expect("dialog opens");
    assert_eq!(*post, PostRestart::StayOnHome);
}

#[test]
#[serial]
fn unsupported_agent_explains_instead_of_opening() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let id = env.view.instances[0].id.clone();
    env.view.mutate_instance(&id, |inst| {
        inst.tool = "opencode".to_string();
        inst.sync_primary_pane_from_legacy();
    });

    env.view.handle_key(key(KeyCode::Char('a')));

    assert!(env.view.switch_agent_dialog.is_none());
    assert!(env.view.info_dialog.is_some());
}

#[test]
#[serial]
fn sandboxed_session_explains_instead_of_opening() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let id = env.view.instances[0].id.clone();
    env.view.mutate_instance(&id, |inst| {
        inst.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: "ubuntu:latest".to_string(),
            container_name: "test-container".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
    });

    env.view.handle_key(key(KeyCode::Char('a')));

    assert!(env.view.switch_agent_dialog.is_none());
    assert!(env.view.info_dialog.is_some());
}

#[test]
#[serial]
fn busy_session_and_group_row_are_ignored() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let id = env.view.instances[0].id.clone();
    env.view
        .mutate_instance(&id, |inst| inst.status = Status::Deleting);
    env.view.handle_key(key(KeyCode::Char('a')));
    assert!(env.view.switch_agent_dialog.is_none());

    env.view.mutate_instance(&id, |inst| {
        inst.status = Status::Restarting;
        inst.restart_in_flight = true;
    });
    env.view
        .handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert!(env.view.switch_agent_dialog.is_none());
    assert!(env.view.info_dialog.is_none());

    let mut env = create_test_env_with_groups();
    env.view.selected_session = None;
    env.view.selected_group = Some("work".to_string());
    env.view.handle_key(key(KeyCode::Char('a')));
    assert!(env.view.switch_agent_dialog.is_none());
    assert!(env.view.info_dialog.is_none());
}

#[test]
#[serial]
fn confirming_from_shift_a_restarts_and_attaches() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let id = env.view.instances[0].id.clone();

    env.view
        .handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    let action = env.view.handle_key(key(KeyCode::Enter));

    assert_eq!(
        action,
        Some(Action::RespawnAgentPane(
            id.clone(),
            RestartMode::Fresh,
            PostRestart::Attach
        ))
    );
    assert_eq!(env.view.get_instance(&id).unwrap().tool, "codex");
}

#[test]
#[serial]
fn confirming_commits_the_switch_then_restarts_fresh() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let id = env.view.instances[0].id.clone();
    let profile = env.view.storage.profile().to_string();
    let store = crate::db::Store::open_with_schema(&profile).unwrap();
    store
        .upsert_agent_slot(&id, 0, "claude", "sess-0", "/tmp/0", "%1", "key-0", 1)
        .unwrap();

    env.view.handle_key(key(KeyCode::Char('a')));
    let action = env.view.handle_key(key(KeyCode::Enter));

    assert_eq!(
        action,
        Some(Action::RespawnAgentPane(
            id.clone(),
            RestartMode::Fresh,
            PostRestart::StayOnHome
        ))
    );
    let inst = env.view.get_instance(&id).unwrap();
    assert_eq!(inst.tool, "codex");
    assert_eq!(inst.command, "codex");
    let saved = env.view.storage.load().unwrap();
    assert_eq!(saved[0].tool, "codex");
    let slots = store.read_slots_for_instance(&id).unwrap();
    assert_eq!(slots[0].agent, "codex");
    assert_eq!(slots[0].xats_identity_key, "key-0");
    assert!(slots[0].native_session_id.is_empty());
}
