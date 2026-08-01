## MODIFIED Requirements

### Requirement: Session creation splits tmux window when right pane tool is selected
When the user selects a tool for the right pane, the session creation flow SHALL automatically split the tmux session window horizontally after the main session is created, and launch the selected tool in the new right pane. The right pane SHALL use the same working directory as the main session's `project_path`, regardless of AoE's own launch directory.

#### Scenario: Right pane tool creates horizontal split
- **WHEN** the user submits the new session dialog with Right Pane set to a tool (e.g., "claude")
- **THEN** after the main tmux session is created, the system SHALL execute `tmux split-window -h` targeting the session
- **AND** the right pane SHALL run the selected tool's binary command
- **AND** the right pane SHALL use the same working directory as the main session

#### Scenario: Shell right pane uses session working directory
- **WHEN** the user creates a new session with path set to `/some/project`
- **AND** the right pane tool is set to "shell"
- **THEN** the shell in the right pane SHALL start with its working directory set to `/some/project`
- **AND** running `pwd` in the right pane SHALL output `/some/project`

#### Scenario: Right pane working directory matches left pane after worktree resolution
- **WHEN** the user creates a new session with a worktree branch specified
- **AND** the right pane tool is set to "shell"
- **THEN** the right pane's working directory SHALL be the resolved worktree path (same as the left pane), not the original repository path

#### Scenario: Right pane command is wrapped to disable Ctrl-Z
- **WHEN** a right pane tool is launched
- **THEN** the tool command SHALL be wrapped with the same `stty susp undef` wrapper used for the main tool

#### Scenario: Right pane has remain-on-exit enabled
- **WHEN** a right pane is created
- **THEN** `remain-on-exit` SHALL be set to `on` at the pane level for the right pane
- **AND** this SHALL NOT affect the main (left) pane's remain-on-exit setting
