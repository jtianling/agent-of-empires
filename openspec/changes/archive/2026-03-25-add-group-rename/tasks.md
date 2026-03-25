## 1. GroupTree rename logic

- [x] 1.1 Add `rename_group(old_path, new_path)` method to `GroupTree` in `src/session/groups.rs` that updates the target group path, cascades to all descendant group paths, and migrates metadata (collapsed state, default_directory)
- [x] 1.2 Handle merge case in `rename_group`: when target path exists, merge children and sessions from source into target, with target metadata taking priority
- [x] 1.3 Add path validation helper (reject empty, leading/trailing slashes, consecutive slashes)
- [x] 1.4 Add unit tests for `rename_group` covering: simple rename, cascading children, metadata migration, merge, and path validation

## 2. GroupRenameDialog

- [x] 2.1 Create `src/tui/dialogs/group_rename.rs` with `GroupRenameDialog` struct: single `Input` field pre-filled with current group path, returns `DialogResult<String>` with the new path
- [x] 2.2 Implement rendering: centered dialog with title "Rename Group", input field, and Submit/Cancel buttons using existing theme colors
- [x] 2.3 Implement key handling: Enter to confirm, Escape to cancel, character input forwarded to the Input widget
- [x] 2.4 Add path validation feedback in the dialog (show error text for invalid paths)
- [x] 2.5 Export the new dialog from `src/tui/dialogs/mod.rs`

## 3. HomeView integration

- [x] 3.1 Add `group_rename_dialog: Option<GroupRenameDialog>` field to `HomeView` in `src/tui/home/mod.rs`
- [x] 3.2 Add pending merge state fields: `pending_group_rename: Option<(String, String)>` (old_path, new_path) to HomeView
- [x] 3.3 Update `r` key handler in `src/tui/home/input.rs` to open `GroupRenameDialog` when a group is selected
- [x] 3.4 Add input handling for `group_rename_dialog` in the dialog priority chain in `handle_key`
- [x] 3.5 Add input handling for merge confirmation: on Yes complete the rename, on No cancel and clear pending state
- [x] 3.6 Add rendering of `GroupRenameDialog` in `src/tui/home/render.rs` in the dialog render order

## 4. Rename operation

- [x] 4.1 Add `rename_selected_group()` method in `src/tui/home/operations.rs` that: validates the new path, checks for conflicts, either applies rename directly or sets pending merge state and opens ConfirmDialog
- [x] 4.2 Implement session `group_path` cascading update: iterate all sessions and update paths matching or descending from old path
- [x] 4.3 Ensure intermediate groups are auto-created when the new path introduces parent groups that don't exist
- [x] 4.4 Persist changes to `groups.json` and session storage after rename

## 5. Tests

- [x] 5.1 Update `test_rename_dialog_not_opened_on_group` in `src/tui/home/tests.rs` to verify the GroupRenameDialog opens (or replace with a new test)
- [x] 5.2 Add unit test: rename dialog pre-fills with group path and returns new path on confirm
- [x] 5.3 Add unit test: merge confirmation shown when target path conflicts
- [x] 5.4 Add unit test: declining merge cancels operation with no side effects
- [x] 5.5 Run `cargo fmt`, `cargo clippy`, and `cargo test` to verify everything passes
