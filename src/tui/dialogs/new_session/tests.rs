use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn dialog() -> NewSessionDialog {
    NewSessionDialog::new_with_tools(
        vec!["claude", "codex", "shell"],
        std::env::temp_dir().to_string_lossy().to_string(),
    )
}

#[test]
fn field_order_places_session_metadata_before_primary_and_secondary() {
    let mut dialog = dialog();
    let collapsed = dialog.field_layout();
    assert!(collapsed.title < collapsed.group);
    assert!(collapsed.group < collapsed.tool);
    assert!(collapsed.tool < collapsed.path);
    assert!(collapsed.path < collapsed.worktree);
    assert!(collapsed.worktree < collapsed.right_pane);
    assert_eq!(collapsed.right_pane_path, ABSENT);

    dialog.set_right_pane_selection(1);
    let expanded = dialog.field_layout();
    assert!(expanded.right_pane < expanded.right_pane_path);
    assert!(expanded.right_pane_path < expanded.right_pane_worktree);
}

#[test]
fn collapsing_secondary_preserves_its_independent_draft() {
    let mut dialog = dialog();
    dialog.set_right_pane_selection(2);
    let secondary = dialog.secondary.as_mut().unwrap();
    secondary.path.set_value("/tmp/secondary");
    secondary.yolo_mode = true;
    secondary.cross_agent_team = false;
    secondary.worktree_branch = tui_input::Input::new("secondary-branch".to_string());

    dialog.set_right_pane_selection(0);
    assert!(dialog.secondary.is_none());
    dialog.set_right_pane_selection(2);
    let restored = dialog.secondary.as_ref().unwrap();
    assert_eq!(restored.path.value(), "/tmp/secondary");
    assert!(restored.yolo_mode);
    assert!(!restored.cross_agent_team);
    assert_eq!(restored.worktree_branch.value(), "secondary-branch");
}

#[test]
fn pane_flags_are_independent() {
    let mut dialog = dialog();
    dialog.set_right_pane_selection(1);
    dialog.primary.yolo_mode = false;
    dialog.secondary.as_mut().unwrap().yolo_mode = true;
    dialog.focused_field = dialog.field_layout().cross_agent_team;
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.primary.cross_agent_team);
    assert!(dialog.secondary.as_ref().unwrap().yolo_mode);
    assert!(!dialog.secondary.as_ref().unwrap().cross_agent_team);

    dialog.focused_field = dialog.field_layout().right_pane_cross_agent_team;
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.primary.cross_agent_team);
    assert!(dialog.secondary.as_ref().unwrap().cross_agent_team);
}

#[test]
fn shell_hides_only_its_flags_and_keeps_worktree() {
    let mut dialog = dialog();
    dialog.primary.tool_index = 2;
    dialog.primary.worktree_branch = tui_input::Input::new("shell-worktree".to_string());
    dialog.normalize_pane_for_tool(PaneTarget::Primary);
    let layout = dialog.field_layout();
    assert_eq!(layout.yolo, ABSENT);
    assert_eq!(layout.cross_agent_team, ABSENT);
    assert_ne!(layout.worktree, ABSENT);
    assert_eq!(dialog.primary.worktree_branch.value(), "shell-worktree");
}

#[test]
fn switching_from_shell_to_always_yolo_restores_required_mode() {
    let mut dialog = NewSessionDialog::new_with_tools(
        vec!["shell", "pi"],
        std::env::temp_dir().to_string_lossy().to_string(),
    );
    dialog.primary.tool_index = 0;
    dialog.primary.yolo_mode = false;
    dialog.normalize_pane_for_tool(PaneTarget::Primary);

    dialog.primary.tool_index = 1;
    dialog.normalize_pane_for_tool(PaneTarget::Primary);

    assert!(dialog.primary.yolo_mode);
    let DialogResult::Submit(data) = dialog.build_submit_result() else {
        panic!("expected submit");
    };
    assert!(data.primary.yolo_mode);
}

#[test]
fn submitted_panes_keep_separate_paths_flags_and_worktrees() {
    let temp = tempfile::tempdir().unwrap();
    let mut dialog = dialog();
    dialog.primary.path.set_value(temp.path().to_string_lossy());
    dialog.primary.yolo_mode = true;
    dialog.primary.cross_agent_team = false;
    dialog.primary.worktree_branch = tui_input::Input::new("left".to_string());
    dialog.set_right_pane_selection(2);
    let secondary = dialog.secondary.as_mut().unwrap();
    secondary.path.set_value(temp.path().to_string_lossy());
    secondary.yolo_mode = false;
    secondary.cross_agent_team = true;
    secondary.worktree_branch = tui_input::Input::new("right".to_string());

    let DialogResult::Submit(data) = dialog.build_submit_result() else {
        panic!("expected submit");
    };
    assert_eq!(data.primary.worktree.branch.as_deref(), Some("left"));
    assert!(data.primary.yolo_mode);
    assert!(!data.primary.cross_agent_team);
    let right = data.secondary.unwrap();
    assert_eq!(right.worktree.branch.as_deref(), Some("right"));
    assert!(!right.yolo_mode);
    assert!(right.cross_agent_team);
}

