use super::*;
use crate::session::{merge_configs, Config, ProfileConfig, SessionConfigOverride};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;

const TEST_PATH: &str = ".";

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn alt_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::ALT)
}

fn shift_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

fn single_tool_dialog() -> NewSessionDialog {
    NewSessionDialog::new_with_tools(vec!["claude"], TEST_PATH.to_string())
}

fn multi_tool_dialog() -> NewSessionDialog {
    NewSessionDialog::new_with_tools(vec!["claude", "opencode"], TEST_PATH.to_string())
}

/// Field indices for the dialog's current conditional layout. Tests name the
/// field they mean through this rather than hardcoding an index, so adding a
/// conditional field cannot silently change which field a test was focusing.
fn fields(dialog: &NewSessionDialog) -> layout::FieldLayout {
    dialog.field_layout()
}

#[test]
fn test_initial_state() {
    let dialog = single_tool_dialog();
    assert_eq!(dialog.title.value(), "");
    assert_eq!(dialog.path.value(), TEST_PATH);
    assert_eq!(dialog.group.value(), "");
    assert_eq!(dialog.focused_field, fields(&dialog).title);
    assert_eq!(dialog.tool_index, 0);
    assert_eq!(dialog.profile, "default");
}

#[test]
fn test_esc_cancels() {
    let mut dialog = single_tool_dialog();
    let result = dialog.handle_key(key(KeyCode::Esc));
    assert!(matches!(result, DialogResult::Cancel));
}

#[test]
fn test_enter_submits_with_auto_title() {
    use crate::session::civilizations;

    let mut dialog = single_tool_dialog();
    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(
                civilizations::CIVILIZATIONS.contains(&data.title.as_str()),
                "Expected a civilization name, got: {}",
                data.title
            );
            assert_eq!(data.path, TEST_PATH);
            assert_eq!(data.group, "");
            assert_eq!(data.tool, "claude");
            assert_eq!(data.profile, "default");
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_enter_preserves_custom_title() {
    let mut dialog = single_tool_dialog();
    dialog.title = Input::new("My Custom Title".to_string());
    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert_eq!(data.title, "My Custom Title");
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_tab_cycles_fields_single_tool() {
    let mut dialog = single_tool_dialog();
    let f = fields(&dialog);
    assert_eq!(dialog.focused_field, f.title);
    assert_eq!(f.tool, layout::ABSENT, "single tool is not focusable");

    for expected in [
        f.path,
        f.right_pane,
        f.yolo,
        f.cross_agent_team,
        f.worktree,
        f.group,
        f.title,
    ] {
        dialog.handle_key(key(KeyCode::Tab));
        assert_eq!(dialog.focused_field, expected);
    }
}

#[test]
fn test_tab_cycles_fields_single_tool_with_worktree() {
    // Even with worktree set, new_branch and extra_repos are in a Ctrl+P overlay,
    // so the main form has the same tab stops as without worktree.
    let mut dialog = single_tool_dialog();
    dialog.worktree_branch = Input::new("feature".to_string());
    let f = fields(&dialog);
    assert_eq!(dialog.focused_field, f.title);
    assert_ne!(f.new_branch, layout::ABSENT, "worktree shows new branch");

    for expected in [
        f.path,
        f.right_pane,
        f.yolo,
        f.cross_agent_team,
        f.worktree,
        f.new_branch,
        f.group,
        f.title,
    ] {
        dialog.handle_key(key(KeyCode::Tab));
        assert_eq!(dialog.focused_field, expected);
    }
}

#[test]
fn test_tab_cycles_fields_multi_tool() {
    let mut dialog = multi_tool_dialog();
    let f = fields(&dialog);
    assert_eq!(dialog.focused_field, f.title);
    assert_ne!(f.tool, layout::ABSENT, "multiple tools are focusable");

    for expected in [
        f.path,
        f.tool,
        f.right_pane,
        f.yolo,
        f.cross_agent_team,
        f.worktree,
        f.group,
        f.title,
    ] {
        dialog.handle_key(key(KeyCode::Tab));
        assert_eq!(dialog.focused_field, expected);
    }
}

#[test]
fn test_backtab_cycles_fields_reverse() {
    let mut dialog = single_tool_dialog();
    let f = fields(&dialog);
    assert_eq!(dialog.focused_field, f.title);

    for expected in [
        f.group,
        f.worktree,
        f.cross_agent_team,
        f.yolo,
        f.right_pane,
        f.path,
        f.title,
    ] {
        dialog.handle_key(shift_key(KeyCode::BackTab));
        assert_eq!(dialog.focused_field, expected);
    }
}

#[test]
fn test_char_input_to_title() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).title;
    dialog.handle_key(key(KeyCode::Char('H')));
    dialog.handle_key(key(KeyCode::Char('i')));
    assert_eq!(dialog.title.value(), "Hi");
}

#[test]
fn test_char_input_to_path() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.handle_key(key(KeyCode::Char('/')));
    dialog.handle_key(key(KeyCode::Char('a')));
    assert_eq!(dialog.path.value(), format!("{TEST_PATH}/a"));
}

#[test]
fn test_ghost_text_appears_for_single_match() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");
    fs::write(tmp.path().join("project-file"), "not a directory").expect("failed to write file");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/pro", tmp.path().display()));
    dialog.path.recompute_ghost();

    assert_eq!(dialog.path.ghost_text(), Some("ject-alpha/"));
}

#[test]
fn test_ghost_text_shows_common_prefix_for_multiple_matches() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("client-api")).expect("failed to create directory");
    fs::create_dir(tmp.path().join("client-web")).expect("failed to create directory");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/cl", tmp.path().display()));
    dialog.path.recompute_ghost();

    assert_eq!(dialog.path.ghost_text(), Some("ient-"));
}

#[test]
fn test_ghost_text_none_when_no_matches() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/zzz_nonexistent", tmp.path().display()));
    dialog.path.recompute_ghost();

    assert_eq!(dialog.path.ghost_text(), None);
}

#[test]
fn test_ghost_shows_slash_for_exact_directory_match() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("alpha")).expect("failed to create directory");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/alpha", tmp.path().display()));
    dialog.path.recompute_ghost();

    assert_eq!(dialog.path.ghost_text(), Some("/"));
}

#[test]
fn test_right_arrow_accepts_ghost_text() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/pro", tmp.path().display()));
    dialog.path.recompute_ghost();
    assert!(dialog.path.ghost_text().is_some());

    dialog.handle_key(key(KeyCode::Right));

    assert_eq!(
        dialog.path.value(),
        format!("{}/project-alpha/", tmp.path().display())
    );
}

#[test]
fn test_end_key_accepts_ghost_text() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/pro", tmp.path().display()));
    dialog.path.recompute_ghost();
    assert!(dialog.path.ghost_text().is_some());

    dialog.handle_key(key(KeyCode::End));

    assert_eq!(
        dialog.path.value(),
        format!("{}/project-alpha/", tmp.path().display())
    );
}

#[test]
fn test_right_arrow_at_mid_input_moves_cursor_normally() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new("/tmp/alpha/beta".to_string());
    // Move cursor to start
    dialog.handle_key(ctrl_key(KeyCode::Char('a')));
    let cursor_before = dialog.path.cursor();

    dialog.handle_key(key(KeyCode::Right));
    let cursor_after = dialog.path.cursor();

    // Cursor should have moved right by 1 (normal behavior)
    assert_eq!(cursor_after, cursor_before + 1);
}

