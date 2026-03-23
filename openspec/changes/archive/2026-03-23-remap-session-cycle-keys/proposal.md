## Why

The current session cycling keybindings use the tmux prefix key (`Ctrl+b`) followed by `n/p` (within-group) and `N/P` (cross-group). This requires two separate keystrokes and occupies four prefix-table keys. Remapping global (cross-group) cycling to `Ctrl+,` and `Ctrl+.` (root-table chords) makes session switching faster (single keystroke, no prefix required) and frees up `n`, `p`, `N`, and `P` in the prefix table. The within-group cycling (`n/p`) is also removed since the global variant fully subsumes it -- all sessions are reachable in a single ordered list.

## What Changes

- **BREAKING**: Remove `Ctrl+b n` and `Ctrl+b p` bindings (within-group session cycling).
- **BREAKING**: Remove `Ctrl+b N` and `Ctrl+b P` bindings (cross-group session cycling).
- Add `Ctrl+,` (`C-,` in tmux notation, bound in the root table) to cycle to the previous session in global display order.
- Add `Ctrl+.` (`C-.` in tmux notation, bound in the root table) to cycle to the next session in global display order.
- Update all binding lifecycle functions: `setup_session_cycle_bindings()`, `apply_managed_session_bindings()`, `cleanup_session_cycle_bindings()`, `cleanup_nested_detach_binding()` (including the hook command string).
- Remove the `--global` flag from `aoe tmux switch-session` (all cycling is now global; the `--direction` flag alone is sufficient). Non-global (group-scoped) cycling code can be removed.
- Update existing specs that reference `n/p`/`N/P` keybindings (session-back-toggle, cross-group-cycling).

## Capabilities

### New Capabilities
- `root-key-session-cycle`: Bind `Ctrl+,` and `Ctrl+.` in the tmux root key table for previous/next global session cycling, replacing prefix-table `n/p/N/P` bindings.

### Modified Capabilities
- `session-back-toggle`: Scenarios and requirements reference `Ctrl+b n/p` and `Ctrl+b N/P` -- these must be updated to reflect the new `Ctrl+,`/`Ctrl+.` keybindings and the removal of group-scoped cycling.

## Impact

- **`src/tmux/utils.rs`**: All binding setup, cleanup, and hook functions. The `session_cycle_run_shell_cmds()` (group-scoped) function and `--global` flag handling can be removed; global cycling becomes the only mode. New root-table bindings for `C-,`/`C-.` added. The nested hook command string (the `unbind-key n ; unbind-key p ; unbind-key N ; unbind-key P` portion) must be updated.
- **`src/tmux/status_bar.rs`**: No direct impact (status bar does not show n/p/N/P hints currently), but the existing spec docs referencing those keys need updating.
- **`src/cli/tmux.rs`** (or wherever `switch-session` CLI args are defined): Remove `--global` flag; `--direction` always uses global scope.
- **`openspec/specs/session-back-toggle/spec.md`**: Scenarios referencing `Ctrl+b n`, `Ctrl+b N` need updating to `Ctrl+,`/`Ctrl+.`.
- **`openspec/changes/add-cross-group-session-cycling/`**: Existing spec references `Ctrl+b N/P` -- superseded by this change.
- **Welcome dialog** (`src/tui/dialogs/welcome.rs`): May want to add `Ctrl+,`/`Ctrl+.` hint if session cycling is documented there.
- **`AGENTS.md`/`CLAUDE.md`**: References to `Ctrl+b N/P` and `Ctrl+b n/p` must be updated.
