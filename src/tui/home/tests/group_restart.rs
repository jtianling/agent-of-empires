use super::*;
use crate::session::{RestartMode, Status};

#[test]
#[serial]
fn lowercase_group_restart_selects_all_members_without_attaching() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_groups();
    env.view.selected_session = None;
    env.view.selected_group = Some("work".to_string());
    let id = env
        .view
        .instances
        .iter()
        .find(|inst| inst.group_path == "work")
        .unwrap()
        .id
        .clone();

    for (code, mode) in [('r', RestartMode::Resume), ('c', RestartMode::Fresh)] {
        assert_eq!(
            env.view.handle_key(key(KeyCode::Char(code))),
            Some(Action::RestartGroup(vec![id.clone()], mode))
        );
    }
    for code in ['R', 'C'] {
        assert_eq!(env.view.handle_key(key(KeyCode::Char(code))), None);
    }
    env.view
        .mutate_instance(&id, |inst| inst.status = Status::Deleting);
    assert_eq!(env.view.handle_key(key(KeyCode::Char('r'))), None);
    env.view.mutate_instance(&id, |inst| {
        inst.status = Status::Restarting;
        inst.restart_in_flight = true;
    });
    assert_eq!(env.view.handle_key(key(KeyCode::Char('c'))), None);
}
