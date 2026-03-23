## Context

The tmux status bar currently shows:
- status-left: raw tmux session name (#S) + key hints (Ctrl+q, Ctrl+b d, Ctrl+b 1-9)
- status-right: "aoe: Title | branch | sandbox | time"
- window-status (center): default tmux window list (e.g., "0:2.1.81*")

With number jump and global cycling in place, users frequently switch between sessions but cannot see which numbered session they are in or where they came from. The window list is noise since AoE sessions have a single window.

There is no "go back" keybinding -- users must remember the number of the session they were previously in.

## Goals / Non-Goals

**Goals:**
- Show session index number prominently in the status bar
- Show session title in the status bar (replacing raw tmux session name)
- Conditionally show "from: <title>" when a back target exists
- Hide the useless window list
- Add Ctrl+b b toggle between current and previous session
- Track previous session across all jump types (n/p, N/P, 1-9, b)

**Non-Goals:**
- Session history stack (only toggle between last two, not full history)
- Making the status bar layout user-configurable
- Changing TUI-side index display

## Decisions

### 1. Previous session tracking: per-client tmux global option

Store `@aoe_prev_session_{client_key}` as a global tmux option (same pattern as existing `@aoe_return_session_` and `@aoe_last_detached_session_`). The value is the tmux session name of the session the user was in before the last switch.

**Why per-client**: Multiple clients can be attached to the same tmux server. Each client has its own navigation history.

**Why global option**: Session-scoped options are tied to one session, but the "previous session" is a property of the client's navigation, not any specific session.

**Alternative considered**: tmux `switch-client -l` (built-in last session toggle). Rejected because it tracks ALL session switches including non-AoE sessions, and does not integrate with AoE's own session validation (checking if target still exists).

### 2. Index and from-title: per-session tmux options

Store `@aoe_index` and `@aoe_from_title` as session-scoped tmux options (set via `set-option -t <session>`). The status bar format references these via `#{@aoe_index}` and `#{@aoe_from_title}`.

**Why per-session**: Each session has its own index and its own "where I came from" context. When switching to a session, both values are set on the target session.

**Index calculation**: On every switch, compute the target session's 1-based position in `ordered_profile_session_names()` filtered by `tmux_session_exists()`. This matches the TUI display order.

### 3. Unified tracking in all switch paths

All switch functions (`switch_aoe_session`, `switch_aoe_session_by_index`, and new `switch_aoe_session_back`) record:
1. Current session name -> `@aoe_prev_session_{client}` (for Ctrl+b b)
2. Current session title -> target's `@aoe_from_title` (for status bar)
3. Target session index -> target's `@aoe_index` (for status bar)

Centralizing this in a helper function called after `switch_client_to_session()` ensures consistency.

### 4. Status bar layout

```
status-left:
  " <index> <title>  from: <from_title>  Ctrl+b d detach "
   ^green,bold  ^white  ^dim,conditional      ^dim hint

status-right:
  " <branch> | <sandbox> | HH:MM "
   ^cyan,cond  ^orange,cond  ^white

window-status-format: "" (hidden)
window-status-current-format: "" (hidden)
```

### 5. Ctrl+b b binding lifecycle

Follows the same pattern as n/p/N/P bindings:
- Non-nested: bound in `setup_session_cycle_bindings()` with hardcoded profile
- Nested: overridden in `apply_managed_session_bindings()` with profile-from-option
- Cleanup: unbound in `cleanup_session_cycle_bindings()` and `cleanup_nested_detach_binding()`

The `b` key runs `aoe tmux switch-session --back --profile <profile> --client-name <client>`.

## Risks / Trade-offs

- [Stale @aoe_index after session create/delete] -> Acceptable. Index is refreshed on next switch. No session create/delete from within a tmux session currently.
- [Ctrl+b b overrides tmux default send-prefix] -> Acceptable. AoE already overrides d/n/p/N/P/h/j/k/l/1-9. The `b` binding is only active in AoE-managed sessions and cleaned up on exit.
- [@aoe_from_title unset on first entry] -> Status bar conditionally hides the "from:" section. First entry from TUI has no "from" context, which is correct.