#[test]
fn test_ghost_recomputes_after_accepting() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("alpha")).expect("failed to create directory");
    fs::create_dir(tmp.path().join("alpha").join("inner")).expect("failed to create directory");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/alp", tmp.path().display()));
    dialog.path.recompute_ghost();
    assert_eq!(dialog.path.ghost_text(), Some("ha/"));

    dialog.handle_key(key(KeyCode::Right)); // accept ghost

    assert_eq!(
        dialog.path.value(),
        format!("{}/alpha/", tmp.path().display())
    );
    // Ghost should have been recomputed for the next level
    assert_eq!(dialog.path.ghost_text(), Some("inner/"));
}

#[test]
fn test_tab_always_navigates_from_path_field() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/pro", tmp.path().display()));
    dialog.path.recompute_ghost();
    assert!(dialog.path.ghost_text().is_some());

    dialog.handle_key(key(KeyCode::Tab));

    // Tab should navigate to next field, not accept ghost
    assert_eq!(dialog.focused_field, fields(&dialog).right_pane);
}

#[test]
fn test_ghost_cleared_when_leaving_path_field() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/pro", tmp.path().display()));
    dialog.path.recompute_ghost();
    assert!(dialog.path.ghost_text().is_some());

    dialog.handle_key(key(KeyCode::Tab));

    assert_eq!(dialog.path.ghost_text(), None);
}

#[test]
fn test_ghost_not_shown_when_cursor_not_at_end() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    fs::create_dir(tmp.path().join("alpha")).expect("failed to create directory");

    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new(format!("{}/alp", tmp.path().display()));
    // Move cursor to start
    dialog.handle_key(ctrl_key(KeyCode::Char('a')));
    dialog.path.recompute_ghost();

    assert_eq!(dialog.path.ghost_text(), None);
}

#[test]
fn test_invalid_path_flash_expires_after_tick() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog
        .path
        .flash_invalid_until(std::time::Instant::now() - std::time::Duration::from_millis(1));
    assert!(dialog.tick());
    assert!(!dialog.path.is_invalid_flash_active());
}

#[test]
fn test_ctrl_left_jumps_to_previous_path_segment() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new("/tmp/alpha/beta".to_string());

    dialog.handle_key(ctrl_key(KeyCode::Left));
    dialog.handle_key(key(KeyCode::Char('X')));

    assert_eq!(dialog.path.value(), "/tmp/alpha/Xbeta");
}

#[test]
fn test_alt_b_jumps_to_previous_path_segment() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new("/tmp/alpha/beta".to_string());

    dialog.handle_key(alt_key(KeyCode::Char('b')));
    dialog.handle_key(key(KeyCode::Char('X')));

    assert_eq!(dialog.path.value(), "/tmp/alpha/Xbeta");
}

#[test]
fn test_ctrl_a_jumps_to_start_of_path() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).path;
    dialog.path = PathField::new("/tmp/alpha/beta".to_string());

    dialog.handle_key(ctrl_key(KeyCode::Char('a')));
    dialog.handle_key(key(KeyCode::Char('X')));

    assert_eq!(dialog.path.value(), "X/tmp/alpha/beta");
}

#[test]
fn test_char_input_to_group() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).group;
    dialog.handle_key(key(KeyCode::Char('w')));
    dialog.handle_key(key(KeyCode::Char('o')));
    dialog.handle_key(key(KeyCode::Char('r')));
    dialog.handle_key(key(KeyCode::Char('k')));
    assert_eq!(dialog.group.value(), "work");
}

#[test]
fn test_backspace_removes_char() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).title;
    dialog.title = Input::new("Hello".to_string());
    dialog.handle_key(key(KeyCode::Backspace));
    assert_eq!(dialog.title.value(), "Hell");
}

#[test]
fn test_backspace_on_empty_field() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).title;
    dialog.handle_key(key(KeyCode::Backspace));
    assert_eq!(dialog.title.value(), "");
}

#[test]
fn test_tool_selection_left_right() {
    let mut dialog = multi_tool_dialog();
    dialog.focused_field = fields(&dialog).tool;
    assert_eq!(dialog.tool_index, 0);

    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.tool_index, 1);

    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.tool_index, 0);

    dialog.handle_key(key(KeyCode::Left));
    assert_eq!(dialog.tool_index, 1);
}

#[test]
fn test_tool_selection_space() {
    let mut dialog = multi_tool_dialog();
    dialog.focused_field = fields(&dialog).tool;
    assert_eq!(dialog.tool_index, 0);

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(dialog.tool_index, 1);

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(dialog.tool_index, 0);
}

#[test]
fn test_tool_selection_ignored_on_text_field() {
    let mut dialog = multi_tool_dialog();
    dialog.focused_field = fields(&dialog).title;
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(dialog.title.value(), " ");
    assert_eq!(dialog.tool_index, 0);
}

#[test]
fn test_tool_selection_ignored_single_tool() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).right_pane;
    dialog.handle_key(key(KeyCode::Left));
    assert_eq!(dialog.tool_index, 0);
}

#[test]
fn test_submit_with_selected_tool() {
    let mut dialog = multi_tool_dialog();
    dialog.focused_field = fields(&dialog).tool;
    dialog.handle_key(key(KeyCode::Right));
    dialog.title = Input::new("Test".to_string());

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert_eq!(data.tool, "opencode");
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_unknown_key_continues() {
    let mut dialog = single_tool_dialog();
    let result = dialog.handle_key(key(KeyCode::F(1)));
    assert!(matches!(result, DialogResult::Continue));
}

#[test]
fn test_error_clears_on_input() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).title;
    dialog.error_message = Some("Some error".to_string());

    dialog.handle_key(key(KeyCode::Char('a')));
    assert_eq!(dialog.error_message, None);
}

#[test]
fn test_esc_clears_error() {
    let mut dialog = single_tool_dialog();
    dialog.error_message = Some("Some error".to_string());

    let result = dialog.handle_key(key(KeyCode::Esc));
    assert!(matches!(result, DialogResult::Cancel));
    assert_eq!(dialog.error_message, None);
}

#[test]
fn test_new_branch_checkbox_default_true() {
    let dialog = single_tool_dialog();
    assert!(dialog.create_new_branch);
}

#[test]
fn test_new_branch_checkbox_toggle() {
    let mut dialog = single_tool_dialog();
    dialog.worktree_branch = Input::new("feature-branch".to_string());
    dialog.focused_field = fields(&dialog).new_branch;
    assert!(dialog.create_new_branch);

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(!dialog.create_new_branch);

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.create_new_branch);
}

#[test]
fn test_submit_respects_create_new_branch() {
    let mut dialog = single_tool_dialog();
    dialog.worktree_branch = Input::new("feature-branch".to_string());
    dialog.focused_field = fields(&dialog).new_branch;
    dialog.handle_key(key(KeyCode::Char(' '))); // Toggle off

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(!data.create_new_branch);
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_new_branch_field_hidden_without_worktree() {
    let mut dialog = single_tool_dialog();
    let f = fields(&dialog);
    assert_eq!(f.new_branch, layout::ABSENT);
    assert_eq!(dialog.focused_field, f.title);

    for _ in 0..(f.count - 1) {
        dialog.handle_key(key(KeyCode::Tab));
    }
    assert_eq!(dialog.focused_field, f.group);
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.focused_field, f.title);
}

