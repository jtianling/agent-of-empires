## MODIFIED Requirements

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
