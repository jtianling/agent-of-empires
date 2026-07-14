# agent-fresh-restart Specification

## Purpose
TBD - created by archiving change restart-session-fresh. Update Purpose after archive.
## Requirements
### Requirement: Fresh restart respawns every tracked pane without resume
Pressing `r` on a selected session SHALL restart every tracked agent pane of that instance with a fresh command that does NOT carry a resume flag, while preserving each pane's agent, cwd, and full launch context (YOLO flag, hook env). The action SHALL reuse the multi-pane fan-out used by resume, forcing the no-resume path for every pane.

#### Scenario: Fresh restart of a multi-pane session drops history
- **WHEN** the user presses `r` on a selected session with multiple tracked agent panes
- **THEN** the system SHALL, for each tracked pane, kill the pane process tree and respawn it with the pane's agent command
- **AND** the respawn command SHALL NOT include a `--resume`/`resume` flag or token
- **AND** each respawn command SHALL still carry the pane's launch context (YOLO flag and hook env) and run in the pane's recorded cwd
- **AND** the session layout and pane count SHALL be preserved

#### Scenario: Fresh restart sets Restarting status then Starting
- **WHEN** a fresh restart is initiated for a session
- **THEN** the instance status SHALL be `Restarting` during the fan-out
- **AND** the status SHALL transition to `Starting` once all panes have respawned

### Requirement: Fresh restart isolates per-pane failures
A pane whose fresh respawn fails (e.g. an unknown/unsafe agent name, or a tmux respawn error) SHALL be recorded as a per-pane error and SHALL NOT abort the fresh restart of its sibling panes.

#### Scenario: One pane fails, siblings still restart
- **WHEN** the user presses `r` on a session with multiple tracked panes
- **AND** one pane cannot be safely respawned
- **THEN** the system SHALL respawn the remaining panes fresh
- **AND** SHALL record the failing pane's error without aborting the others

### Requirement: Fresh single-pane fallback ignores any stored resume token
When the selected session has no tracked slots, the fresh restart SHALL respawn the single primary `@aoe_agent_pane` with a fresh command that does NOT consult or inject the instance's stored `resume_token`.

#### Scenario: Single-pane fresh restart does not reinject stored token
- **WHEN** the user presses `r` on a session that has no tracked slots
- **AND** the instance has a stored `resume_token`
- **THEN** the system SHALL respawn the primary agent pane with a fresh command
- **AND** the respawn command SHALL NOT include the stored resume token or a resume flag

### Requirement: Fresh restart honors the in-flight guard and Deleting no-op
A fresh restart SHALL be ignored while a restart is already in flight for the same instance, and SHALL be a no-op for a session whose status is `Deleting`.

#### Scenario: Duplicate r press during in-flight restart is ignored
- **WHEN** a fresh restart is already in flight for an instance
- **AND** the user presses `r` again on the same instance
- **THEN** the system SHALL ignore the second press

#### Scenario: r on a session being deleted is a no-op
- **WHEN** the user presses `r` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

