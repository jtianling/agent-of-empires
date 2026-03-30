## Context

When a user selects a session in the TUI, `App::attach_session()` calls `with_raw_mode_disabled()` which leaves the alternate screen, then calls `tmux_session.attach(profile)`. Inside `Session::attach()`, `setup_session_cycle_bindings(profile)` executes 120+ individual `Command::new("tmux")` subprocess spawns (9 base bindings + 108 number-jump key table bindings + N session tags) before the actual `tmux attach-session`. All of these run while the normal buffer (command line) is visible, causing a noticeable flash.

The tmux binding commands are server-side operations -- they don't interact with the terminal at all. They can execute while the TUI alternate screen is still displayed.

## Goals / Non-Goals

**Goals:**
- Eliminate visible command-line flash when attaching to a session from TUI
- Reduce subprocess overhead of tmux binding setup

**Non-Goals:**
- Changing which bindings are set up (count optimization is a separate change)
- Changing the `with_raw_mode_disabled` alternate screen behavior itself
- Modifying session cycling or number-jump behavior

## Decisions

### Decision 1: Move binding setup before `with_raw_mode_disabled`

Extract `setup_session_cycle_bindings(profile)` from `Session::attach()` and call it in `App::attach_session()` before the `with_raw_mode_disabled` block. `Session::attach()` becomes a minimal wrapper: check session exists, run `tmux attach-session`.

**Why**: The binding commands are tmux server operations that don't need raw mode disabled or alternate screen left. Moving them out means the entire 120+ command execution happens invisibly.

**Alternative**: Keep bindings inside `attach()` but skip `LeaveAlternateScreen`. Rejected because tmux expects a clean terminal state for `attach-session`, and the terminal state interaction between AoE's alternate screen and tmux's own alternate screen management is fragile.

### Decision 2: Batch tmux commands via `source-file`

Write all bind-key commands to a `NamedTempFile`, then invoke `tmux source-file <path>` once. The temp file is automatically cleaned up when dropped.

**Why**: Reduces 120+ process spawns to 1. Each `Command::new("tmux")` has fork+exec overhead (~2-5ms locally, more over SSH). Batching eliminates this entirely.

**Alternative**: Chain commands with `tmux cmd1 \; cmd2 \; ...`. Rejected because the `run-shell` arguments for number-jump contain complex shell commands with quotes/escapes. A temp file avoids shell escaping issues and command-line length limits.

### Decision 3: Also batch `tag_sessions_with_profile` and `cleanup_session_cycle_bindings`

Include `set-option` commands (for profile tagging) and `unbind-key` commands (for cleanup) in the same batching approach when they are called alongside binding setup.

**Why**: Same subprocess overhead applies. `tag_sessions_with_profile` adds N commands (one per session). Cleanup adds ~20 unbind commands. Batching these too keeps the approach consistent.

## Risks / Trade-offs

- **Temp file I/O**: Writing a temp file adds a filesystem operation. Mitigated: the file is small (<10KB) and written to the OS temp directory which is typically tmpfs/ramdisk.
- **Error visibility**: If a single bind-key fails in `source-file` mode, tmux reports it but continues processing. This is actually better than the current approach where individual command failures are silently ignored with `.ok()`.
- **Binding setup separated from attach**: If `attach-session` fails, bindings are already set up. This is harmless -- bindings are idempotent and will be cleaned up on TUI exit regardless.
