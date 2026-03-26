## ADDED Requirements

### Requirement: Pane info cache stores all panes per session
The pane info cache SHALL store information for ALL panes in each AoE-managed tmux session, not just the lowest-indexed pane. Each `PaneInfo` entry SHALL include `pane_index` and `pane_id` fields.

#### Scenario: Session with multiple panes cached
- **WHEN** a tmux session has 3 panes (indices 0, 1, 2)
- **AND** `refresh_pane_info_cache()` is called
- **THEN** `get_all_cached_pane_infos(session_name)` SHALL return 3 `PaneInfo` entries

#### Scenario: Single-pane accessor returns agent pane
- **WHEN** `get_cached_pane_info(session_name)` is called (existing API)
- **THEN** it SHALL return the pane with the lowest index (preserving backwards compatibility)

### Requirement: Agent type detection from pane process
The system SHALL detect which agent type is running in a pane by inspecting process information. Detection SHALL follow this priority chain:

1. Match `pane_current_command` against known agent binary names
2. Match `pane_pid` process comm name against known agent binary names
3. Get foreground PID from `pane_pid` and match its comm name
4. Classify as shell if no agent detected

#### Scenario: Direct command match for Codex
- **WHEN** a pane has `pane_current_command` containing "codex"
- **THEN** the agent type SHALL be detected as "codex"

#### Scenario: Direct command match for Gemini
- **WHEN** a pane has `pane_current_command` equal to "gemini"
- **THEN** the agent type SHALL be detected as "gemini"

#### Scenario: Process name match for Claude Code
- **WHEN** a pane has `pane_current_command` that does not match any known agent
- **AND** `ps -o comm= -p <pane_pid>` returns "claude"
- **THEN** the agent type SHALL be detected as "claude"

#### Scenario: Foreground process detection for user-split pane
- **WHEN** a pane has `pane_pid` pointing to a shell process (zsh/bash)
- **AND** the foreground process of that shell is "claude"
- **THEN** the agent type SHALL be detected as "claude"

#### Scenario: Shell pane classification
- **WHEN** a pane's `pane_current_command` is a known shell (bash, zsh, fish, sh, dash, ksh, tcsh)
- **AND** no agent process is found in the foreground
- **THEN** the pane SHALL be classified as "shell"

#### Scenario: Unknown process classified as shell
- **WHEN** no detection step matches a known agent
- **THEN** the pane SHALL be classified as "shell" and excluded from status aggregation

### Requirement: Multi-pane status aggregation
The system SHALL aggregate statuses from all non-shell panes in a session using priority: Waiting > Running > Idle.

#### Scenario: One pane Waiting, one Running
- **WHEN** a session has pane 0 with status Running and pane 1 with status Waiting
- **THEN** the aggregated session status SHALL be Waiting

#### Scenario: One pane Running, one Idle
- **WHEN** a session has pane 0 with status Idle and pane 1 with status Running
- **THEN** the aggregated session status SHALL be Running

#### Scenario: All panes Idle
- **WHEN** a session has pane 0 with status Idle and pane 1 with status Idle
- **THEN** the aggregated session status SHALL be Idle

#### Scenario: Shell panes excluded from aggregation
- **WHEN** a session has pane 0 (claude, Idle), pane 1 (zsh, shell), pane 2 (codex, Running)
- **THEN** the aggregated status SHALL be Running (shell pane ignored)

#### Scenario: Only shell panes remain
- **WHEN** all non-shell panes have exited and only shell panes remain
- **THEN** the session status SHALL follow existing single-pane Error/shell detection rules

#### Scenario: Single-pane session unchanged
- **WHEN** a session has only one pane (the AoE-created agent pane)
- **THEN** status detection SHALL behave identically to the current implementation

### Requirement: Per-pane status detection uses correct agent function
For each non-shell pane, status detection SHALL use the `detect_status` function registered for the detected agent type, not the session's configured `instance.tool`.

#### Scenario: Session with mixed agents
- **WHEN** a session was created with `tool=claude` but pane 1 runs Codex
- **THEN** pane 0 SHALL use `detect_claude_status` and pane 1 SHALL use `detect_codex_status`

#### Scenario: Hook detection only for AoE agent pane
- **WHEN** pane 0 is the AoE-created agent pane with hook support
- **AND** pane 1 is a user-split pane running the same agent type
- **THEN** hook-based detection SHALL only apply to pane 0
- **AND** pane 1 SHALL use title/content-based detection

### Requirement: Acknowledged-waiting applies to aggregated status
The acknowledged-waiting mapping SHALL apply to the final aggregated session status, not to individual pane statuses.

#### Scenario: Session acknowledged with pane still Waiting
- **WHEN** a session's aggregated status is Waiting
- **AND** the session has been acknowledged
- **THEN** the displayed status SHALL be Idle (acknowledged mapping applied)

### Requirement: TUI status poller uses multi-pane aggregation
The TUI status poller SHALL detect status for all non-shell panes and use the aggregated result in `StatusUpdate` messages.

#### Scenario: Poller reports aggregated status
- **WHEN** the status poller polls a session with 2 agent panes
- **AND** pane 0 is Idle and pane 1 is Running
- **THEN** the `StatusUpdate` SHALL report status as Running

### Requirement: Notification monitor uses multi-pane aggregation
The notification monitor SHALL use the same multi-pane aggregation logic as the TUI status poller.

#### Scenario: Notification bar reflects multi-pane status
- **WHEN** the notification monitor detects pane 0 Idle and pane 1 Waiting in a session
- **THEN** the notification bar SHALL show the session with Waiting icon
