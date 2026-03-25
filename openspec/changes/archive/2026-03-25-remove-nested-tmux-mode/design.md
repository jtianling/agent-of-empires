## Context

AoE currently supports two tmux runtime modes:

1. **Non-nested mode** (`TMUX` env var unset): AoE launches tmux from a bare terminal, attaches via `attach-session`.
2. **Nested mode** (`TMUX` env var set): AoE runs inside an existing tmux session, attaches via `switch-client`, and installs a `client-session-changed` hook for dynamic rebinding.

The nested mode requires:
- A `switch-client` branch in `attach_with_client()` that stores origin profile/return session in tmux options
- `setup_nested_detach_binding()` that hooks `client-session-changed` to call `aoe tmux refresh-bindings`
- `apply_managed_session_bindings()` that re-reads profile from tmux options on every session switch
- Shell command variants (`*_from_option`) that extract the profile from tmux options at runtime
- Mouse mode save/restore when entering/leaving the TUI
- A `refresh-bindings` CLI subcommand

The non-nested mode is simpler: `setup_session_cycle_bindings(profile)` binds all keys with the profile hardcoded, `cleanup_session_cycle_bindings()` unbinds them, and `attach-session` handles attach/detach.

The nested mode is unused and is a recurring source of bugs due to the dual code path.

## Goals / Non-Goals

**Goals:**
- Remove all nested-mode-specific code paths to simplify the codebase
- Maintain all existing keybinding functionality in non-nested mode
- Preserve per-client session tracking (last detached session, previous session for back toggle) which works via tty name
- Remove ~200+ lines of nested-specific code from `src/tmux/utils.rs`
- Simplify the attach flow to a single `attach-session` code path
- Update specs and documentation to reflect the single-mode architecture

**Non-Goals:**
- Adding new features or keybindings
- Changing the behavior of any existing keybinding in non-nested mode
- Removing per-client tracking infrastructure (it works in non-nested mode via tty name)
- Modifying tmux session creation or status detection

## Decisions

### Decision 1: Delete nested functions entirely rather than feature-flagging

Delete all nested-specific functions (`setup_nested_detach_binding`, `cleanup_nested_detach_binding`, `apply_managed_session_bindings`, `refresh_bindings`, `store_client_attach_context`, and the `*_from_option` shell command builders) rather than hiding them behind a feature flag.

**Rationale**: The nested mode is confirmed unused. Feature-flagging would preserve the complexity without benefit. Dead code should be removed.

**Alternative considered**: Keep functions behind `#[cfg(feature = "nested-tmux")]`. Rejected because it still requires maintenance and test coverage for an unused feature.

### Decision 2: Simplify attach_with_client to always use attach-session

Remove the `if std::env::var("TMUX").is_ok()` branch in `attach_with_client()` that uses `switch-client`. The function becomes a straight `attach-session` call.

**Rationale**: With nested mode removed, there is only one attach path. The `switch-client` path required storing origin profile and return session in tmux options, which is unnecessary complexity.

### Decision 3: Keep per-client tracking functions with tty name resolution

Keep `client_context_option_key()`, `resolve_client_name()`, `sanitize_tmux_option_suffix()`, and the per-client session tracking functions (`set/take_last_detached_session`, `set/clear/get_previous_session`). These work in non-nested mode using the tty name as the client identifier.

**Rationale**: These functions support TUI selection restoration and back-toggle tracking, which are valuable features in non-nested mode. The tty name fallback in `resolve_client_name()` is the primary code path after this change.

### Decision 4: Simplify attach_client_name to just get_tty_name()

In `src/tui/app.rs`, the `attach_client_name` logic currently checks `TMUX` env var to resolve the client name differently in nested mode. After removal, it simplifies to just calling `get_tty_name()`.

**Rationale**: The TMUX branch used `tmux display-message -p '#{client_name}'` to get the client name in nested mode. Without nested mode, the tty name is the only client identifier needed.

### Decision 5: Remove TMUX-gated mouse mode save/restore

The mouse mode save/restore in `src/tui/mod.rs` that checks `TMUX` env var is only relevant when running inside an existing tmux session (to avoid breaking the outer session's mouse settings). Remove this entirely.

**Rationale**: In non-nested mode, AoE owns the tmux server. There is no outer tmux session whose mouse settings need preservation.

### Decision 6: Remove the refresh-bindings CLI subcommand

Delete the `refresh-bindings` variant from the tmux CLI subcommands and its handler. This subcommand was only called by the `client-session-changed` hook in nested mode.

**Rationale**: No other code path calls `refresh-bindings`. It exists solely for the nested hook.

## Risks / Trade-offs

- **[Breaking change for nested users]** Any user who runs AoE inside an existing tmux session will lose the switch-client behavior. They will need to run AoE from a non-tmux terminal. **Mitigation**: User has confirmed this is acceptable and backwards compatibility is not a concern.

- **[Accidental deletion of shared code]** Some functions are used by both nested and non-nested modes. Care must be taken to only delete functions that are exclusively used in nested mode. **Mitigation**: The exploration context clearly identifies which functions to keep vs delete. The `setup_session_cycle_bindings()` and `cleanup_session_cycle_bindings()` functions, which serve non-nested mode, are explicitly kept.

- **[Spec references to nested mode]** Multiple specs reference nested mode behavior. If delta specs are not applied cleanly, stale spec text could cause confusion. **Mitigation**: Delta specs are created for all affected capabilities, removing or modifying nested-specific requirements.

- **[root_ctrl_q_run_shell_cmd usage]** This function is used in `setup_session_cycle_bindings()` for non-nested mode. It must NOT be deleted despite appearing related to nested code. **Mitigation**: Explicitly listed in the "keep" list in exploration context.
