## ADDED Requirements

### Requirement: Agent pane ID is stored on session creation
When an AoE-managed tmux session is created, the system SHALL capture the initial pane's `#{pane_id}` and store it as the session-level tmux option `@aoe_agent_pane`. This applies to all session types (agent, terminal, container terminal).

#### Scenario: Pane ID stored atomically with session creation
- **WHEN** `Session::create_with_size()` creates a new tmux session
- **THEN** the session SHALL have a `@aoe_agent_pane` option set to the pane ID of the initial pane (e.g. `%42`)
- **AND** the option SHALL be set atomically in the same tmux command chain as session creation

#### Scenario: Terminal session stores pane ID
- **WHEN** `TerminalSession::create()` creates a new tmux session
- **THEN** the session SHALL have a `@aoe_agent_pane` option set to the initial pane ID

#### Scenario: Container terminal session stores pane ID
- **WHEN** `ContainerTerminalSession::create()` creates a new tmux session
- **THEN** the session SHALL have a `@aoe_agent_pane` option set to the initial pane ID

### Requirement: Pane health checks target the stored agent pane
All pane health check functions (`is_pane_dead`, `is_pane_running_shell`, `get_pane_pid`) SHALL target the stored agent pane ID rather than the session's currently active pane. If no stored pane ID exists, the functions SHALL fall back to targeting the session name.

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
- **AND** AoE SHALL correctly detect the agent has exited and restart the session