#[test]
fn test_sandbox_disabled_by_default() {
    let dialog = multi_tool_dialog();
    assert!(!dialog.sandbox_enabled);
}

#[test]
fn test_sandbox_image_initialized_with_effective_default() {
    use crate::containers;
    let dialog = multi_tool_dialog();
    assert_eq!(
        dialog.sandbox_image.value(),
        containers::get_container_runtime().effective_default_image()
    );
}

#[test]
fn test_tab_skips_sandbox_options_in_main_form() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;

    // With sandbox enabled, sandbox sub-options are in a separate mode.
    let f = fields(&dialog);
    for _ in 0..(f.count - 2) {
        dialog.handle_key(key(KeyCode::Tab));
    }
    assert_eq!(dialog.focused_field, f.sandbox);

    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.focused_field, f.group);

    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.focused_field, f.title);
}

#[test]
fn test_tab_skips_sandbox_when_disabled() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = false;

    // claude + docker + no sandbox also shows the cross agent team checkbox.
    let f = fields(&dialog);
    assert_ne!(f.cross_agent_team, layout::ABSENT);
    for _ in 0..(f.count - 2) {
        dialog.handle_key(key(KeyCode::Tab));
    }
    assert_eq!(dialog.focused_field, f.sandbox);

    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.focused_field, f.group);

    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.focused_field, f.title);
}

#[test]
fn test_submit_with_custom_sandbox_image() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.sandbox_image = Input::new("custom/image:tag".to_string());
    dialog.title = Input::new("Test".to_string());

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(data.sandbox);
            assert_eq!(data.sandbox_image, "custom/image:tag");
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_submit_with_default_image_passes_through() {
    use crate::containers;
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.title = Input::new("Test".to_string());

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(data.sandbox);
            assert_eq!(
                data.sandbox_image,
                containers::get_container_runtime().effective_default_image()
            );
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_submit_with_empty_image() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.sandbox_image = Input::new("".to_string());
    dialog.title = Input::new("Test".to_string());

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(data.sandbox);
            assert_eq!(data.sandbox_image, "");
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_submit_sandbox_image_always_included() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = false;
    dialog.sandbox_image = Input::new("custom/image:tag".to_string());
    dialog.title = Input::new("Test".to_string());

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(!data.sandbox);
            assert_eq!(data.sandbox_image, "custom/image:tag");
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_sandbox_image_input_in_config_mode() {
    use crate::containers;
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.sandbox_config_mode = true;
    dialog.sandbox_focused_field = 0; // image field

    dialog.handle_key(key(KeyCode::Char('a')));
    dialog.handle_key(key(KeyCode::Char('b')));
    dialog.handle_key(key(KeyCode::Char('c')));

    let expected = format!(
        "{}abc",
        containers::get_container_runtime().effective_default_image()
    );
    assert_eq!(dialog.sandbox_image.value(), expected);
}

#[test]
fn test_yolo_mode_disabled_by_default() {
    let dialog = multi_tool_dialog();
    assert!(!dialog.yolo_mode);
}

#[test]
fn test_yolo_mode_toggle() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.focused_field = fields(&dialog).yolo;
    assert!(!dialog.yolo_mode);

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.yolo_mode);

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(!dialog.yolo_mode);
}

#[test]
fn test_submit_with_yolo_mode_enabled() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.yolo_mode = true;
    dialog.title = Input::new("Test".to_string());

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(data.sandbox);
            assert!(data.yolo_mode);
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_yolo_independent_of_sandbox() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = false;
    dialog.yolo_mode = true;
    dialog.title = Input::new("Test".to_string());

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(!data.sandbox);
            assert!(data.yolo_mode);
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_disabling_sandbox_does_not_reset_yolo_mode() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.yolo_mode = true;
    dialog.focused_field = fields(&dialog).sandbox;

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(!dialog.sandbox_enabled);
    assert!(dialog.yolo_mode);
}

#[test]
fn help_content_fits_in_dialog() {
    const BORDER_WIDTH: u16 = 2;
    const INDENT: usize = 2;
    let available_width = (HELP_DIALOG_WIDTH - BORDER_WIDTH) as usize;

    for help in FIELD_HELP {
        let line_width = INDENT + help.description.len();
        assert!(
            line_width <= available_width,
            "Help for '{}': description '{}' exceeds dialog width ({} > {})",
            help.name,
            help.description,
            line_width,
            available_width
        );
    }
}

#[test]
fn test_profile_override_sets_default_tool() {
    let global = Config::default();
    let profile_config = ProfileConfig {
        session: Some(SessionConfigOverride {
            default_tool: Some("opencode".to_string()),
            yolo_mode_default: None,
            ..Default::default()
        }),
        ..Default::default()
    };

    let resolved = merge_configs(global, &profile_config);
    let dialog = NewSessionDialog::new_with_config(
        vec!["claude", "opencode"],
        "/tmp/project".to_string(),
        resolved,
    );

    assert_eq!(
        dialog.tool_index, 1,
        "Profile override should select opencode (index 1)"
    );
    assert_eq!(dialog.available_tools[dialog.tool_index], "opencode");
}

#[test]
fn test_profile_override_beats_global_default_tool() {
    let mut global = Config::default();
    global.session.default_tool = Some("claude".to_string());

    let profile_config = ProfileConfig {
        session: Some(SessionConfigOverride {
            default_tool: Some("opencode".to_string()),
            yolo_mode_default: None,
            ..Default::default()
        }),
        ..Default::default()
    };

    let resolved = merge_configs(global, &profile_config);
    assert_eq!(
        resolved.session.default_tool.as_deref(),
        Some("opencode"),
        "Profile override should take precedence over global default"
    );

    let dialog = NewSessionDialog::new_with_config(
        vec!["claude", "opencode"],
        "/tmp/project".to_string(),
        resolved,
    );

    assert_eq!(
        dialog.tool_index, 1,
        "Profile override should select opencode over global claude"
    );
    assert_eq!(dialog.available_tools[dialog.tool_index], "opencode");
}

// --- create-directory confirmation tests ---

/// Stage the confirmation the way Enter would, with `yes` preselected.
fn stage_create_dirs_confirm(dialog: &mut NewSessionDialog, yes: bool) {
    dialog.confirm_create_dirs = Some(CreateDirsConfirm {
        dirs: dialog.missing_directories(),
        yes_selected: yes,
    });
}

fn confirm_selection(dialog: &NewSessionDialog) -> Option<bool> {
    dialog
        .confirm_create_dirs
        .as_ref()
        .map(|confirm| confirm.yes_selected)
}

fn confirm_dirs(dialog: &NewSessionDialog) -> Vec<String> {
    dialog
        .confirm_create_dirs
        .as_ref()
        .map(|confirm| confirm.dirs.clone())
        .unwrap_or_default()
}

fn nonexistent_dialog() -> NewSessionDialog {
    NewSessionDialog::new_with_tools(vec!["claude"], "/__aoe_nonexistent__/project".to_string())
}

#[test]
fn test_enter_with_nonexistent_path_enters_confirm() {
    let mut dialog = nonexistent_dialog();
    let result = dialog.handle_key(key(KeyCode::Enter));
    assert!(matches!(result, DialogResult::Continue));
    assert_eq!(confirm_selection(&dialog), Some(false));
}

