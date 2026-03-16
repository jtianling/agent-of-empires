## Why

The `client-session-changed` tmux hook that dynamically rebinds `Ctrl+b d/j/k` when switching between managed and non-managed sessions fails to install due to a quoting incompatibility: `shell_escape()` produces shell-style `'\''` escaping and embeds double-quote-containing commands inside a double-quoted tmux argument, causing `set-hook` to return "syntax error". This means the hook has **never worked**, and `Ctrl+b d` after cycling with `j/k` may fail to return to the AoE TUI.

## What Changes

- Replace the broken `if-shell`-based hook with a delegation approach: the hook calls `aoe tmux refresh-bindings` which sets bindings via `Command::new("tmux")` (bypassing tmux's internal parser)
- Add `aoe tmux refresh-bindings` CLI subcommand that checks the current session and sets d/j/k bindings accordingly
- Add `-c` (target client) to the `switch-client` call in `Session::attach()`, `TerminalSession::attach()`, and `ContainerTerminalSession::attach()` to ensure the correct client is switched in multi-client environments

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `nested-tmux-detach`: The implementation changes from embedding shell commands in `if-shell` to delegating to the `aoe` binary for key rebinding. No requirement-level changes -- existing scenarios remain the same.

## Impact

- `src/tmux/utils.rs`: Rewrite `setup_nested_detach_binding()` hook generation, add `refresh_bindings()` function
- `src/cli/tmux.rs`: Add `refresh-bindings` subcommand
- `src/tmux/session.rs`: Add `-c` flag to `switch-client` call
- `src/tmux/terminal_session.rs`: Add `-c` flag to `switch-client` calls (both TerminalSession and ContainerTerminalSession)
