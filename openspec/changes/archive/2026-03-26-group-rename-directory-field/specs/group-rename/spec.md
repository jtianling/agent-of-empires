## MODIFIED Requirements

### Requirement: Group rename dialog allows full path editing
The GroupRenameDialog SHALL display two input fields: a Path field containing the full slash-delimited group path, and a Directory field for editing the group's default directory. The Path field SHALL be pre-filled with the group's current path. The Directory field SHALL be pre-filled with the group's `default_directory` if set, otherwise with the application's `launch_dir`. The user can edit either field.

#### Scenario: User modifies the last segment (rename)
- **WHEN** the dialog shows path `work/frontend`
- **AND** the user changes it to `work/backend`
- **AND** the user confirms
- **THEN** AoE SHALL rename the group from `work/frontend` to `work/backend`

#### Scenario: User modifies a parent segment (move)
- **WHEN** the dialog shows path `work/frontend`
- **AND** the user changes it to `personal/frontend`
- **AND** the user confirms
- **THEN** AoE SHALL move the group from `work/frontend` to `personal/frontend`

#### Scenario: User cancels the dialog
- **WHEN** the GroupRenameDialog is open
- **AND** the user presses Escape
- **THEN** AoE SHALL close the dialog without making any changes

#### Scenario: Directory field pre-filled with group default directory
- **WHEN** the GroupRenameDialog opens for a group with `default_directory` set to `/home/user/project`
- **THEN** the Directory field SHALL show `/home/user/project`

#### Scenario: Directory field pre-filled with launch_dir when no default directory
- **WHEN** the GroupRenameDialog opens for a group with no `default_directory`
- **AND** the application's launch directory is `/home/user`
- **THEN** the Directory field SHALL show `/home/user`

#### Scenario: User edits the directory field
- **WHEN** the user changes the Directory field to `/home/user/new-project`
- **AND** the user confirms
- **THEN** AoE SHALL set the group's `default_directory` to `/home/user/new-project`

#### Scenario: User clears the directory field
- **WHEN** the user clears the Directory field (empty string)
- **AND** the user confirms
- **THEN** AoE SHALL clear the group's `default_directory` (set to `None`)

#### Scenario: Directory field has filesystem path autocomplete
- **WHEN** the Directory field is focused
- **AND** the user types a partial filesystem path
- **THEN** the Directory field SHALL display ghost text with filesystem directory completions

#### Scenario: Accept ghost completion with Right arrow
- **WHEN** the Directory field is focused
- **AND** ghost text is displayed
- **AND** the cursor is at the end of the input
- **AND** the user presses the Right arrow key
- **THEN** AoE SHALL accept the ghost text into the input

#### Scenario: Accept ghost completion with End key
- **WHEN** the Directory field is focused
- **AND** ghost text is displayed
- **AND** the cursor is at the end of the input
- **AND** the user presses the End key
- **THEN** AoE SHALL accept the ghost text into the input

## ADDED Requirements

### Requirement: GroupRenameDialog supports focus switching between fields
The GroupRenameDialog SHALL support switching focus between the Path and Directory fields using Tab, Up, and Down arrow keys. Focus SHALL wrap around (e.g., Tab on Directory moves focus to Path).

#### Scenario: Tab switches focus from Path to Directory
- **WHEN** the Path field is focused
- **AND** the user presses Tab
- **THEN** focus SHALL move to the Directory field

#### Scenario: Tab switches focus from Directory to Path
- **WHEN** the Directory field is focused
- **AND** the user presses Tab
- **THEN** focus SHALL move to the Path field

#### Scenario: Down arrow switches focus from Path to Directory
- **WHEN** the Path field is focused
- **AND** the user presses Down arrow
- **THEN** focus SHALL move to the Directory field

#### Scenario: Up arrow switches focus from Directory to Path
- **WHEN** the Directory field is focused
- **AND** the user presses Up arrow
- **THEN** focus SHALL move to the Path field

#### Scenario: Focus wraps around with Down arrow
- **WHEN** the Directory field is focused
- **AND** the user presses Down arrow
- **THEN** focus SHALL move to the Path field

#### Scenario: Focus wraps around with Up arrow
- **WHEN** the Path field is focused
- **AND** the user presses Up arrow
- **THEN** focus SHALL move to the Directory field

### Requirement: GroupRenameDialog returns GroupRenameResult struct
The `GroupRenameDialog::handle_key` method SHALL return `DialogResult<GroupRenameResult>` where `GroupRenameResult` contains the new path and an optional directory. This replaces the previous `DialogResult<String>` return type.

#### Scenario: Submit returns both path and directory
- **WHEN** the user sets path to `work/backend` and directory to `/home/user/backend`
- **AND** the user confirms
- **THEN** `handle_key` SHALL return `DialogResult::Submit(GroupRenameResult { new_path: "work/backend", directory: Some("/home/user/backend") })`

#### Scenario: Submit with empty directory returns None directory
- **WHEN** the user sets path to `work/backend` and clears the directory field
- **AND** the user confirms
- **THEN** `handle_key` SHALL return `DialogResult::Submit(GroupRenameResult { new_path: "work/backend", directory: None })`

#### Scenario: Submit with unchanged path and changed directory
- **WHEN** the path is unchanged from the original
- **AND** the directory has been changed
- **AND** the user confirms
- **THEN** `handle_key` SHALL return `DialogResult::Submit` with the result (not cancel, since directory changed)

#### Scenario: Cancel when both path and directory unchanged
- **WHEN** the path is unchanged from the original
- **AND** the directory is unchanged from the original pre-filled value
- **AND** the user confirms
- **THEN** `handle_key` SHALL return `DialogResult::Cancel`

### Requirement: Callers handle GroupRenameResult for directory updates
When the GroupRenameDialog submits a `GroupRenameResult`, the caller SHALL apply both the path rename (if changed) and the directory update. If `directory` is `Some(path)`, the caller SHALL call `GroupTree::set_default_directory`. If `directory` is `None`, the caller SHALL clear the group's `default_directory`.

#### Scenario: Path renamed and directory updated
- **WHEN** `GroupRenameResult` has `new_path = "work/ui"` and `directory = Some("/home/user/ui")`
- **AND** the original group path was `work/frontend`
- **THEN** the caller SHALL rename the group from `work/frontend` to `work/ui`
- **AND** set the new group's `default_directory` to `/home/user/ui`

#### Scenario: Only directory updated (path unchanged)
- **WHEN** `GroupRenameResult` has `new_path` equal to the original path
- **AND** `directory = Some("/home/user/new-dir")`
- **THEN** the caller SHALL NOT perform a rename
- **AND** the caller SHALL set the group's `default_directory` to `/home/user/new-dir`

#### Scenario: Directory cleared
- **WHEN** `GroupRenameResult` has `directory = None`
- **THEN** the caller SHALL clear the group's `default_directory`