#[test]
fn test_enter_with_existing_path_submits_directly() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], tmp.path().to_string_lossy().to_string());
    let result = dialog.handle_key(key(KeyCode::Enter));
    assert!(matches!(result, DialogResult::Submit(_)));
    assert!(dialog.confirm_create_dirs.is_none());
}

#[test]
fn test_confirm_esc_cancels() {
    let mut dialog = nonexistent_dialog();
    stage_create_dirs_confirm(&mut dialog, false);
    let result = dialog.handle_key(key(KeyCode::Esc));
    assert!(matches!(result, DialogResult::Continue));
    assert!(dialog.confirm_create_dirs.is_none());
    assert_eq!(dialog.focused_field, dialog.path_field());
}

#[test]
fn test_confirm_n_cancels() {
    let mut dialog = nonexistent_dialog();
    stage_create_dirs_confirm(&mut dialog, true);
    dialog.handle_key(key(KeyCode::Char('n')));
    assert!(dialog.confirm_create_dirs.is_none());
    assert_eq!(dialog.focused_field, dialog.path_field());
}

#[test]
fn test_confirm_h_selects_yes() {
    let mut dialog = nonexistent_dialog();
    stage_create_dirs_confirm(&mut dialog, false);
    dialog.handle_key(key(KeyCode::Char('h')));
    assert_eq!(confirm_selection(&dialog), Some(true));
}

#[test]
fn test_confirm_l_selects_no() {
    let mut dialog = nonexistent_dialog();
    stage_create_dirs_confirm(&mut dialog, true);
    dialog.handle_key(key(KeyCode::Char('l')));
    assert_eq!(confirm_selection(&dialog), Some(false));
}

#[test]
fn test_confirm_tab_toggles() {
    let mut dialog = nonexistent_dialog();
    stage_create_dirs_confirm(&mut dialog, false);
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(confirm_selection(&dialog), Some(true));
    stage_create_dirs_confirm(&mut dialog, true);
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(confirm_selection(&dialog), Some(false));
}

#[test]
fn test_confirm_y_creates_dir_and_submits() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let new_path = tmp.path().join("new_project");
    assert!(!new_path.exists());

    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], new_path.to_string_lossy().to_string());
    stage_create_dirs_confirm(&mut dialog, false);
    let result = dialog.handle_key(key(KeyCode::Char('y')));
    assert!(matches!(result, DialogResult::Submit(_)));
    assert!(new_path.exists());
}

#[test]
fn test_confirm_enter_yes_creates_dir_and_submits() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let new_path = tmp.path().join("another_dir");

    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], new_path.to_string_lossy().to_string());
    stage_create_dirs_confirm(&mut dialog, true);
    let result = dialog.handle_key(key(KeyCode::Enter));
    assert!(matches!(result, DialogResult::Submit(_)));
    assert!(new_path.exists());
}

#[test]
fn test_confirm_enter_no_cancels() {
    let mut dialog = nonexistent_dialog();
    stage_create_dirs_confirm(&mut dialog, false);
    let result = dialog.handle_key(key(KeyCode::Enter));
    assert!(matches!(result, DialogResult::Continue));
    assert!(dialog.confirm_create_dirs.is_none());
    assert_eq!(dialog.focused_field, dialog.path_field());
}

#[test]
fn test_confirm_create_failure_shows_error() {
    let mut dialog = NewSessionDialog::new_with_tools(
        vec!["claude"],
        "/proc/aoe_test_cannot_create".to_string(),
    );
    stage_create_dirs_confirm(&mut dialog, true);
    let result = dialog.handle_key(key(KeyCode::Char('y')));
    assert!(matches!(result, DialogResult::Continue));
    assert!(dialog.error_message.is_some());
    assert!(dialog.confirm_create_dirs.is_none());
}

// --- Profile tests ---

#[test]
fn test_profile_always_current() {
    let mut dialog = single_tool_dialog();
    assert_eq!(dialog.profile, "default");

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert_eq!(data.profile, "default");
        }
        _ => panic!("Expected Submit"),
    }
}

// --- Sandbox config mode tests ---

#[test]
fn test_ctrl_p_on_sandbox_enters_config_mode() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.focused_field = fields(&dialog).sandbox;

    let result = dialog.handle_key(ctrl_key(KeyCode::Char('p')));
    assert!(matches!(result, DialogResult::Continue));
    assert!(dialog.sandbox_config_mode);
    assert_eq!(dialog.sandbox_focused_field, 0);
}

#[test]
fn test_enter_on_sandbox_submits() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    dialog.focused_field = fields(&dialog).group;

    let result = dialog.handle_key(key(KeyCode::Enter));
    // Enter should submit, not enter config mode
    assert!(!dialog.sandbox_config_mode);
    assert!(matches!(result, DialogResult::Submit(_)));
}

#[test]
fn test_ctrl_p_on_disabled_sandbox_does_not_open_config() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.sandbox_enabled = false;
    dialog.focused_field = fields(&dialog).group;

    dialog.handle_key(ctrl_key(KeyCode::Char('p')));
    assert!(!dialog.sandbox_config_mode);
}

#[test]
fn test_sandbox_config_mode_esc_returns_to_main() {
    let mut dialog = multi_tool_dialog();
    dialog.sandbox_config_mode = true;
    dialog.sandbox_focused_field = 1;

    let result = dialog.handle_key(key(KeyCode::Esc));
    assert!(matches!(result, DialogResult::Continue));
    assert!(!dialog.sandbox_config_mode);
}

#[test]
fn test_sandbox_config_mode_tab_cycles() {
    let mut dialog = multi_tool_dialog();
    dialog.sandbox_config_mode = true;
    dialog.sandbox_focused_field = 0;

    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.sandbox_focused_field, 1);

    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.sandbox_focused_field, 0); // wrap
}

#[test]
fn test_sandbox_config_mode_enter_on_image_returns_to_main() {
    let mut dialog = multi_tool_dialog();
    dialog.sandbox_config_mode = true;
    dialog.sandbox_focused_field = 0; // image

    let result = dialog.handle_key(key(KeyCode::Enter));
    assert!(matches!(result, DialogResult::Continue));
    assert!(!dialog.sandbox_config_mode);
}

// --- Worktree reuse confirmation tests ---

#[test]
fn test_reuse_worktree_second_enter_submits_with_flag() {
    let mut dialog = single_tool_dialog();
    dialog.worktree_branch = Input::new("feat/test".to_string());
    // Simulate the state after first Enter showed the warning
    dialog.confirm_reuse_worktree = true;

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(data.reuse_worktree);
            assert_eq!(data.worktree_branch, Some("feat/test".to_string()));
        }
        _ => panic!("Expected Submit on second Enter"),
    }
}

