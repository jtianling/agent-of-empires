## MODIFIED Requirements

### Requirement: Session attach configures tmux key bindings
When attaching to any AoE-managed tmux session, the attach operation SHALL configure tmux key bindings for navigation: root-table `Ctrl+,` and `Ctrl+.` for session cycling, `Ctrl+b 1-9` for number jump via key tables, `Ctrl+b b` for back toggle, `Ctrl+b h/j/k/l` for pane navigation, and `Ctrl+;` for pane cycling. The `attach()` method accepts a `profile` parameter to scope session cycling and number jump to the current profile. The attach SHALL always use `attach-session` (never `switch-client`).

#### Scenario: Agent session attach sets bindings
- **WHEN** `Session::attach(profile)` is called
- **THEN** session cycling bindings (`Ctrl+,`/`Ctrl+.`) are configured scoped to the given profile
- **AND** number jump bindings (`1`-`9` with `aoe-1` through `aoe-9` key tables) are configured
- **AND** back toggle binding (`Ctrl+b b`) is configured
- **AND** pane navigation bindings (`Ctrl+b h/j/k/l`) are configured
- **AND** pane cycle binding (`Ctrl+;`) is configured
- **AND** the system uses `attach-session` to attach the terminal

#### Scenario: Attach always uses attach-session regardless of environment
- **WHEN** `Session::attach(profile)` is called
- **AND** the `TMUX` env var may or may not be set
- **THEN** the system SHALL always use `tmux attach-session` (never `switch-client`)

#### Scenario: Number jump bindings cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** keys `1`-`9` SHALL be unbound from the prefix table
- **AND** all `aoe-N` key table bindings SHALL be unbound

## REMOVED Requirements

### Requirement: Nested mode mouse save/restore
**Reason**: The requirement that AoE temporarily enables `mouse on` when running inside an existing tmux session (to ensure crossterm receives mouse events instead of tmux converting them) is removed. This was only relevant when AoE ran nested inside another tmux session. In non-nested mode, AoE owns the tmux server and there is no outer tmux session whose mouse settings need preservation. AoE-managed tmux sessions continue to have session-level `mouse on` enabled regardless.
**Migration**: No user action needed. Mouse behavior in AoE-managed sessions is unchanged.
