## MODIFIED Requirements

### Requirement: Session attach configures tmux key bindings
When attaching to any AoE-managed tmux session, the attach operation SHALL configure tmux key bindings for navigation: root-table `Ctrl+,` and `Ctrl+.` for session cycling, `Ctrl+b 1-9` for number jump via key tables, `Ctrl+b b` for back toggle, `Ctrl+b h/j/k/l` for pane navigation, and `Ctrl+;` for pane cycling. The `attach()` method accepts a `profile` parameter to scope session cycling and number jump to the current profile.

The binding setup SHALL be performed by the caller (TUI or CLI) before entering the raw-mode-disabled window, not inside `Session::attach()`. `Session::attach()` SHALL only execute `tmux attach-session`. The attach SHALL always use `attach-session` (never `switch-client`).

#### Scenario: Agent session attach sets bindings before raw mode change
- **WHEN** `App::attach_session()` is called from the TUI
- **THEN** `setup_session_cycle_bindings(profile)` SHALL be called while the TUI alternate screen is still visible
- **AND** `Session::attach()` SHALL only execute `tmux attach-session`
- **AND** no tmux binding commands SHALL execute after `LeaveAlternateScreen`

#### Scenario: Attach always uses attach-session regardless of environment
- **WHEN** `Session::attach(profile)` is called
- **AND** the `TMUX` env var may or may not be set
- **THEN** the system SHALL always use `tmux attach-session` (never `switch-client`)

#### Scenario: Number jump bindings cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** keys `1`-`9` SHALL be unbound from the prefix table
- **AND** all `aoe-N` key table bindings SHALL be unbound

#### Scenario: CLI attach still sets up bindings
- **WHEN** `aoe session attach` is called from the CLI
- **THEN** `setup_session_cycle_bindings(profile)` SHALL be called before `tmux attach-session`
