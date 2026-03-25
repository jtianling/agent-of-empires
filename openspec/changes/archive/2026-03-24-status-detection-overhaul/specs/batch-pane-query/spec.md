## ADDED Requirements

### Requirement: Single batch pane info query per poll cycle
The status poller SHALL execute a single `tmux list-panes -a` command at the start of each poll cycle to collect pane information for all sessions. This replaces per-instance `display-message` calls for pane dead state, current command, pane title, and pane PID.

#### Scenario: Batch query populates pane info cache
- **WHEN** the status poller begins a new poll cycle
- **THEN** it SHALL execute one `tmux list-panes -a -F` command
- **AND** parse the output into a `PaneInfoCache` keyed by session name
- **AND** each entry SHALL contain: pane title, current command, dead flag, pane PID

#### Scenario: Instance reads pane info from cache
- **WHEN** `update_status()` needs pane dead state or current command
- **THEN** it SHALL read from the `PaneInfoCache` instead of spawning a tmux subprocess
- **AND** fall back to a direct tmux query only if the cache entry is missing

#### Scenario: Non-AoE sessions filtered from cache
- **WHEN** the batch query returns panes from non-AoE sessions
- **THEN** the cache SHALL only store entries for sessions whose name starts with the AoE prefix (`aoe_`)

### Requirement: Batch query includes agent pane targeting
For sessions with the `@aoe_agent_pane` user option, the batch query SHALL identify the correct agent pane. The `list-panes -a` output includes all panes per session; the cache SHALL prefer the pane matching `@aoe_agent_pane` if set, otherwise use the first pane.

#### Scenario: Multi-pane session with agent pane option
- **WHEN** a session has multiple panes
- **AND** the `@aoe_agent_pane` user option is set
- **THEN** the cache SHALL store pane info for the pane matching that option

#### Scenario: Single-pane session
- **WHEN** a session has exactly one pane
- **THEN** the cache SHALL store that pane's info regardless of `@aoe_agent_pane`
