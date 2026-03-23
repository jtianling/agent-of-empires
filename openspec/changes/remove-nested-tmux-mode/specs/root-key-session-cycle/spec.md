## MODIFIED Requirements

### Requirement: Keybinding lifecycle for Ctrl+, and Ctrl+.
The `Ctrl+,` and `Ctrl+.` bindings SHALL follow a simplified lifecycle with only setup and cleanup:
- Set up in `setup_session_cycle_bindings()` with the profile hardcoded in the shell command
- Cleaned up in `cleanup_session_cycle_bindings()`

#### Scenario: Bindings set during session cycle setup
- **WHEN** `setup_session_cycle_bindings()` is called with a profile
- **THEN** `C-,` and `C-.` SHALL be bound in the root key table with session-guard logic and the profile hardcoded

#### Scenario: Bindings cleaned up on exit
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** `C-,` and `C-.` SHALL be unbound from the root key table

## REMOVED Requirements

### Requirement: Nested mode binding override
**Reason**: The nested mode override in `apply_managed_session_bindings()` (which re-bound `C-,` and `C-.` with profile-from-option lookup) is removed. This only applied when AoE ran inside an existing tmux session.
**Migration**: No user action needed. The bindings continue working via `setup_session_cycle_bindings()`.

### Requirement: Nested hook cleanup
**Reason**: The cleanup in `cleanup_nested_detach_binding()` (which included unbinding `C-,` and `C-.` when the `client-session-changed` hook fired for a non-managed session) is removed. The hook itself is removed.
**Migration**: No user action needed. Cleanup is handled by `cleanup_session_cycle_bindings()`.
