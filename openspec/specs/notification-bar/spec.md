## ADDED Requirements

### Requirement: Notification bar displays in tmux status-left
The tmux status bar for each AoE-managed session SHALL display a notification section after the "Ctrl+b d detach" hint, showing other sessions that are Waiting, Running, or Idle.

#### Scenario: Sessions are waiting
- **WHEN** one or more other AoE sessions are in Waiting status
- **THEN** the status bar shows ` | [index] ◐ title [index] ◐ title` after "Ctrl+b d detach"

#### Scenario: Sessions are running
- **WHEN** one or more other AoE sessions are in Running status and not in a collapsed group
- **THEN** the status bar shows those sessions with ● icon, e.g., `[index] ● title`

#### Scenario: No sessions need attention
- **WHEN** no other AoE sessions are Waiting, Running, or Idle (per visibility rules)
- **THEN** the notification section is empty and no ` | ` separator is shown

#### Scenario: Current session excluded
- **WHEN** session A is Waiting and the user is viewing session A's status bar
- **THEN** session A does NOT appear in its own notification bar

### Requirement: Idle sessions shown conditionally based on group collapse state
Idle sessions SHALL be shown in the notification bar unless they belong to a collapsed group.

#### Scenario: Idle session in expanded or no group
- **WHEN** a session is Idle and belongs to an expanded group or no group
- **THEN** it appears in the notification bar

#### Scenario: Idle session in collapsed group
- **WHEN** a session is Idle and belongs to a collapsed group
- **THEN** it does NOT appear in the notification bar

#### Scenario: Waiting session in collapsed group
- **WHEN** a session is Waiting and belongs to a collapsed group
- **THEN** it still appears in the notification bar (Waiting always shown)

### Requirement: Running sessions shown conditionally based on group collapse state
Running sessions SHALL be shown in the notification bar unless they belong to a collapsed group. This follows the same visibility rules as Idle sessions.

#### Scenario: Running session in expanded or no group
- **WHEN** a session is Running and belongs to an expanded group or no group
- **THEN** it appears in the notification bar with ● icon

#### Scenario: Running session in collapsed group
- **WHEN** a session is Running and belongs to a collapsed group
- **THEN** it does NOT appear in the notification bar

### Requirement: Running status icon is filled circle
Running sessions in the notification bar SHALL use ● (`U+25CF`) as their status icon, matching the TUI session list.

#### Scenario: Running session icon displayed
- **WHEN** a Running session appears in the notification bar
- **THEN** it is displayed as `[index] ● title`

### Requirement: Notification entries sorted by session index
Notification bar entries SHALL be sorted by session index (ascending), regardless of status.

#### Scenario: Mixed status sessions sorted by index
- **WHEN** sessions with index 2 (Running), 3 (Waiting), and 5 (Idle) are all visible
- **THEN** the notification shows `[2] ● run [3] ◐ wait [5] ○ idle` in index order

### Requirement: Notification format uses index and title
Each session in the notification bar SHALL be displayed as `[index] title` where index matches the session's `@aoe_index` used by `Ctrl+b <N>` jump keys.

#### Scenario: Multiple sessions displayed
- **WHEN** sessions with index 2 ("api") and index 5 ("frontend") are both Waiting
- **THEN** the notification shows `[2] api [5] frontend`

### Requirement: Notification text uses distinct color
The notification section SHALL use a visually distinct color (yellow/colour220) to differentiate from the dim hint text (colour245).

#### Scenario: Notification visible
- **WHEN** notification text is displayed
- **THEN** it renders in colour220 (yellow), contrasting with the colour245 (grey) of "Ctrl+b d detach"

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

### Requirement: Status-left-length increased
The tmux `status-left-length` SHALL be increased from 80 to 160 to accommodate notification text with multiple session entries.

#### Scenario: Long notification fits
- **WHEN** 4+ sessions are in the notification bar
- **THEN** the text is not truncated by tmux's status-left-length limit
