## MODIFIED Requirements

### Requirement: Back toggle keybinding lifecycle
The `Ctrl+b b` binding SHALL follow a simplified lifecycle with only setup and cleanup:
- Set up in `setup_session_cycle_bindings()` with the profile hardcoded in the shell command
- Cleaned up in `cleanup_session_cycle_bindings()`

#### Scenario: Binding set during session cycle setup
- **WHEN** `setup_session_cycle_bindings()` is called with a profile
- **THEN** key `b` SHALL be bound in the prefix table to execute the back-toggle command with the profile hardcoded

#### Scenario: Binding cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** key `b` SHALL be unbound from the prefix table

## REMOVED Requirements

### Requirement: Nested mode binding override and cleanup
**Reason**: The nested mode override in `apply_managed_session_bindings()` (which re-bound `b` with profile-from-option lookup) and the cleanup in `cleanup_nested_detach_binding()` are removed. These only applied when AoE ran inside an existing tmux session.
**Migration**: No user action needed. The `Ctrl+b b` binding continues working via `setup_session_cycle_bindings()`.
