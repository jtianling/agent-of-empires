## 1. Extract PathGhostCompletion as shared component

- [x] 1.1 Create `src/tui/components/path_ghost.rs` with the `PathGhostCompletion` struct, `compute(&Input) -> Option<Self>` and `accept(self, &Input) -> Option<String>` methods, plus `ghost_text(&self) -> &str` accessor. Move `expand_tilde` (public), `path_completion_base` (private), and `char_to_byte_idx` (private) helper functions from `src/tui/dialogs/new_session/path_input.rs` into this module.
- [x] 1.2 Add `pub mod path_ghost;` to `src/tui/components/mod.rs` and export `PathGhostCompletion` and `expand_tilde`.
- [x] 1.3 Add unit tests for `PathGhostCompletion::compute()` and `accept()` in the new module (single match, multiple matches with common prefix, no match, tilde expansion, staleness check).

## 2. Refactor NewSessionDialog to use extracted PathGhostCompletion

- [x] 2.1 Update `src/tui/dialogs/new_session/path_input.rs`: remove the inline `PathGhostCompletion` struct, `expand_tilde`, `path_completion_base`, and `char_to_byte_idx`. Import them from `crate::tui::components::path_ghost`. Update `recompute_path_ghost()` to call `PathGhostCompletion::compute(&self.path)` and store the result. Update `accept_path_ghost()` to call `ghost.accept(&self.path)` and use the returned value.
- [x] 2.2 Update any imports of `expand_tilde` in `src/tui/dialogs/new_session/mod.rs` or sibling files to use the new path from `crate::tui::components::path_ghost`.
- [x] 2.3 Verify existing tests pass with `cargo test` -- no behavior change expected.

## 3. Define GroupRenameResult and update dialog return type

- [x] 3.1 Define `GroupRenameResult` struct in `src/tui/dialogs/group_rename.rs` with fields `pub new_path: String` and `pub directory: Option<String>`.
- [x] 3.2 Export `GroupRenameResult` from `src/tui/dialogs/mod.rs`.
- [x] 3.3 Change `GroupRenameDialog::handle_key` return type from `DialogResult<String>` to `DialogResult<GroupRenameResult>`. On submit, construct `GroupRenameResult` from both fields. Return `Cancel` when both path and directory are unchanged from their initial values.

## 4. Add directory field to GroupRenameDialog

- [x] 4.1 Add fields to `GroupRenameDialog`: `directory: Input`, `initial_directory: String`, `dir_ghost: Option<PathGhostCompletion>`, and `focused_field: GroupRenameField` enum (Path, Directory).
- [x] 4.2 Update `GroupRenameDialog::new()` to accept `current_directory: &str` parameter. Initialize `directory` input with this value. Set `initial_directory` to track whether the directory changed.
- [x] 4.3 Implement focus switching in `handle_key`: Tab, Up, and Down arrows toggle `focused_field` between Path and Directory (wrapping).
- [x] 4.4 When `focused_field` is Directory, dispatch character input to `directory` Input, recompute `dir_ghost` via `PathGhostCompletion::compute(&self.directory)`. Handle Right/End to accept ghost completion.
- [x] 4.5 When `focused_field` is Path, dispatch character input to `new_path` Input (existing behavior).

## 5. Update GroupRenameDialog rendering

- [x] 5.1 Increase dialog height from 9 to ~13 lines to accommodate the new field.
- [x] 5.2 Add layout constraints for the Directory label and input field row.
- [x] 5.3 Render the Directory field using `render_text_field_with_ghost()` with ghost text from `dir_ghost`. Show focused styling when `focused_field == Directory`.
- [x] 5.4 Update help text to mention Tab/arrows for field switching.

## 6. Update callers to handle GroupRenameResult

- [x] 6.1 Update `src/tui/home/input.rs`: change the `GroupRenameDialog` constructor call to pass `current_directory` (from `group_tree.get_default_directory(path)` with fallback to `self.launch_dir`). Update the `DialogResult::Submit(path)` match arm to destructure `GroupRenameResult` and call both `rename_selected_group` (if path changed) and directory update logic.
- [x] 6.2 Update `src/tui/home/operations.rs`: add a method or extend `rename_selected_group` to handle directory updates via `GroupTree::set_default_directory` (when `Some`) or clearing default_directory (when `None`). Handle the case where only the directory changed (no rename needed).
- [x] 6.3 Update existing unit tests in `group_rename.rs` for the new return type and new constructor signature.

## 7. Verify and finalize

- [x] 7.1 Run `cargo fmt` and `cargo clippy` to ensure code quality.
- [x] 7.2 Run `cargo test` to verify all existing and new tests pass.
- [x] 7.3 Manual TUI check: open GroupRenameDialog, verify both fields render, Tab/Up/Down switch focus, ghost completion works on directory field, submit applies both changes.
