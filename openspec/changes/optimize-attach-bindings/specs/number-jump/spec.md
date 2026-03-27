## MODIFIED Requirements

### Requirement: Number jump keybinding lifecycle
The number jump tmux bindings (1-9 in prefix table, aoe-1 through aoe-9 key tables) SHALL follow the same lifecycle as existing n/p/h/j/k/l bindings: set up before attach, cleaned up on detach/exit.

All tmux binding commands (bind-key, set-option for profile tagging, unbind-key for cleanup) SHALL be batched into a single tmux invocation using `tmux source-file` with a temporary file, rather than individual subprocess calls.

#### Scenario: Bindings set up via single tmux invocation
- **WHEN** `setup_session_cycle_bindings()` is called
- **THEN** all bind-key commands (session cycling, number jump tables, pane navigation, back toggle, detach) SHALL be written to a temporary file
- **AND** `tmux source-file <tmpfile>` SHALL be called exactly once
- **AND** no individual `Command::new("tmux").args(["bind-key"...])` calls SHALL be made

#### Scenario: Profile tagging batched with bindings
- **WHEN** `setup_session_cycle_bindings()` is called
- **THEN** `set-option -t <session> @aoe_profile <profile>` commands for all sessions SHALL be included in the same source-file batch

#### Scenario: Cleanup uses batched unbind
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** all unbind-key commands SHALL be batched into a single `tmux source-file` invocation

#### Scenario: Bindings cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** keys 1-9 SHALL be unbound from the prefix table
- **AND** all aoe-N key table bindings SHALL be unbound