#[test]
fn new_session_state_has_no_sandbox_entry() {
    let dialog = dialog();
    assert!(FIELD_HELP.iter().all(|entry| entry.name != "Sandbox"));
    assert_eq!(
        FIELD_HELP
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        vec![
            "Title",
            "Group",
            "Tool",
            "Path",
            "YOLO Mode",
            "Cross Agent Team",
            "Worktree",
            "Right Pane",
            "Right Pane Path",
        ]
    );
    assert_eq!(
        dialog.field_layout().count,
        dialog.field_layout().right_pane + 1
    );
}

#[test]
fn pane_path_ghosts_are_computed_independently() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("primary-project")).unwrap();
    std::fs::create_dir(temp.path().join("secondary-project")).unwrap();
    let mut dialog = dialog();
    dialog.set_right_pane_selection(1);
    dialog
        .primary
        .path
        .set_value(format!("{}/primary-p", temp.path().display()));
    dialog.primary.path.recompute_ghost();
    dialog
        .secondary
        .as_mut()
        .unwrap()
        .path
        .set_value(format!("{}/secondary-p", temp.path().display()));
    dialog.secondary.as_mut().unwrap().path.recompute_ghost();

    assert_eq!(dialog.primary.path.ghost_text(), Some("roject/"));
    assert_eq!(
        dialog.secondary.as_ref().unwrap().path.ghost_text(),
        Some("roject/")
    );
}

#[test]
fn worktree_overlay_tracks_the_focused_pane() {
    let mut dialog = dialog();
    dialog.set_right_pane_selection(1);
    dialog.primary.worktree_branch = tui_input::Input::new("left".to_string());
    dialog.secondary.as_mut().unwrap().worktree_branch = tui_input::Input::new("right".to_string());

    dialog.focused_field = dialog.field_layout().worktree;
    dialog.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(dialog.worktree_config_mode);
    assert_eq!(dialog.worktree_config_target, PaneTarget::Primary);

    dialog.worktree_config_mode = false;
    dialog.focused_field = dialog.field_layout().right_pane_worktree;
    dialog.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(dialog.worktree_config_mode);
    assert_eq!(dialog.worktree_config_target, PaneTarget::Secondary);
}

#[test]
fn render_orders_primary_before_divided_secondary_and_omits_sandbox() {
    let mut dialog = dialog();
    dialog.set_right_pane_selection(1);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            dialog.render(frame, area, &crate::tui::styles::Theme::default());
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let screen = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    let tool = screen.find("Tool:").unwrap();
    let path = screen.find("Path:").unwrap();
    let right = screen.find("Right Pane Agent:").unwrap();
    let right_path = screen.find("Right Pane Path:").unwrap();
    assert!(tool < path && path < right && right < right_path);
    assert!(!screen.contains("Sandbox:"));
}

#[test]
fn escape_cancels_and_enter_submits_session_metadata() {
    let mut cancelled = dialog();
    assert!(matches!(
        cancelled.handle_key(key(KeyCode::Esc)),
        DialogResult::Cancel
    ));

    let mut submitted = dialog();
    submitted.title = tui_input::Input::new("custom-title".to_string());
    submitted.group = tui_input::Input::new("custom-group".to_string());
    let DialogResult::Submit(data) = submitted.handle_key(key(KeyCode::Enter)) else {
        panic!("expected submit");
    };
    assert_eq!(data.title, "custom-title");
    assert_eq!(data.group, "custom-group");
    assert_eq!(data.primary.tool, "claude");
    assert!(data.secondary.is_none());
}

#[test]
fn tab_visits_each_visible_field_exactly_once() {
    for expanded in [false, true] {
        let mut dialog = dialog();
        if expanded {
            dialog.set_right_pane_selection(2);
        }
        let count = dialog.field_layout().count;
        let mut visited = Vec::new();
        for _ in 0..count {
            visited.push(dialog.focused_field);
            dialog.handle_key(key(KeyCode::Tab));
        }
        visited.sort_unstable();
        visited.dedup();
        assert_eq!(visited, (0..count).collect::<Vec<_>>());
        assert_eq!(dialog.focused_field, 0);
    }
}

#[test]
fn text_input_routes_to_session_and_each_pane() {
    let mut dialog = dialog();
    dialog.focused_field = dialog.field_layout().title;
    dialog.handle_key(key(KeyCode::Char('t')));
    dialog.focused_field = dialog.field_layout().group;
    dialog.handle_key(key(KeyCode::Char('g')));
    dialog.focused_field = dialog.field_layout().path;
    dialog.handle_key(key(KeyCode::Char('p')));

    dialog.set_right_pane_selection(1);
    dialog.focused_field = dialog.field_layout().right_pane_path;
    dialog.handle_key(key(KeyCode::Char('r')));

    assert_eq!(dialog.title.value(), "t");
    assert_eq!(dialog.group.value(), "g");
    assert!(dialog.primary.path.value().ends_with('p'));
    assert_eq!(dialog.secondary.as_ref().unwrap().path.value(), "r");
}

