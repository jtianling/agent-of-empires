## MODIFIED Requirements

### Requirement: Pane health checks target the stored agent pane
All pane health check functions (`is_pane_dead`, `is_pane_running_shell`, `get_pane_pid`) SHALL target the stored agent pane ID rather than the session's currently active pane. If no stored pane ID exists, the functions SHALL fall back to targeting the session name.

Additionally, the `Session` struct SHALL expose a `pane_count()` method that returns the number of panes in the session, and a `respawn_agent_pane(command)` method that respawns only the agent pane.

#### Scenario: Health check with user-created split panes
- **WHEN** a session has user-created split panes via tmux shortcuts
- **AND** the active pane is a user-created shell (not the agent pane)
- **AND** `is_pane_dead()` or `is_pane_running_shell()` is called
- **THEN** the check SHALL target the original agent pane, not the active pane
- **AND** the result SHALL reflect the agent pane's state

#### Scenario: Session survives detach from user-created pane
- **WHEN** a user creates a split pane inside an AoE-managed session
- **AND** the user detaches from the user-created pane (Ctrl+b d)
- **AND** the user re-enters the session from the AoE TUI
- **THEN** the session SHALL NOT be killed and recreated
- **AND** all user-created split panes SHALL be preserved

#### Scenario: Fallback for sessions without stored pane ID
- **WHEN** `is_pane_dead()` or `is_pane_running_shell()` is called on a session
- **AND** the session does not have a `@aoe_agent_pane` option (e.g. created before this change)
- **THEN** the functions SHALL fall back to the previous behavior of targeting the session name

#### Scenario: Agent pane health is correctly detected through splits
- **WHEN** the agent process exits or crashes in the original pane
- **AND** user-created split panes are still running shells
- **THEN** `is_pane_dead()` SHALL return true (or `is_pane_running_shell()` SHALL return true)
- **AND** AoE SHALL correctly detect the agent has exited

#### Scenario: Attach-time recovery prefers respawn for multi-pane sessions
- **WHEN** the agent pane is dead during attach
- **AND** the session has more than one pane
- **THEN** the system SHALL use `respawn-pane` instead of `kill-session`
- **AND** the session layout and user-created panes SHALL be preserved

#### Scenario: Attach-time recovery uses kill-session for single-pane sessions
- **WHEN** the agent pane is dead during attach
- **AND** the session has exactly one pane
- **THEN** the system SHALL use the existing `kill-session` + recreate flow
