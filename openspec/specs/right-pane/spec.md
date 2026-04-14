# right-pane Specification

## Purpose
TBD - created by archiving change new-session-right-pane. Update Purpose after archive.
## Requirements
### Requirement: New session dialog includes right pane tool selector
The new session dialog SHALL include a "Right Pane" field directly below the "Tool" field. This field SHALL always be visible regardless of the main tool selection (including when "shell" is selected). This field SHALL offer the same list of available tools as the main Tool field, prefixed with a "none" option. The default selection SHALL be "none".

#### Scenario: Right pane field displays with none selected
- **WHEN** the user opens the new session dialog
- **THEN** a "Right Pane" field SHALL appear below the "Tool" field
- **AND** the field SHALL show "none" as the selected value

#### Scenario: User cycles through right pane tool options
- **WHEN** the user focuses the "Right Pane" field
- **AND** presses Left or Right arrow keys
- **THEN** the selection SHALL cycle through "none" followed by all available tools (same list as the Tool field)

#### Scenario: Right pane none selection creates session without split
- **WHEN** the user submits the new session dialog with Right Pane set to "none"
- **THEN** the session SHALL be created identically to the current behavior (single pane, no split)

### Requirement: Session creation splits tmux window when right pane tool is selected
When the user selects a tool for the right pane, the session creation flow SHALL automatically split the tmux session window horizontally after the main session is created, and launch the selected tool in the new right pane.

#### Scenario: Right pane tool creates horizontal split
- **WHEN** the user submits the new session dialog with Right Pane set to a tool (e.g., "claude")
- **THEN** after the main tmux session is created, the system SHALL execute `tmux split-window -h` targeting the session
- **AND** the right pane SHALL run the selected tool's binary command
- **AND** the right pane SHALL use the same working directory as the main session

#### Scenario: Right pane command is wrapped to disable Ctrl-Z
- **WHEN** a right pane tool is launched
- **THEN** the tool command SHALL be wrapped with the same `stty susp undef` wrapper used for the main tool

#### Scenario: Right pane has remain-on-exit enabled
- **WHEN** a right pane is created
- **THEN** `remain-on-exit` SHALL be set to `on` at the pane level for the right pane
- **AND** this SHALL NOT affect the main (left) pane's remain-on-exit setting

### Requirement: Agent pane tracking remains correct after right pane split
The `@aoe_agent_pane` session option SHALL continue to point to the main (left) pane after the right pane split. Status detection, health checks, and detach behavior SHALL all target the left pane.

#### Scenario: Status detection targets left pane after split
- **WHEN** a session is created with a right pane tool
- **AND** the agent in the left pane is running
- **AND** `detect_status()` is called
- **THEN** the status SHALL be determined from the left pane content, not the right pane

#### Scenario: Detach from right pane returns to AoE correctly
- **WHEN** a user is viewing the right pane of a split session
- **AND** the user presses `Ctrl+b d` to detach (nested mode)
- **THEN** the user SHALL return to the AoE TUI
- **AND** the session SHALL NOT be killed or recreated on next attach

#### Scenario: Detach from left pane returns to AoE correctly
- **WHEN** a user is viewing the left pane of a split session
- **AND** the user presses `Ctrl+b d` to detach (nested mode)
- **THEN** the user SHALL return to the AoE TUI
- **AND** the session SHALL NOT be killed or recreated on next attach

### Requirement: Right pane works with sandboxed sessions
For sandboxed sessions, the right pane tool command SHALL be executed inside the container, using the same container exec wrapping as the main tool.

#### Scenario: Sandboxed session right pane runs inside container
- **WHEN** the user creates a sandboxed session with a right pane tool selected
- **THEN** the right pane command SHALL be wrapped with the container's `docker exec` invocation
- **AND** the right pane SHALL use the same container and working directory as the main pane

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