#[test]
fn primary_and_secondary_tool_selectors_are_independent() {
    let mut dialog = dialog();
    dialog.focused_field = dialog.field_layout().tool;
    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.primary.tool_index, 1);

    dialog.set_right_pane_selection(1);
    dialog.focused_field = dialog.field_layout().right_pane;
    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.primary.tool_index, 1);
    assert_eq!(dialog.secondary.as_ref().unwrap().tool_index, 1);
}

#[test]
fn profile_defaults_initialize_each_supported_pane() {
    let mut config = Config::default();
    config.session.yolo_mode_default = true;
    config.session.cross_agent_team_default = true;
    let mut dialog = NewSessionDialog::new_with_config(
        vec!["claude", "codex"],
        std::env::temp_dir().to_string_lossy().to_string(),
        config,
    );

    dialog.set_right_pane_selection(2);
    assert!(dialog.primary.yolo_mode);
    assert!(dialog.primary.cross_agent_team);
    let secondary = dialog.secondary.as_ref().unwrap();
    assert!(secondary.yolo_mode);
    assert!(secondary.cross_agent_team);
}

#[test]
fn opencode_panes_expose_independent_cross_agent_team_controls() {
    let mut config = Config::default();
    config.session.cross_agent_team_default = true;
    let mut dialog = NewSessionDialog::new_with_config(
        vec!["opencode", "shell"],
        std::env::temp_dir().to_string_lossy().to_string(),
        config,
    );
    dialog.set_right_pane_selection(1);

    let layout = dialog.field_layout();
    assert_ne!(layout.cross_agent_team, ABSENT);
    assert_ne!(layout.right_pane_cross_agent_team, ABSENT);
    assert!(dialog.primary.cross_agent_team);
    assert!(dialog.secondary.as_ref().unwrap().cross_agent_team);

    dialog.focused_field = layout.right_pane_cross_agent_team;
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.primary.cross_agent_team);
    assert!(!dialog.secondary.as_ref().unwrap().cross_agent_team);
}

#[test]
fn both_missing_pane_directories_share_one_confirmation() {
    let temp = tempfile::tempdir().unwrap();
    let primary = temp.path().join("primary");
    let secondary = temp.path().join("secondary");
    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], primary.to_string_lossy().to_string());
    dialog.set_right_pane_selection(1);
    dialog
        .secondary
        .as_mut()
        .unwrap()
        .path
        .set_value(secondary.to_string_lossy());

    assert!(matches!(
        dialog.handle_key(key(KeyCode::Enter)),
        DialogResult::Continue
    ));
    assert_eq!(
        dialog.confirm_create_dirs.as_ref().unwrap().dirs,
        vec![
            primary.to_string_lossy().to_string(),
            secondary.to_string_lossy().to_string()
        ]
    );
    assert!(matches!(
        dialog.handle_key(key(KeyCode::Char('n'))),
        DialogResult::Continue
    ));
    assert!(!primary.exists());
    assert!(!secondary.exists());
}

#[test]
fn confirming_both_missing_directories_creates_then_submits() {
    let temp = tempfile::tempdir().unwrap();
    let primary = temp.path().join("primary");
    let secondary = temp.path().join("secondary");
    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], primary.to_string_lossy().to_string());
    dialog.set_right_pane_selection(1);
    dialog
        .secondary
        .as_mut()
        .unwrap()
        .path
        .set_value(secondary.to_string_lossy());

    dialog.handle_key(key(KeyCode::Enter));
    assert!(matches!(
        dialog.handle_key(key(KeyCode::Char('y'))),
        DialogResult::Submit(_)
    ));
    assert!(primary.is_dir());
    assert!(secondary.is_dir());
}

#[test]
fn secondary_regular_file_is_rejected_before_submit() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("not-a-directory");
    std::fs::write(&file, "file").unwrap();
    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], temp.path().to_string_lossy().to_string());
    dialog.set_right_pane_selection(1);
    dialog
        .secondary
        .as_mut()
        .unwrap()
        .path
        .set_value(file.to_string_lossy());

    assert!(matches!(
        dialog.handle_key(key(KeyCode::Enter)),
        DialogResult::Continue
    ));
    assert_eq!(dialog.focused_field, dialog.field_layout().right_pane_path);
    assert!(dialog
        .error_message
        .as_deref()
        .unwrap()
        .contains("Not a directory"));
}

#[test]
fn tracked_directory_creation_records_only_owned_levels() {
    let temp = tempfile::tempdir().unwrap();
    let nested = temp.path().join("a").join("b").join("c");
    let mut owned = Vec::new();
    create_dir_tracked(&nested.to_string_lossy(), &mut owned).unwrap();

    assert_eq!(owned.len(), 3);
    assert_eq!(owned.last(), Some(&nested));
    assert!(!owned.iter().any(|path| path == temp.path()));
}

#[test]
fn tracked_directory_creation_refuses_regular_files() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file");
    std::fs::write(&file, "file").unwrap();
    let mut owned = Vec::new();

    let error = create_dir_tracked(&file.to_string_lossy(), &mut owned).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(owned.is_empty());
}
