## MODIFIED Requirements

### Requirement: Session creation splits tmux window when right pane tool is selected
When the user selects a tool for the right pane, the session creation flow SHALL automatically split the tmux session window horizontally after the main session is created, and launch the selected tool in the new right pane.

The right pane's command for an agent tool SHALL be built by the same pane-command builder AoE uses to launch a session's tracked panes, as a non-primary pane. Being non-primary, it SHALL NOT carry the instance's own launch context -- its command override, pre-allocated session id, fork token, or identity key -- all of which describe the instance's own agent pane. Launch-context decoration that follows from the session's settings, including Cross Agent Team decoration for a supported agent, SHALL apply to it exactly as it applies to a tracked pane relaunched by AoE.

A right pane whose tool is `shell` SHALL continue to launch the user's login shell in the session's working directory. The `shell` registry entry names no launchable agent binary, and a shell pane is never captured into a slot.

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

#### Scenario: Cross Agent Team right pane is decorated like a tracked pane
- **WHEN** a Cross Agent Team session is created with a right pane tool that supports Cross Agent Team
- **THEN** the right pane's command SHALL carry that agent's Cross Agent Team decoration
- **AND** for `codex` that decoration SHALL include the pane pre-registration step and the app-server connection the primary pane uses

#### Scenario: Right pane does not inherit the instance's launch context
- **WHEN** a session whose instance carries a command override, a pre-allocated session id, or an identity key is created with a right pane tool
- **THEN** the right pane's command SHALL NOT contain any of them
- **AND** the right pane SHALL launch that tool's own binary

#### Scenario: Shell right pane keeps the user's shell
- **WHEN** the user submits the new session dialog with Right Pane set to "shell"
- **THEN** the right pane SHALL run the user's login shell
- **AND** it SHALL start in the session's working directory
