## Context

AoE creates one tmux session per agent instance with a single pane. Before re-attaching, AoE checks `is_pane_dead()` and `is_pane_running_shell()` to decide whether to kill and restart the session. These functions use `tmux display-message -t <session_name>`, which resolves to the session's **currently active pane**.

Users can split panes inside AoE-managed sessions using native tmux shortcuts. When they detach from a user-created shell pane, that pane becomes the active pane. On re-attach, AoE sees a shell in the active pane, misinterprets it as the agent having exited, and kills the entire session -- destroying all user splits.

## Goals / Non-Goals

**Goals:**
- Pane health checks always target the original agent pane, regardless of user-created splits
- User-created tmux panes survive detach/re-attach cycles
- No change to behavior when there are no user-created splits

**Non-Goals:**
- Managing or tracking user-created split panes
- Preventing users from splitting panes
- Persisting user split layouts across session restarts

## Decisions

### Decision 1: Store agent pane ID as a tmux session option

When creating a session, capture `#{pane_id}` (e.g. `%42`) from the new session's pane and store it as `@aoe_agent_pane` on the session. All subsequent pane health queries target this stored pane ID explicitly.

**Why this over alternatives:**
- **vs. always target `{session}:0.0`**: The first pane index could change if the user closes and reopens panes in creative ways. `#{pane_id}` is a globally unique, stable identifier assigned by the tmux server.
- **vs. storing in Instance struct on disk**: The pane ID is ephemeral (only valid for the tmux server's lifetime). Storing it in tmux options keeps ephemeral data with the ephemeral system.

### Decision 2: Query the stored pane ID with fallback to session name

When reading `@aoe_agent_pane`, if the option is missing (e.g., sessions created before this change), fall back to the current behavior (`-t session_name`). This avoids needing a migration for existing sessions.

### Decision 3: Capture pane ID atomically with session creation

Append a `set-option` command to the same tmux command chain that creates the session (similar to how `remain-on-exit` is already appended). This avoids a race where the pane ID could be read before the session is fully created.

## Risks / Trade-offs

- **[Risk] Pre-existing sessions lack `@aoe_agent_pane`** -> Fallback to session-name targeting preserves old behavior. These sessions will get the option set on next restart.
- **[Risk] Pane ID becomes stale after session restart** -> `kill()` destroys the entire session (including the option), and `create_with_size()` stores a fresh pane ID.
- **[Trade-off] Does not fix status polling for user panes** -> The status poller only checks the agent pane. User-created panes don't affect AoE status display. This is acceptable since AoE doesn't manage those panes.