#[test]
fn test_reuse_worktree_flag_false_when_not_confirmed() {
    let mut dialog = single_tool_dialog();
    dialog.worktree_branch = Input::new("feat/test".to_string());
    // confirm_reuse_worktree is false by default

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(!data.reuse_worktree);
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_reuse_worktree_confirmation_cleared_on_text_input() {
    let mut dialog = single_tool_dialog();
    dialog.focused_field = fields(&dialog).worktree;
    dialog.confirm_reuse_worktree = true;
    dialog.error_message = Some("Worktree exists".to_string());

    dialog.handle_key(key(KeyCode::Char('a')));

    assert!(!dialog.confirm_reuse_worktree);
    assert!(dialog.error_message.is_none());
}

#[test]
fn test_right_pane_default_none() {
    let dialog = multi_tool_dialog();
    assert_eq!(dialog.right_pane_tool_index, 0);
}

#[test]
fn test_right_pane_cycle_left_right() {
    let mut dialog = multi_tool_dialog();
    dialog.focused_field = fields(&dialog).right_pane;
    assert_eq!(dialog.right_pane_tool_index, 0); // none

    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.right_pane_tool_index, 1); // claude

    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.right_pane_tool_index, 2); // opencode

    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.right_pane_tool_index, 0); // wrap to none

    dialog.handle_key(key(KeyCode::Left));
    assert_eq!(dialog.right_pane_tool_index, 2); // wrap to opencode
}

#[test]
fn test_right_pane_none_submits_without_tool() {
    let mut dialog = multi_tool_dialog();
    assert_eq!(dialog.right_pane_tool_index, 0); // none

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(data.right_pane_tool.is_none());
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_right_pane_selected_tool_submits() {
    let mut dialog = multi_tool_dialog();
    dialog.right_pane_tool_index = 2; // opencode (index 2 maps to available_tools[1])

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert_eq!(data.right_pane_tool, Some("opencode".to_string()));
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_has_yolo_with_shell_left_and_code_agent_right_pane() {
    let mut dialog = multi_tool_dialog();
    dialog.available_tools.push("shell");
    dialog.tool_index = 2; // shell
    dialog.right_pane_tool_index = 1; // claude

    assert!(dialog.has_yolo_field());
}

#[test]
fn test_has_yolo_with_shell_left_and_no_right_pane() {
    let mut dialog = multi_tool_dialog();
    dialog.available_tools.push("shell");
    dialog.tool_index = 2; // shell
    dialog.right_pane_tool_index = 0; // none

    assert!(!dialog.has_yolo_field());
}

#[test]
fn test_has_yolo_with_shell_left_and_shell_right_pane() {
    let mut dialog = multi_tool_dialog();
    dialog.available_tools.push("shell");
    dialog.tool_index = 2; // shell
    dialog.right_pane_tool_index = 3; // shell

    assert!(!dialog.has_yolo_field());
}

#[test]
fn test_has_yolo_with_code_agent_left_and_no_right_pane() {
    let dialog = single_tool_dialog();

    assert!(dialog.has_yolo_field());
}

#[test]
fn test_yolo_default_restored_when_right_pane_needs_yolo_and_left_is_shell() {
    let mut dialog = multi_tool_dialog();
    dialog.available_tools.push("shell");
    dialog.yolo_mode_default = true;
    dialog.yolo_mode = true;
    // Switch left to shell: yolo_mode saved, set false (no right pane yet)
    dialog.tool_index = 2; // shell
    dialog.reload_tool_config();
    assert!(!dialog.yolo_mode);
    // Now select right pane = claude
    dialog.right_pane_tool_index = 1;
    dialog.sync_yolo_for_right_pane();
    assert!(dialog.yolo_mode);
}

#[test]
fn test_yolo_stays_false_when_right_pane_is_none_and_left_is_shell() {
    let mut dialog = multi_tool_dialog();
    dialog.available_tools.push("shell");
    dialog.yolo_mode_default = true;
    dialog.yolo_mode = true;
    dialog.tool_index = 2; // shell
    dialog.reload_tool_config();
    // Right pane is "none"
    dialog.right_pane_tool_index = 0;
    dialog.sync_yolo_for_right_pane();
    assert!(!dialog.yolo_mode);
}

#[test]
fn test_yolo_preserved_when_switching_to_shell_with_right_pane_code_agent() {
    let mut dialog = multi_tool_dialog();
    dialog.available_tools.push("shell");
    dialog.yolo_mode = true;
    dialog.right_pane_tool_index = 1; // claude
                                      // Switch left to shell: yolo_mode should stay true since right pane needs it
    dialog.tool_index = 2;
    dialog.reload_tool_config();
    assert!(dialog.yolo_mode);
}

#[test]
fn test_reuse_worktree_confirmation_cleared_on_branch_picker_select() {
    let mut dialog = single_tool_dialog();
    dialog.confirm_reuse_worktree = true;
    dialog
        .branch_picker
        .activate(vec!["main".to_string(), "dev".to_string()]);

    // Select first item with Enter
    dialog.handle_key(key(KeyCode::Enter));

    assert!(!dialog.confirm_reuse_worktree);
    assert_eq!(dialog.worktree_branch.value(), "main");
}

#[test]
fn test_cross_agent_team_field_visible_for_claude() {
    let dialog = single_tool_dialog();
    assert!(dialog.has_cross_agent_team_field());
}

#[test]
fn test_cross_agent_team_field_visible_for_codex() {
    let dialog = NewSessionDialog::new_with_tools(vec!["codex"], TEST_PATH.to_string());
    assert!(dialog.has_cross_agent_team_field());
}

#[test]
fn test_cross_agent_team_field_hidden_for_unsupported_tool() {
    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["opencode", "claude"], TEST_PATH.to_string());
    dialog.tool_index = 0; // opencode
    assert!(!dialog.has_cross_agent_team_field());
    dialog.tool_index = 1; // claude
    assert!(dialog.has_cross_agent_team_field());
}

#[test]
fn test_cross_agent_team_field_hidden_when_sandbox() {
    let mut dialog = NewSessionDialog::new_with_tools(vec!["codex"], TEST_PATH.to_string());
    dialog.docker_available = true;
    dialog.sandbox_enabled = true;
    assert!(!dialog.has_cross_agent_team_field());
    dialog.sandbox_enabled = false;
    assert!(dialog.has_cross_agent_team_field());
}

#[test]
fn test_cross_agent_team_default_from_config() {
    let mut global = Config::default();
    global.session.cross_agent_team_default = true;
    global.session.cross_agent_team_channel = "server:custom-channel".to_string();

    let dialog =
        NewSessionDialog::new_with_config(vec!["codex"], "/tmp/project".to_string(), global);

    assert!(dialog.cross_agent_team);
    assert_eq!(dialog.cross_agent_team_channel, "server:custom-channel");
}

#[test]
fn test_cross_agent_team_toggle_and_submit() {
    let mut dialog = NewSessionDialog::new_with_tools(vec!["codex"], TEST_PATH.to_string());
    dialog.focused_field = fields(&dialog).cross_agent_team;
    assert!(!dialog.cross_agent_team);

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.cross_agent_team);

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(data.cross_agent_team);
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_codex_cross_agent_team_toggle_is_independent_of_yolo() {
    let mut dialog = NewSessionDialog::new_with_tools(vec!["codex"], TEST_PATH.to_string());
    dialog.focused_field = fields(&dialog).cross_agent_team;
    assert!(!dialog.yolo_mode);
    assert!(!dialog.cross_agent_team);

    dialog.handle_key(key(KeyCode::Char(' ')));

    assert!(!dialog.yolo_mode);
    assert!(dialog.cross_agent_team);

    dialog.focused_field = fields(&dialog).yolo;
    dialog.handle_key(key(KeyCode::Char(' ')));

    assert!(dialog.yolo_mode);
    assert!(dialog.cross_agent_team);
}

#[test]
fn test_left_right_navigates_between_yolo_and_cross_agent_team() {
    let mut dialog = single_tool_dialog();
    let f = fields(&dialog);
    dialog.focused_field = f.yolo;

    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(
        dialog.focused_field, f.cross_agent_team,
        "Right moves focus yolo -> cross agent teams"
    );

    dialog.handle_key(key(KeyCode::Left));
    assert_eq!(
        dialog.focused_field, f.yolo,
        "Left moves focus cross agent teams -> yolo"
    );

    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.focused_field, f.cross_agent_team);
    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(
        dialog.focused_field, f.yolo,
        "Right from cross agent teams returns to yolo"
    );
}

