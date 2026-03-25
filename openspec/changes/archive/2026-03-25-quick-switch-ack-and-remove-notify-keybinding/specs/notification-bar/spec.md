## REMOVED Requirements

### Requirement: Maximum notification bindings increased to 8
**Reason**: The Ctrl+b N notification keybinding and its sub-table are removed. Notification bindings are no longer needed because quick-switch paths (number jump, root-key cycle, back toggle) now handle session switching and acknowledgment directly.
**Migration**: Users should use Ctrl+b <num> Space, Ctrl+./Ctrl+,, or Ctrl+b b to switch to and acknowledge Waiting sessions.

## MODIFIED Requirements

### Requirement: Background notification monitor daemon
A background daemon process SHALL poll session statuses and update each session's `@aoe_waiting` tmux user option. This enables real-time notification updates even when the TUI is blocked (user attached to a session). The monitor SHALL NOT set up or clean up any tmux keybindings. The monitor SHALL NOT write `@aoe_notification_hint`, `@aoe_notify_target_*`, or `@aoe_notify_instance_*` session options.

#### Scenario: Monitor spawned on session creation
- **WHEN** an AoE session is created or tmux options are applied
- **THEN** a notification monitor daemon is ensured to be running (spawned if not already active)

#### Scenario: Single instance enforcement
- **WHEN** a monitor is already running (PID tracked in tmux option)
- **THEN** a new monitor is NOT spawned

#### Scenario: Monitor updates waiting list
- **WHEN** the monitor polls and detects session B transitioned to Waiting
- **THEN** `@aoe_waiting` is updated on all other sessions to include `[N] B`

#### Scenario: Monitor exits when no sessions remain
- **WHEN** all AoE tmux sessions have been destroyed
- **THEN** the monitor process exits

#### Scenario: Monitor does not manage keybindings
- **WHEN** the monitor starts or stops
- **THEN** no tmux keybindings SHALL be set up or torn down by the monitor

#### Scenario: Monitor does not write notification hint option
- **WHEN** the monitor updates session options
- **THEN** the `@aoe_notification_hint` option SHALL NOT be set on any session

#### Scenario: Monitor does not write notify target/instance options
- **WHEN** the monitor updates session options
- **THEN** `@aoe_notify_target_*` and `@aoe_notify_instance_*` options SHALL NOT be set on any session

#### Scenario: Monitor cleanup does not unset hint option
- **WHEN** the monitor exits and cleans up session options
- **THEN** the `@aoe_notification_hint` option SHALL NOT be unset (it no longer exists)
