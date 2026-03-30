## MODIFIED Requirements

### Requirement: Keybinding lifecycle for Ctrl+, and Ctrl+.
The `Ctrl+,` and `Ctrl+.` bindings SHALL follow a simplified lifecycle with only setup and cleanup:
- Set up in `setup_session_cycle_bindings()` with the profile hardcoded in the shell command
- Cleaned up in `cleanup_session_cycle_bindings()`

Additionally, `setup_session_cycle_bindings()` SHALL set up guarded `%` and `"` prefix-table bindings that pass the session's `@aoe_project_path` to `split-window` for AoE-managed sessions. `cleanup_session_cycle_bindings()` SHALL restore `%` and `"` to their tmux default behavior.

#### Scenario: Bindings set during session cycle setup
- **WHEN** `setup_session_cycle_bindings()` is called with a profile
- **THEN** `C-,` and `C-.` SHALL be bound in the root key table with session-guard logic and the profile hardcoded
- **AND** `%` SHALL be bound in the prefix table with an if-shell guard for AoE sessions
- **AND** `"` SHALL be bound in the prefix table with an if-shell guard for AoE sessions

#### Scenario: Bindings cleaned up on exit
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** `C-,` and `C-.` SHALL be unbound from the root key table
- **AND** `%` SHALL be restored to `split-window -h` in the prefix table
- **AND** `"` SHALL be restored to `split-window -v` in the prefix table