#[test]
fn test_space_toggles_not_navigates_on_yolo_row() {
    let mut dialog = single_tool_dialog();
    let f = fields(&dialog);
    dialog.focused_field = f.yolo;
    let yolo_before = dialog.yolo_mode;
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(dialog.focused_field, f.yolo, "Space does not move focus");
    assert_ne!(dialog.yolo_mode, yolo_before, "Space toggles yolo");

    dialog.focused_field = f.cross_agent_team;
    assert!(!dialog.cross_agent_team);
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(
        dialog.focused_field, f.cross_agent_team,
        "Space does not move focus"
    );
    assert!(dialog.cross_agent_team, "Space toggles cross agent teams");
}

#[test]
fn test_cross_agent_team_not_submitted_for_non_claude() {
    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["opencode", "claude"], TEST_PATH.to_string());
    dialog.tool_index = 0; // opencode
    dialog.cross_agent_team = true; // even if somehow set

    let result = dialog.handle_key(key(KeyCode::Enter));
    match result {
        DialogResult::Submit(data) => {
            assert!(!data.cross_agent_team);
        }
        _ => panic!("Expected Submit"),
    }
}

// --- Field layout regression net ---
//
// Every conditional field the dialog gains rides on these: they assert that the
// index the layout hands out for a field is the index whose key handling that
// field actually answers, for each combination of conditional fields that can
// be on screen at once.

/// The fields the layout says are shown, in index order.
fn shown_fields(dialog: &NewSessionDialog) -> Vec<usize> {
    let f = fields(dialog);
    let mut shown: Vec<usize> = [
        f.title,
        f.path,
        f.tool,
        f.right_pane,
        f.right_pane_path,
        f.yolo,
        f.cross_agent_team,
        f.worktree,
        f.new_branch,
        f.sandbox,
        f.group,
    ]
    .into_iter()
    .filter(|&index| index != layout::ABSENT)
    .collect();
    shown.sort_unstable();
    shown
}

/// Tab from the first field through every field, recording where focus lands.
fn tab_order(dialog: &mut NewSessionDialog) -> Vec<usize> {
    dialog.focused_field = 0;
    let count = fields(dialog).count;
    let mut visited = vec![dialog.focused_field];
    for _ in 1..count {
        dialog.handle_key(key(KeyCode::Tab));
        visited.push(dialog.focused_field);
    }
    visited
}

/// Focus each shown field in turn and send the key that field responds to,
/// confirming the key reaches that field and not its neighbour. Every action is
/// undone so the layout stays the one `fields()` reported.
fn assert_each_field_answers_its_index(dialog: &mut NewSessionDialog, case: &str) {
    let f = fields(dialog);

    dialog.focused_field = f.title;
    let title_before = dialog.title.value().to_string();
    dialog.handle_key(key(KeyCode::Char('T')));
    assert_eq!(
        dialog.title.value(),
        format!("{title_before}T"),
        "{case}: title"
    );

    dialog.focused_field = f.path;
    let path_before = dialog.path.value().to_string();
    dialog.handle_key(key(KeyCode::Char('P')));
    assert_eq!(
        dialog.path.value(),
        format!("{path_before}P"),
        "{case}: path"
    );

    dialog.focused_field = f.group;
    let group_before = dialog.group.value().to_string();
    dialog.handle_key(key(KeyCode::Char('G')));
    assert_eq!(
        dialog.group.value(),
        format!("{group_before}G"),
        "{case}: group"
    );

    if f.tool != layout::ABSENT {
        dialog.focused_field = f.tool;
        let before = dialog.tool_index;
        dialog.handle_key(key(KeyCode::Right));
        assert_ne!(dialog.tool_index, before, "{case}: tool");
        dialog.handle_key(key(KeyCode::Left));
    }

    dialog.focused_field = f.right_pane;
    let right_pane_before = dialog.right_pane_tool_index;
    dialog.handle_key(key(KeyCode::Right));
    assert_ne!(
        dialog.right_pane_tool_index, right_pane_before,
        "{case}: right pane"
    );
    dialog.handle_key(key(KeyCode::Left));

    if f.yolo != layout::ABSENT {
        dialog.focused_field = f.yolo;
        let before = dialog.yolo_mode;
        dialog.handle_key(key(KeyCode::Char(' ')));
        assert_ne!(dialog.yolo_mode, before, "{case}: yolo");
        dialog.handle_key(key(KeyCode::Char(' ')));
    }

    if f.cross_agent_team != layout::ABSENT {
        dialog.focused_field = f.cross_agent_team;
        let before = dialog.cross_agent_team;
        dialog.handle_key(key(KeyCode::Char(' ')));
        assert_ne!(dialog.cross_agent_team, before, "{case}: cross agent team");
        dialog.handle_key(key(KeyCode::Char(' ')));
    }

    if f.sandbox != layout::ABSENT {
        dialog.focused_field = f.sandbox;
        let before = dialog.sandbox_enabled;
        dialog.handle_key(key(KeyCode::Char(' ')));
        assert_ne!(dialog.sandbox_enabled, before, "{case}: sandbox");
        dialog.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(dialog.sandbox_enabled, before, "{case}: sandbox restored");
    }

    if f.new_branch != layout::ABSENT {
        dialog.focused_field = f.new_branch;
        let before = dialog.create_new_branch;
        dialog.handle_key(key(KeyCode::Char(' ')));
        assert_ne!(dialog.create_new_branch, before, "{case}: new branch");
        dialog.handle_key(key(KeyCode::Char(' ')));
    }

    // Last: typing here can make the new branch checkbox appear.
    if f.worktree != layout::ABSENT {
        dialog.focused_field = f.worktree;
        let before = dialog.worktree_branch.value().to_string();
        dialog.handle_key(key(KeyCode::Char('W')));
        assert_eq!(
            dialog.worktree_branch.value(),
            format!("{before}W"),
            "{case}: worktree"
        );
    }
}

fn assert_layout_is_consistent(dialog: &mut NewSessionDialog, case: &str) {
    let shown = shown_fields(dialog);
    let count = fields(dialog).count;
    assert_eq!(
        shown,
        (0..count).collect::<Vec<_>>(),
        "{case}: shown fields must be the contiguous range Tab walks"
    );
    assert_eq!(tab_order(dialog), shown, "{case}: tab order");
    assert_each_field_answers_its_index(dialog, case);
}

