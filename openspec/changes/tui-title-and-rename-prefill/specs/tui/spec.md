## MODIFIED Requirements

### Requirement: Home screen title
The TUI home screen border title SHALL display `AoE [<profile>]` where `<profile>` is the active profile name.

#### Scenario: Default profile
- **WHEN** the user opens the TUI with the default profile
- **THEN** the home screen border title displays `AoE [default]`

#### Scenario: Custom profile
- **WHEN** the user opens the TUI with profile "work"
- **THEN** the home screen border title displays `AoE [work]`

### Requirement: Rename dialog pre-fills current title
The rename dialog SHALL initialize the "New title" input field with the session's current title so that users can edit in place.

#### Scenario: Opening rename dialog
- **WHEN** the user opens the rename dialog for a session with title "My Project"
- **THEN** the "New title" field SHALL contain "My Project"
- **AND** the cursor SHALL be positioned at the end of the pre-filled text

#### Scenario: Submitting without changes
- **WHEN** the user opens the rename dialog and submits without editing
- **THEN** the session title SHALL remain unchanged

#### Scenario: Clearing and entering new title
- **WHEN** the user clears the pre-filled title and types a new title
- **THEN** the session SHALL be renamed to the new title
