## Context

The `client-session-changed[99]` tmux hook is designed to dynamically rebind `Ctrl+b d/j/k` depending on whether the current session is AoE-managed or not. The hook command uses `if-shell` with complex shell commands embedded as arguments.

The hook command is constructed as:
```
if-shell "test" "bind-key d run-shell '<escaped_cmd>' ; ..." "bind-key d detach-client ; ..."
```

The `<escaped_cmd>` is produced by `shell_escape()` which wraps in single quotes. But the shell commands contain double-quote characters (`"#{client_name}"`, `"$target"`, etc.). In tmux's parser, double quotes inside a double-quoted argument always close the string -- single quotes have no nesting effect. This causes `set-hook` to fail with "syntax error".

Additionally, `Session::attach()` calls `switch-client` without `-c`, which in multi-client environments may switch the wrong client.

## Goals / Non-Goals

**Goals:**
- Make the `client-session-changed` hook install and fire correctly
- Ensure `Ctrl+b d` returns to the AoE TUI after session cycling in all environments
- Ensure `switch-client` targets the correct client in multi-client tmux setups

**Non-Goals:**
- Changing the session cycling logic (scoping, ordering, group filtering)
- Changing how `@aoe_return_session_` or `@aoe_last_detached_session_` options work
- Adding new user-facing features

## Decisions

### Decision 1: Delegate hook rebinding to `aoe` binary via `run-shell`

Instead of embedding complex shell commands in the `if-shell` true/false branches (which requires multi-level quoting that tmux's parser cannot handle), the hook will call a simple `run-shell` that invokes the `aoe` binary:

```
if-shell "test" "run-shell '<aoe_bin> tmux refresh-bindings --client-name #{client_name}'" "bind-key d detach-client ; unbind-key j ; unbind-key k"
```

The `aoe tmux refresh-bindings` subcommand will:
1. Determine the current session for the given client
2. Check if the session is AoE-managed (matches `^aoe(_|_term_|_cterm_)`)
3. If managed: call `tmux bind-key d/j/k` via `Command::new("tmux")` (bypassing tmux's parser)
4. If not managed: restore `d` to `detach-client` and unbind `j/k`

**Why not fix `shell_escape()`?** tmux's parser doesn't support any escaping for `"` inside `"..."` except `\"`. But `\"` would be unescaped when `if-shell` evaluates the true-command, and then the `"` would appear bare in the `bind-key run-shell` argument, causing the same problem at the next parsing level. Multi-level quoting in tmux is fundamentally broken for complex shell commands.

**Why delegation works:** `Command::new("tmux").args(["bind-key", "d", "run-shell", &cmd])` passes each argument as a separate OS process argument, completely bypassing tmux's internal parser. This is proven to work -- the initial `bind-key` calls already use this approach successfully.

### Decision 2: Keep the false-branch inline

The false-branch (`bind-key d detach-client ; unbind-key j ; unbind-key k`) contains no special characters and works correctly in tmux's parser. Keep it inline for simplicity. Only the true-branch needs delegation.

### Decision 3: Simplify the `if-shell` test

Replace the `display-message | grep` pattern with tmux's built-in format conditional:

```
if-shell -F "#{m:aoe_*,#{session_name}}" "true-cmd" "false-cmd"
```

This uses tmux's `#{m:pattern,string}` format to match the session name against a glob pattern, avoiding the shell pipeline entirely. However, we need to match three prefixes (`aoe_`, `aoe_term_`, `aoe_cterm_`). Since all three start with `aoe_`, a simple `#{m:aoe_*,#{session_name}}` glob suffices.

### Decision 4: Add `-c` to `switch-client` in attach paths

Pass `client_name` to `switch-client -c` in `Session::attach()`, `TerminalSession::attach()`, and `ContainerTerminalSession::attach()` to ensure the correct client is switched.

## Risks / Trade-offs

- **[Risk] `aoe` binary path changes**: The hook stores the path to `aoe` at hook-set time. If the binary is recompiled to a different path, the hook would fail. → Mitigation: `aoe_bin_path()` already handles this via `std::env::current_exe()`, same as current behavior.
- **[Risk] Hook fires before `refresh-bindings` completes**: The `run-shell` command runs asynchronously. → Mitigation: tmux processes hook commands synchronously within its event loop; the key rebinding happens before the user can press another key.
- **[Trade-off] Extra process spawn on each session change**: The hook now spawns `aoe tmux refresh-bindings` as a subprocess on every `client-session-changed` event. This is a lightweight CLI call that runs in <50ms. Acceptable for the reliability gain.