#[test]
fn test_field_layout_holds_for_existing_conditional_combinations() {
    // No tool selector.
    assert_layout_is_consistent(&mut single_tool_dialog(), "single tool");

    // Tool selector shown.
    assert_layout_is_consistent(&mut multi_tool_dialog(), "multi tool");

    // Shell hides YOLO, Cross Agent Teams and the worktree fields.
    let mut shell = NewSessionDialog::new_with_tools(vec!["shell"], TEST_PATH.to_string());
    assert_eq!(fields(&shell).yolo, layout::ABSENT);
    assert_eq!(fields(&shell).worktree, layout::ABSENT);
    assert_layout_is_consistent(&mut shell, "shell");

    // Shell on the left with an agent on the right brings YOLO back.
    let mut shell_with_agent =
        NewSessionDialog::new_with_tools(vec!["shell", "claude"], TEST_PATH.to_string());
    shell_with_agent.right_pane_tool_index = 2; // claude
    assert_ne!(fields(&shell_with_agent).yolo, layout::ABSENT);
    assert_eq!(fields(&shell_with_agent).worktree, layout::ABSENT);
    assert_layout_is_consistent(&mut shell_with_agent, "shell with agent right pane");

    // Cross Agent Teams hidden for an unsupported tool.
    let mut unsupported = NewSessionDialog::new_with_tools(vec!["opencode"], TEST_PATH.to_string());
    assert_eq!(fields(&unsupported).cross_agent_team, layout::ABSENT);
    assert_layout_is_consistent(&mut unsupported, "cross agent teams unavailable");

    // Worktree branch set shows the new branch checkbox.
    let mut worktree = single_tool_dialog();
    worktree.worktree_branch = Input::new("feature".to_string());
    assert_ne!(fields(&worktree).new_branch, layout::ABSENT);
    assert_layout_is_consistent(&mut worktree, "worktree branch set");

    // Docker available shows the sandbox checkbox.
    let mut sandbox_available = multi_tool_dialog();
    sandbox_available.docker_available = true;
    assert_ne!(fields(&sandbox_available).sandbox, layout::ABSENT);
    assert_layout_is_consistent(&mut sandbox_available, "sandbox available");

    // Sandbox on hides Cross Agent Teams.
    let mut sandboxed = multi_tool_dialog();
    sandboxed.docker_available = true;
    sandboxed.sandbox_enabled = true;
    assert_eq!(fields(&sandboxed).cross_agent_team, layout::ABSENT);
    assert_layout_is_consistent(&mut sandboxed, "sandbox enabled");
}

// --- Right Pane Path field ---

#[test]
fn test_right_pane_path_field_appears_with_a_right_pane_tool() {
    let mut dialog = multi_tool_dialog();
    assert_eq!(fields(&dialog).right_pane_path, layout::ABSENT);

    dialog.focused_field = fields(&dialog).right_pane;
    dialog.handle_key(key(KeyCode::Right));
    let f = fields(&dialog);
    assert_ne!(f.right_pane_path, layout::ABSENT);
    assert_eq!(
        f.right_pane_path,
        f.right_pane + 1,
        "the field sits directly below Right Pane"
    );

    dialog.handle_key(key(KeyCode::Left));
    assert_eq!(fields(&dialog).right_pane_path, layout::ABSENT);
}

#[test]
fn test_sandboxing_hides_the_right_pane_path_field_and_restores_it() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.right_pane_tool_index = 1;
    assert_ne!(fields(&dialog).right_pane_path, layout::ABSENT);

    dialog.focused_field = fields(&dialog).sandbox;
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.sandbox_enabled);
    assert_eq!(fields(&dialog).right_pane_path, layout::ABSENT);

    dialog.focused_field = fields(&dialog).sandbox;
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(!dialog.sandbox_enabled);
    assert_ne!(fields(&dialog).right_pane_path, layout::ABSENT);
}

#[test]
fn test_hidden_right_pane_path_contributes_no_value() {
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.right_pane_tool_index = 1;
    dialog.right_pane_path = PathField::new("/tmp/elsewhere".to_string());
    dialog.sandbox_enabled = true;

    match dialog.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(data) => assert_eq!(
            data.right_pane_path, None,
            "a field that is not on screen must not reach the launch"
        ),
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_empty_right_pane_path_submits_as_none() {
    let mut dialog = multi_tool_dialog();
    dialog.right_pane_tool_index = 1;

    match dialog.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(data) => {
            assert_eq!(data.right_pane_tool.as_deref(), Some("claude"));
            assert_eq!(data.right_pane_path, None);
        }
        _ => panic!("Expected Submit"),
    }
}

#[test]
fn test_right_pane_path_is_submitted_when_set() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], tmp.path().to_string_lossy().to_string());
    dialog.right_pane_tool_index = 1;
    dialog.right_pane_path = PathField::new(tmp.path().to_string_lossy().to_string());

    match dialog.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(data) => assert_eq!(
            data.right_pane_path.as_deref(),
            Some(tmp.path().to_string_lossy().as_ref())
        ),
        _ => panic!("Expected Submit"),
    }
}

/// Drive the open directory picker down into `picked` and select it, the way a
/// user does: the listing is `./`, `../`, then the subdirectories, so two Downs
/// land on the only subdirectory, the first Enter navigates into it and the
/// second selects `./` there.
fn pick_the_only_subdirectory(dialog: &mut NewSessionDialog) {
    assert!(dialog.dir_picker.is_active(), "picker must be open");
    dialog.handle_key(key(KeyCode::Down));
    dialog.handle_key(key(KeyCode::Down));
    dialog.handle_key(key(KeyCode::Enter));
    dialog.handle_key(key(KeyCode::Enter));
    assert!(
        !dialog.dir_picker.is_active(),
        "selecting a directory closes the picker"
    );
}

#[test]
fn test_ctrl_p_on_right_pane_path_targets_that_field() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let picked = tmp.path().join("picked");
    fs::create_dir(&picked).expect("create dir");

    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], tmp.path().to_string_lossy().to_string());
    dialog.right_pane_tool_index = 1;
    dialog.right_pane_path = PathField::new(tmp.path().to_string_lossy().to_string());
    let session_path_before = dialog.path.value().to_string();

    dialog.focused_field = fields(&dialog).right_pane_path;
    dialog.handle_key(ctrl_key(KeyCode::Char('p')));
    pick_the_only_subdirectory(&mut dialog);

    assert_eq!(
        dialog.right_pane_path.value(),
        picked.to_string_lossy(),
        "the selection is written into the field the picker was opened from"
    );
    assert_eq!(
        dialog.path.value(),
        session_path_before,
        "the session path field is untouched"
    );
}

#[test]
fn test_ctrl_p_on_the_session_path_targets_that_field() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let picked = tmp.path().join("picked");
    fs::create_dir(&picked).expect("create dir");

    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], tmp.path().to_string_lossy().to_string());
    dialog.right_pane_tool_index = 1;

    dialog.focused_field = fields(&dialog).path;
    dialog.handle_key(ctrl_key(KeyCode::Char('p')));
    pick_the_only_subdirectory(&mut dialog);

    assert_eq!(dialog.path.value(), picked.to_string_lossy());
    assert_eq!(
        dialog.right_pane_path.value(),
        "",
        "the right pane path field is untouched"
    );
}

