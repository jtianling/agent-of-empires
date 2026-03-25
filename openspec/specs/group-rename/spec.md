## ADDED Requirements

### Requirement: Group rename dialog opens on 'r' key
When a group is selected in the home view and the user presses `r`, AoE SHALL open a GroupRenameDialog with a single text input pre-filled with the group's full path.

#### Scenario: Open rename dialog for a group
- **WHEN** a group is selected in the home view
- **AND** the user presses `r`
- **THEN** AoE SHALL open a GroupRenameDialog
- **AND** the text input SHALL be pre-filled with the group's current full path

#### Scenario: Dialog does not open without group selection
- **WHEN** no group is selected (a session is selected or nothing is selected)
- **AND** the user presses `r`
- **THEN** AoE SHALL NOT open the GroupRenameDialog

### Requirement: Group rename dialog allows full path editing
The GroupRenameDialog SHALL display a single text input field containing the full slash-delimited group path. The user can edit any part of the path.

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
