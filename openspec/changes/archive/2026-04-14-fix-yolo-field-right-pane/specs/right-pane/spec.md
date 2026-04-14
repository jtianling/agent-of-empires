## ADDED Requirements

### Requirement: YOLO field visibility considers both pane tools
The new session dialog SHALL show the "Skip permission prompts" (YOLO mode) checkbox when either the left pane tool or the right pane tool is a code agent that supports opt-in YOLO mode. The checkbox SHALL be hidden only when neither pane has a tool that needs the YOLO option.

#### Scenario: Shell left pane with code agent right pane shows YOLO checkbox
- **WHEN** the user opens the new session dialog
- **AND** selects "shell" as the left pane tool
- **AND** selects a code agent (e.g., "claude") as the right pane tool
- **THEN** the "Skip permission prompts" checkbox SHALL be visible

#### Scenario: Code agent left pane with none right pane shows YOLO checkbox
- **WHEN** the user opens the new session dialog
- **AND** selects a code agent as the left pane tool
- **AND** the right pane is set to "none"
- **THEN** the "Skip permission prompts" checkbox SHALL be visible

#### Scenario: Shell left pane with none right pane hides YOLO checkbox
- **WHEN** the user opens the new session dialog
- **AND** selects "shell" as the left pane tool
- **AND** the right pane is set to "none"
- **THEN** the "Skip permission prompts" checkbox SHALL NOT be visible

#### Scenario: Shell left pane with shell right pane hides YOLO checkbox
- **WHEN** the user opens the new session dialog
- **AND** selects "shell" as the left pane tool
- **AND** selects "shell" as the right pane tool
- **THEN** the "Skip permission prompts" checkbox SHALL NOT be visible

#### Scenario: Changing right pane tool dynamically updates YOLO checkbox visibility
- **WHEN** the user has "shell" as the left pane tool
- **AND** changes the right pane from "none" to a code agent
- **THEN** the "Skip permission prompts" checkbox SHALL appear
- **AND** when changing the right pane back to "none"
- **THEN** the checkbox SHALL disappear