#[test]
fn test_ghost_completion_works_in_the_right_pane_path_field() {
    let tmp = tempfile::tempdir().expect("temp dir");
    fs::create_dir(tmp.path().join("project-alpha")).expect("create dir");

    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], tmp.path().to_string_lossy().to_string());
    dialog.right_pane_tool_index = 1;
    dialog.focused_field = fields(&dialog).right_pane_path;
    dialog.right_pane_path = PathField::new(format!("{}/pro", tmp.path().display()));
    dialog.right_pane_path.recompute_ghost();
    assert_eq!(dialog.right_pane_path.ghost_text(), Some("ject-alpha/"));

    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(
        dialog.right_pane_path.value(),
        format!("{}/project-alpha/", tmp.path().display())
    );
}

// --- One confirmation covers every missing directory ---

#[test]
fn test_both_missing_directories_are_confirmed_together() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let session_dir = tmp.path().join("session");
    let pane_dir = tmp.path().join("pane");

    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], session_dir.to_string_lossy().to_string());
    dialog.right_pane_tool_index = 1;
    dialog.right_pane_path = PathField::new(pane_dir.to_string_lossy().to_string());

    let result = dialog.handle_key(key(KeyCode::Enter));
    assert!(matches!(result, DialogResult::Continue));
    assert_eq!(
        confirm_dirs(&dialog),
        vec![
            session_dir.to_string_lossy().to_string(),
            pane_dir.to_string_lossy().to_string(),
        ]
    );

    let result = dialog.handle_key(key(KeyCode::Char('y')));
    assert!(matches!(result, DialogResult::Submit(_)));
    assert!(session_dir.exists());
    assert!(pane_dir.exists());
}

#[test]
fn test_only_the_missing_directory_is_named() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let pane_dir = tmp.path().join("pane");

    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], tmp.path().to_string_lossy().to_string());
    dialog.right_pane_tool_index = 1;
    dialog.right_pane_path = PathField::new(pane_dir.to_string_lossy().to_string());

    dialog.handle_key(key(KeyCode::Enter));
    assert_eq!(
        confirm_dirs(&dialog),
        vec![pane_dir.to_string_lossy().to_string()],
        "an existing session path is not named"
    );
}

#[test]
fn test_declining_creates_no_directory_at_all() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let session_dir = tmp.path().join("session");
    let pane_dir = tmp.path().join("pane");

    let mut dialog =
        NewSessionDialog::new_with_tools(vec!["claude"], session_dir.to_string_lossy().to_string());
    dialog.right_pane_tool_index = 1;
    dialog.right_pane_path = PathField::new(pane_dir.to_string_lossy().to_string());

    dialog.handle_key(key(KeyCode::Enter));
    let result = dialog.handle_key(key(KeyCode::Char('n')));

    assert!(matches!(result, DialogResult::Continue));
    assert!(dialog.confirm_create_dirs.is_none());
    assert!(!session_dir.exists(), "declining creates nothing");
    assert!(!pane_dir.exists(), "not even the first of the two");
}

#[test]
fn test_focus_follows_the_sandbox_checkbox_when_toggling_moves_it() {
    // Sandboxing hides two conditional fields above the checkbox, so the
    // checkbox's own index changes as it is toggled. Focus that stayed on the
    // old index would silently land on a different field.
    let mut dialog = multi_tool_dialog();
    dialog.docker_available = true;
    dialog.right_pane_tool_index = 1;
    dialog.focused_field = fields(&dialog).sandbox;

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.sandbox_enabled);
    assert_eq!(dialog.focused_field, fields(&dialog).sandbox);

    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(!dialog.sandbox_enabled);
    assert_eq!(dialog.focused_field, fields(&dialog).sandbox);
}

// --- create_dir_tracked tests ---

/// A dangling symlink reads as missing to `Path::exists` (which follows the
/// link), so the dialog offers to create it; `create_dir` then reports
/// AlreadyExists for the link itself. Treating that as success submits a path
/// no pane can start in.
#[test]
#[cfg(unix)]
fn a_dangling_symlink_is_not_mistaken_for_a_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let link = tmp.path().join("dangling");
    std::os::unix::fs::symlink(tmp.path().join("no-such-target"), &link).unwrap();

    assert!(!link.exists(), "precondition: it reads as missing");

    let mut owned = Vec::new();
    let err = super::create_dir_tracked(&link.to_string_lossy(), &mut owned)
        .expect_err("a dangling symlink is not a usable directory");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(
        owned.is_empty(),
        "nothing was created, nothing to roll back"
    );
}

/// The same shape with a regular file, which is what a concurrent writer can
/// leave at the final component while the confirmation is on screen.
#[test]
fn a_regular_file_at_the_final_component_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("parent").join("leaf");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"not a directory").unwrap();

    let mut owned = Vec::new();
    let err = super::create_dir_tracked(&target.to_string_lossy(), &mut owned)
        .expect_err("a regular file is not a usable directory");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
}

/// Every level this call made is tracked, not just the one that was asked for,
/// so a rollback undoes the parents it created too.
#[test]
fn every_created_level_is_tracked_for_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    let nested = tmp.path().join("a").join("b").join("c");

    let mut owned = Vec::new();
    super::create_dir_tracked(&nested.to_string_lossy(), &mut owned).unwrap();

    assert_eq!(owned.len(), 3, "a, b and c were all created: {owned:?}");
    assert_eq!(owned.last().unwrap(), &nested);
    assert!(
        !owned.iter().any(|p| p == tmp.path()),
        "the pre-existing temp dir is not ours to roll back"
    );
}

/// An existing directory is not ours, so it must not enter the rollback list.
#[test]
fn a_pre_existing_directory_is_not_tracked() {
    let tmp = tempfile::tempdir().unwrap();
    let mut owned = Vec::new();
    super::create_dir_tracked(&tmp.path().to_string_lossy(), &mut owned).unwrap();
    assert!(owned.is_empty());
}

#[test]
fn an_empty_path_is_refused() {
    let mut owned = Vec::new();
    let err = super::create_dir_tracked("", &mut owned).expect_err("empty path");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

/// The mirror of the dangling-symlink case: a link that does resolve to a
/// directory is a usable directory, and is not ours to roll back.
#[test]
#[cfg(unix)]
fn a_symlink_to_a_real_directory_is_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("real");
    std::fs::create_dir(&target).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let mut owned = Vec::new();
    super::create_dir_tracked(&link.to_string_lossy(), &mut owned).unwrap();
    assert!(owned.is_empty(), "nothing was created");
}

/// The parents created on the way to a component that turns out to be
/// unusable must not survive: the helper reports the failure and the caller
/// undoes exactly what was made.
#[test]
#[cfg(unix)]
fn parents_created_before_a_failure_are_rolled_back() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("a").join("b");
    let leaf = parent.join("dangling");
    std::fs::create_dir_all(&parent).unwrap();
    std::os::unix::fs::symlink(tmp.path().join("no-such-target"), &leaf).unwrap();
    std::fs::remove_dir_all(tmp.path().join("a")).ok();

    // Rebuild with only the dangling leaf present under a fresh tree.
    std::fs::create_dir_all(&parent).unwrap();
    std::os::unix::fs::symlink(tmp.path().join("no-such-target"), &leaf).unwrap();
    let nested = leaf.join("deeper");

    let mut owned = Vec::new();
    let err = super::create_dir_tracked(&nested.to_string_lossy(), &mut owned)
        .expect_err("the dangling component is not a directory");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);

    // What the caller does with `owned`: undo it, outermost last.
    for done in owned.iter().rev() {
        std::fs::remove_dir(done).unwrap();
    }
    assert!(!nested.exists(), "nothing this call made survives");
}
