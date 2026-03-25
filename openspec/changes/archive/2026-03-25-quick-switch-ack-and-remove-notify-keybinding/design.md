## Context

AoE's notification system has two independent mechanisms for acknowledging a Waiting session:
1. **Ctrl+b N** -- a dedicated keybinding that opens a sub-table, allowing the user to press a number to switch to and acknowledge a Waiting session.
2. **Quick-switch** -- Ctrl+b <num> Space (number jump), Ctrl+./Ctrl+, (root-key cycle), and Ctrl+b b (back toggle) all switch sessions but do NOT acknowledge Waiting status.

The ack mechanism uses a file-based signal: an instance_id is written to `ack-signal` in the app directory. The notification monitor's `take_ack_signal()` reads this file on each poll cycle and sets `acknowledged = true` on the matching `MonitorSessionState`, which causes the monitor to map Waiting to Idle for that session.

Currently, Ctrl+b N is the only path that writes to the ack signal file. Quick-switch paths do not, creating a UX gap where users switch to a session and still see it flagged as Waiting.

## Goals / Non-Goals

**Goals:**
- Integrate ack signal writing into all three quick-switch functions so switching to a Waiting session auto-acknowledges it.
- Remove the Ctrl+b N keybinding, its sub-table, hint text, and all related tmux session options (`@aoe_notify_target_*`, `@aoe_notify_instance_*`, `@aoe_notification_hint`).
- Simplify the notification monitor code by removing keybinding setup/cleanup from the monitor loop and shutdown path.

**Non-Goals:**
- Changing the notification bar display format (session entries, status icons, ordering).
- Changing the ack signal file mechanism or `take_ack_signal()` / `MonitorSessionState` logic.
- Adding ack to the TUI home-screen attach path (this is a separate code path and could be a follow-up).
- Persisting acknowledged state across TUI restarts.

## Decisions

### Decision 1: Resolve instance_id from the instances list using generate_name matching

The quick-switch functions already load instances from storage. To write the ack signal, we need the instance_id for the target session name. The approach is to find the instance whose `Session::generate_name(&instance.id, &instance.title)` matches the target session name.

**Alternative considered**: Parse the instance_id suffix from the session name string directly. Rejected because `generate_name` truncates the ID to 8 chars and the truncation logic could change, making direct parsing fragile.

**Alternative considered**: Store instance_id as a tmux session option and read it back. Rejected because it adds a tmux round-trip and the instances are already loaded in memory.

### Decision 2: Make ack_signal_path and write_ack_signal public utilities

Currently `ack_signal_path()` is private to `notification_monitor.rs`. To let `utils.rs` write the ack signal, we need to either make the path function public or extract a shared `write_ack_signal(instance_id)` helper. A `write_ack_signal` function in `notification_monitor.rs` (pub) keeps the signal file logic co-located with the reader.

**Alternative considered**: Moving signal logic to a shared module. Rejected as over-engineering for a single function call.

### Decision 3: Write ack signal after successful switch only

The ack signal is written only after `switch_client_to_session` succeeds (i.e., after the tmux `switch-client` command runs without error). This avoids acknowledging sessions the user did not actually switch to (e.g., if the target session was destroyed between resolution and switch).

### Decision 4: Remove all notification keybinding infrastructure in one pass

Rather than deprecating Ctrl+b N, remove it entirely. The `setup_notification_key_bindings()`, `cleanup_notification_key_bindings()`, `notification_binding_hint()`, related constants, and the `@aoe_notify_target_*` / `@aoe_notify_instance_*` / `@aoe_notification_hint` session option writes in `build_notification_session_updates()` are all removed. The `NOTIFICATION_HINT_OPTION` unset calls in the cleanup/shutdown paths are also removed.

## Risks / Trade-offs

- **[Users who relied on Ctrl+b N]** Users familiar with Ctrl+b N will find it gone. Since quick-switch already provides the same navigation and now also acknowledges, functionality is not lost, but muscle memory may need adjustment. Mitigation: This is a UX simplification; the notification bar still shows which sessions need attention and users can jump to them with existing keys.
- **[Race between switch and ack write]** If the switch succeeds but the ack signal write fails (disk full, permission error), the session remains unacknowledged until the next switch. Mitigation: The ack file write is a best-effort operation (same as the current Ctrl+b N path), and the signal is tiny (just an instance ID string).
- **[Instance_id resolution failure]** If the target session name cannot be matched to an instance (e.g., stale session from a previous profile), no ack is written. The switch still succeeds. Mitigation: This is a silent no-op, consistent with the current behavior where unresolvable sessions simply have no notification interaction.
