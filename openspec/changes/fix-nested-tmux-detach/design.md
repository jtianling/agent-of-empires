## Context

aoe manages tmux sessions for each agent instance. When aoe itself is running inside an existing tmux session, attaching to a managed session uses `switch-client` to move the tmux client from the aoe TUI session to the managed session. The default tmux `Ctrl+b d` binding calls `detach-client`, which fully disconnects the client from all sessions, closing the terminal. There is no automatic way to "go back" to the aoe TUI session.

All aoe-managed sessions have a name prefix (`aoe_`, `aoe_term_`, `aoe_cterm_`). This is a stable, distinguishable identifier we can use at runtime.

## Goals / Non-Goals

**Goals:**
- When in a managed session (`aoe_*` prefix), `Ctrl+b d` switches back to the previous session instead of detaching the tmux client entirely.
- Normal detach behavior is preserved in non-aoe sessions (the outer session, custom user sessions, etc.).
- No change to user configuration files or persistent tmux config.

**Non-Goals:**
- Supporting custom tmux prefix keys (we assume the default `Ctrl+b` prefix).
- Restoring any pre-existing user `d` binding (if the user had already customized it, this change overrides it while in a managed session).
- Fixing behavior when aoe is NOT running inside tmux (no change needed in that case).

## Decisions

### Use a global `run-shell` key binding for `d`

**Decision**: After any `switch-client` call, run:
```
tmux bind-key d run-shell 'SESSION=$(tmux display-message -p "#{session_name}"); if echo "$SESSION" | grep -q "^aoe_"; then tmux switch-client -l; else tmux detach-client; fi'
```

**Why**: tmux key bindings are server-global (not per-session). A conditional `run-shell` binding that checks the current session name at keypress time is the only reliable per-session-type approach without patching tmux itself.

**Alternative: per-session hook to rebind `d`**: Setting `set-hook` on each managed session to bind/unbind as the client moves between sessions is more complex (requires two hooks per session: `client-attached` and `client-detached`) and is harder to reason about with multiple sessions open simultaneously.

**Alternative: `detach-on-destroy` option**: Only fires when a session is destroyed, not when a client manually detaches. Does not address the use case.

**Alternative: `switch-client -l` binding unconditionally**: Would cause the outer aoe TUI session to also "bounce back" to the managed session instead of detaching normally, breaking exit behavior.

### Set the binding in each `attach()` call

**Decision**: Call the binding setup in `Session::attach()`, `TerminalSession::attach()`, and `ContainerTerminalSession::attach()` after a successful `switch-client`.

**Why**: This is minimal-scope -- the binding is only configured when the code path that needs it executes. Running it once per attach is idempotent (same command each time).

**Alternative: Set binding at aoe startup**: Would require changes to the TUI/main entry point and adds coupling between startup logic and tmux session naming conventions.

### Extract to a shared helper

**Decision**: Add a free function `setup_nested_detach_binding()` in `src/tmux/session.rs`, and call it from all three `attach()` implementations.

**Why**: Avoids duplicating the binding string across three files. Since all three session types are in the same module area, a module-level helper is appropriate.

## Risks / Trade-offs

- **Risk: Overrides user's custom `d` binding** → Mitigation: The binding is only set when `switch-client` is actually called (i.e., aoe is actually inside tmux and the user opened a managed session). The override is idempotent and only changes behavior for `aoe_*`-prefixed sessions.
- **Risk: `switch-client -l` goes to an unexpected session if multiple sessions exist** → Mitigation: `switch-client -l` goes to the most recently used OTHER session, which in practice will be the aoe TUI session since that's where the user came from.
- **Risk: Shell script in `run-shell` is platform-specific** → The `grep` invocation used (`echo "$SESSION" | grep -q '^aoe_'`) is POSIX-compliant and works on macOS and Linux.

## Open Questions

None -- the approach is straightforward and the behavior is well-defined by tmux semantics.
