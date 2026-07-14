# Capability Spec: Group Rename

## Purpose

Supports renaming a group from the home view, editing its full path, cascading the rename to child groups and session `group_path`s, migrating group metadata, and handling path conflicts with a merge confirmation.
## Requirements
### Requirement: Group rename dialog opens on 'r' key
When a group is selected in the home view and the user presses `e`, AoE SHALL open a GroupRenameDialog with a single text input pre-filled with the group's full path. The `r` key no longer opens the group rename dialog (it is now the fresh-restart action).

#### Scenario: Open rename dialog for a group
- **WHEN** a group is selected in the home view
- **AND** the user presses `e`
- **THEN** AoE SHALL open a GroupRenameDialog
- **AND** the text input SHALL be pre-filled with the group's current full path

#### Scenario: Dialog does not open without group selection
- **WHEN** no group is selected (a session is selected or nothing is selected)
- **AND** the user presses `e`
- **THEN** AoE SHALL NOT open the GroupRenameDialog

#### Scenario: r no longer opens the group rename dialog
- **WHEN** a group is selected in the home view
- **AND** the user presses `r`
- **THEN** AoE SHALL NOT open the GroupRenameDialog

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

### Requirement: Path validation on confirm
AoE SHALL validate the new path when the user confirms the rename dialog. Invalid paths SHALL be rejected with feedback.

#### Scenario: Empty path rejected
- **WHEN** the user clears the input field and confirms
- **THEN** AoE SHALL NOT apply the rename
- **AND** AoE SHALL display a validation error in the dialog

#### Scenario: Path with invalid characters rejected
- **WHEN** the user enters a path with leading/trailing slashes or consecutive slashes
- **THEN** AoE SHALL NOT apply the rename
- **AND** AoE SHALL display a validation error in the dialog

#### Scenario: Unchanged path closes dialog
- **WHEN** the user confirms without modifying the path
- **THEN** AoE SHALL close the dialog without making any changes

### Requirement: Cascading rename updates child groups
When a group is renamed, all descendant groups SHALL have their paths updated to reflect the new prefix.

#### Scenario: Rename parent cascades to children
- **WHEN** group `work` is renamed to `projects`
- **AND** child groups `work/frontend` and `work/backend` exist
- **THEN** AoE SHALL update child paths to `projects/frontend` and `projects/backend`

#### Scenario: Deep nesting cascades correctly
- **WHEN** group `a/b` is renamed to `x/y`
- **AND** descendant group `a/b/c/d` exists
- **THEN** AoE SHALL update it to `x/y/c/d`

### Requirement: Cascading rename updates session group_paths
When a group is renamed, all sessions whose `group_path` matches or is a descendant of the old path SHALL have their `group_path` updated.

#### Scenario: Sessions in renamed group are updated
- **WHEN** group `work/frontend` is renamed to `work/ui`
- **AND** sessions exist with `group_path = "work/frontend"`
- **THEN** those sessions SHALL have `group_path` updated to `work/ui`

#### Scenario: Sessions in descendant groups are updated
- **WHEN** group `work` is renamed to `projects`
- **AND** a session exists with `group_path = "work/frontend/react"`
- **THEN** that session SHALL have `group_path` updated to `projects/frontend/react`

### Requirement: Group metadata migrated on rename
When a group is renamed, its metadata (collapsed state, default_directory) SHALL be migrated to the new path. Descendant group metadata SHALL also be migrated.

#### Scenario: Collapsed state preserved after rename
- **WHEN** group `work` has `collapsed = true`
- **AND** it is renamed to `projects`
- **THEN** group `projects` SHALL have `collapsed = true`

#### Scenario: Default directory preserved after rename
- **WHEN** group `work/frontend` has `default_directory = "/home/user/frontend"`
- **AND** it is renamed to `work/ui`
- **THEN** group `work/ui` SHALL have `default_directory = "/home/user/frontend"`

### Requirement: Merge confirmation on path conflict
When the target path of a rename already exists as a group, AoE SHALL show a confirmation dialog asking whether to merge.

#### Scenario: Conflict triggers merge confirmation
- **WHEN** the user renames group `temp/api` to `work/api`
- **AND** group `work/api` already exists
- **THEN** AoE SHALL show a ConfirmDialog asking whether to merge

#### Scenario: User accepts merge
- **WHEN** the merge confirmation is shown
- **AND** the user selects Yes
- **THEN** AoE SHALL merge the source group's children and sessions into the target group
- **AND** the target group's metadata (collapsed, default_directory) SHALL take priority

#### Scenario: User declines merge
- **WHEN** the merge confirmation is shown
- **AND** the user selects No
- **THEN** AoE SHALL cancel the rename operation entirely
- **AND** no groups or sessions SHALL be modified

### Requirement: Intermediate groups auto-created
When a rename introduces a new parent path that does not exist, intermediate groups SHALL be auto-created following the existing group creation convention.

#### Scenario: Rename to new nested path
- **WHEN** group `misc` is renamed to `work/tools/misc`
- **AND** groups `work/tools` does not exist
- **THEN** AoE SHALL auto-create group `work/tools` as an intermediate group

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

