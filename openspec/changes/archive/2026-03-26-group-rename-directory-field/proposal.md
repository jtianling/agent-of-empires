## Why

The GroupRenameDialog currently only allows editing a group's path (name). There is no way to edit a group's `default_directory` from the TUI without creating a new session. Users who want to change a group's default project directory must either manually edit `groups.json` or create a throwaway session. Additionally, the `PathGhostCompletion` logic (filesystem path autocomplete with ghost text) is duplicated inside `NewSessionDialog` and cannot be reused by other dialogs.

## What Changes

- Extract `PathGhostCompletion` and its helpers (`expand_tilde`, `path_completion_base`, `char_to_byte_idx`, recompute/accept logic) from `NewSessionDialog` into a new shared component at `src/tui/components/path_ghost.rs`.
- Refactor `NewSessionDialog` to delegate to the extracted `PathGhostCompletion` component (pure refactor, no behavior change).
- Add a second input field ("Directory") to `GroupRenameDialog` with filesystem path ghost completion, pre-filled with the group's current `default_directory` (falling back to `launch_dir`).
- Add Tab/Up/Down focus switching between the Path and Directory fields in the dialog.
- **BREAKING**: Change `GroupRenameDialog::handle_key` return type from `DialogResult<String>` to `DialogResult<GroupRenameResult>`, where `GroupRenameResult` is a new struct containing both the new path and the optional new directory.
- Update all callers in `input.rs` and `operations.rs` to handle the new `GroupRenameResult` struct and apply directory changes via `GroupTree::set_default_directory`.

## Capabilities

### New Capabilities
- `path-ghost-completion`: Shared filesystem path autocomplete component with ghost text, extractable from NewSessionDialog and reusable across dialogs.

### Modified Capabilities
- `group-rename`: Add directory editing field to the rename dialog, change result type to `GroupRenameResult`, support focus switching between fields.

## Impact

- **Files modified**: `src/tui/components/mod.rs`, `src/tui/components/path_ghost.rs` (new), `src/tui/dialogs/new_session/path_input.rs`, `src/tui/dialogs/group_rename.rs`, `src/tui/dialogs/mod.rs`, `src/tui/home/input.rs`, `src/tui/home/operations.rs`
- **Breaking change**: `GroupRenameDialog::handle_key` return type changes from `DialogResult<String>` to `DialogResult<GroupRenameResult>`. All match sites on this return value must be updated.
- **No data format changes**: Uses existing `GroupTree::set_default_directory` API; no schema or migration needed.
- **No new dependencies**: All functionality uses existing crates (`dirs`, `tui_input`, `ratatui`).
