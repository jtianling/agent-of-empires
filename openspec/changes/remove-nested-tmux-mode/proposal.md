## Why

The nested tmux mode (running AoE inside an existing tmux session, detected by the `TMUX` env var) adds significant complexity to keybinding management, attach/detach flows, and session lifecycle hooks. It requires dual code paths for `switch-client` vs `attach-session`, a `client-session-changed` hook for dynamic rebinding, per-client origin profile/return session tracking via tmux options, and a `refresh-bindings` CLI subcommand. This complexity is a recurring source of bugs, yet the feature is unused. Removing it simplifies the codebase and eliminates an entire class of binding lifecycle issues.

## What Changes

- **BREAKING**: Remove nested tmux attach via `switch-client`. Simplify `attach_with_client()` to always use `attach-session`.
- **BREAKING**: Remove the `aoe tmux refresh-bindings` CLI subcommand (only used by the nested `client-session-changed` hook).
- Remove `setup_nested_detach_binding()`, `cleanup_nested_detach_binding()`, `apply_managed_session_bindings()`, and `refresh_bindings()` functions.
- Remove `store_client_attach_context()` and the `client-session-changed` hook setup.
- Remove nested-mode helper shell commands that read profile from tmux options: `detach_run_shell_cmd()`, `back_toggle_run_shell_cmd_from_option()`, `cycle_run_shell_cmd()`, `index_jump_run_shell_cmd_from_option()`.
- Remove `NESTED_DETACH_HOOK`, `AOE_ORIGIN_PROFILE_OPTION_PREFIX`, `AOE_RETURN_SESSION_OPTION_PREFIX` constants.
- Remove TMUX env var gating in the TUI (mouse mode save/restore, cleanup path selection, client name resolution branch).
- Simplify `attach_client_name` resolution to just `get_tty_name()`.
- Remove nested-specific e2e test and test helpers.
- Update CLAUDE.md/AGENTS.md to remove nested mode documentation.
- All existing keybindings continue working in non-nested mode: Ctrl+q, Ctrl+./Ctrl+, (cycling), Ctrl+b b (back toggle), Ctrl+b h/j/k/l (pane nav), Ctrl+b 1-9 (number jump), Ctrl+; (pane cycle).
- Per-client tracking (last detached session, previous session for back toggle) continues working via tty name fallback.

## Capabilities

### New Capabilities

_(none)_

### Modified Capabilities

- `nested-tmux-detach`: **Remove entirely.** All requirements in this spec are nested-only (switch-client return, client-session-changed hook, dynamic rebinding). The spec becomes obsolete.
- `session-management`: Remove nested mode branch from session attach (FR-003a about switch-client when TMUX is set). Remove mouse mode save/restore gated on TMUX (FR-003b outer-tmux mouse handling). Simplify attach to always use `attach-session`.
- `tui`: Remove FR-003a (nested detach binding behavior). Remove FR-003b (TMUX-gated mouse save/restore). Simplify attach flow to single code path.
- `session-back-toggle`: Remove references to `apply_managed_session_bindings()` nested override and `cleanup_nested_detach_binding()` from keybinding lifecycle. Simplify to only `setup_session_cycle_bindings()` and `cleanup_session_cycle_bindings()`.
- `root-key-session-cycle`: Remove nested mode override in `apply_managed_session_bindings()` and cleanup in `cleanup_nested_detach_binding()`. Simplify keybinding lifecycle to only setup/cleanup pair.
- `cli`: Remove `aoe tmux refresh-bindings` subcommand from command tree.

## Impact

- **Code**: Major changes to `src/tmux/utils.rs` (delete ~200+ lines of nested-specific functions). Moderate changes to `src/tmux/session.rs`, `src/tui/mod.rs`, `src/tui/app.rs`, `src/cli/tmux.rs`. Minor changes to tests.
- **Breaking**: Users who run AoE inside an existing tmux session will no longer get the switch-client behavior. They will need to run AoE from a non-tmux terminal.
- **Dependencies**: No external dependency changes.
- **Documentation**: CLAUDE.md nested mode section needs removal/simplification.
