## REMOVED Requirements

### Requirement: Detach key returns to parent session when inside managed session
**Reason**: Nested tmux mode is being removed entirely. This requirement only applies when AoE runs inside an existing tmux session (TMUX env var set), which is no longer supported. In non-nested mode, `Ctrl+b d` uses standard `detach-client` behavior, and `Ctrl+q` returns to the AoE TUI.
**Migration**: Run AoE from a non-tmux terminal. Use `Ctrl+q` to return to the AoE TUI from a managed session, and `Ctrl+b d` to detach from tmux entirely.

### Requirement: Session cycling via Ctrl+,/Ctrl+.
**Reason**: The scoped cycling requirement in this spec is nested-mode-specific (uses client-session-changed hook and profile-from-option lookup). Session cycling continues to work via `setup_session_cycle_bindings()` in non-nested mode, which is now the only mode. The cycling behavior is defined in the `root-key-session-cycle` spec.
**Migration**: No user action needed. Ctrl+,/Ctrl+. continue working in non-nested mode via the root-key-session-cycle capability.

### Requirement: Vi-style pane navigation via Ctrl+b h/j/k/l
**Reason**: The pane navigation requirement in this spec references nested-mode binding lifecycle (set when entering managed session via hook, removed when leaving). In non-nested mode, these bindings are set in `setup_session_cycle_bindings()` and cleaned up in `cleanup_session_cycle_bindings()`, which is already the case.
**Migration**: No user action needed. Ctrl+b h/j/k/l continue working via setup_session_cycle_bindings().
