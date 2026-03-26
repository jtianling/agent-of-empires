## ADDED Requirements

### Requirement: Notification monitor uses shared detection pipeline
The notification monitor SHALL use the same three-tier detection pipeline as the TUI status poller: hook-based status, pane title fast-path (from batch pane info cache), and content-based detection (via capture cache). The monitor SHALL NOT use its own separate `detect_live_status()` function with direct subprocess calls.

#### Scenario: Monitor detects status via shared pipeline
- **WHEN** the notification monitor polls a session's status
- **THEN** it SHALL first check hook-based status via `read_hook_status()`
- **AND** then check pane title from the `PaneInfoCache` (no per-session subprocess)
- **AND** then fall back to `capture_pane_cached()` for content-based detection
- **AND** SHALL NOT spawn individual `tmux list-panes -t <session>` or `tmux capture-pane -t <session>` subprocesses

#### Scenario: Monitor maintains per-session detection state
- **WHEN** the notification monitor runs across multiple poll cycles
- **THEN** it SHALL maintain a `MonitorSessionState` map in process memory
- **AND** track `last_status`, `last_window_activity`, `last_full_check`, and spike detection fields per session
- **AND** this state SHALL persist across poll cycles within the monitor's process lifetime

#### Scenario: Stale session state cleaned up
- **WHEN** a session that was previously tracked no longer appears in `list_aoe_sessions()`
- **THEN** the monitor SHALL remove its `MonitorSessionState` entry

### Requirement: Adaptive polling interval
The notification monitor SHALL adjust its poll interval based on the aggregate state of all monitored sessions.

#### Scenario: Any session Running uses fast interval
- **WHEN** at least one session has status Running after detection
- **THEN** the monitor SHALL sleep for 1 second before the next cycle

#### Scenario: Any session Waiting uses medium interval
- **WHEN** no session is Running
- **AND** at least one session has status Waiting
- **THEN** the monitor SHALL sleep for 2 seconds before the next cycle

#### Scenario: All sessions Idle uses slow interval
- **WHEN** all sessions are Idle (or Error/Stopped)
- **THEN** the monitor SHALL sleep for 3 seconds before the next cycle

### Requirement: Batched tmux option writes
The notification monitor SHALL write all per-session `@aoe_waiting` options in a single tmux invocation using `\;` command separators.

#### Scenario: Multiple sessions updated in one call
- **WHEN** the monitor has computed notification text for N sessions
- **THEN** it SHALL execute a single `tmux` command with N `set-option` subcommands joined by `\;`
- **AND** SHALL NOT spawn N separate `tmux set-option` subprocesses

#### Scenario: Batched write failure falls back to individual writes
- **WHEN** the batched tmux command fails
- **THEN** the monitor SHALL fall back to individual `set-option` calls per session
