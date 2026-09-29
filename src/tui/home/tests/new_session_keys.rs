use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serial_test::serial;

use super::{create_test_env_with_groups, key};
use crate::session::Item;
use crate::tui::app::{Action, PostRestart};

fn select_group(env: &mut super::TestEnv, group: &str) {
    let idx = env
        .view
        .flat_items
        .iter()
        .position(|item| matches!(item, Item::Group { path, .. } if path == group))
        .expect("group exists");
    env.view.cursor = idx;
    env.view.update_selected();
}

fn select_session_in(env: &mut super::TestEnv, group: &str) {
    let idx = env
        .view
        .flat_items
        .iter()
        .position(|item| {
            matches!(item, Item::Session { id, .. }
                if env.view.get_instance(id).is_some_and(|inst| inst.group_path == group))
        })
        .expect("session exists");
    env.view.cursor = idx;
    env.view.update_selected();
}

#[test]
#[serial]
fn n_and_shift_n_both_use_the_group_directory() {
    let mut env = create_test_env_with_groups();
    env.view
        .group_tree
        .set_default_directory("work", "/tmp/work-dir");

    select_group(&mut env, "work");
    env.view.handle_key(key(KeyCode::Char('n')));
    let dialog = env.view.new_dialog.take().expect("dialog opens");
    assert_eq!(dialog.group_value(), "work");
    assert_eq!(dialog.path_value(), "/tmp/work-dir");
    assert_eq!(env.view.new_session_post, PostRestart::StayOnHome);

    select_session_in(&mut env, "work");
    env.view
        .handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
    let dialog = env.view.new_dialog.take().expect("dialog opens");
    assert_eq!(dialog.group_value(), "work");
    assert_eq!(
        dialog.path_value(),
        "/tmp/work-dir",
        "the group directory wins over the selected session's"
    );
    assert_eq!(env.view.new_session_post, PostRestart::Attach);
}

#[test]
#[serial]
fn a_session_in_a_group_without_a_directory_supplies_its_path() {
    let mut env = create_test_env_with_groups();
    select_session_in(&mut env, "personal");

    env.view.handle_key(key(KeyCode::Char('n')));

    let dialog = env.view.new_dialog.as_ref().expect("dialog opens");
    assert_eq!(dialog.group_value(), "personal");
    assert_eq!(dialog.path_value(), "/tmp/personal");
}

#[test]
#[serial]
fn n_creates_without_attaching_and_shift_n_attaches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    for (code, modifiers, started) in [
        (KeyCode::Char('n'), KeyModifiers::NONE, true),
        (KeyCode::Char('N'), KeyModifiers::SHIFT, false),
    ] {
        crate::tmux::isolate_tmux_socket();
        let mut env = create_test_env_with_groups();
        env.view.handle_key(KeyEvent::new(code, modifiers));
        env.view.new_dialog.as_mut().unwrap().set_path(path.clone());

        let action = env.view.handle_key(key(KeyCode::Enter));

        match action {
            Some(Action::StartSession(_)) => assert!(started, "N must attach"),
            Some(Action::AttachSession(_)) => assert!(!started, "n must not attach"),
            other => panic!("unexpected action {other:?}"),
        }
    }
}
