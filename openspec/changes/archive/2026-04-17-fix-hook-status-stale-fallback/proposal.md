## Why

Hook-based status detection (Claude / Cursor) shortcuts straight to the hook-written status without any freshness check or conflict arbitration. When Claude Code stops emitting events mid-turn (e.g. the user interrupts with Esc, switches to a client-side slash command, or the agent exits abnormally), `/tmp/aoe-hooks/<id>/status` stays at `running` forever and AoE shows the session as Running even though the pane is clearly at an idle prompt (`❯ `).

We observed a live case where the hook file had mtime > 3 hours old, content `running`, and all three Claude panes in the session were idle. The existing status-detection spec already hints at this (`hook status file exists **and is fresh**`) but the implementation does not enforce freshness at all.

## What Changes

- Track the mtime of `/tmp/aoe-hooks/<id>/status` at read time; treat the hook status as **fresh** only if mtime is within a freshness window.
- When the hook says `running` / `waiting` but the file is **stale**, do NOT short-circuit; fall through to content-based detection (the same path non-hook agents use) and use its result.
- Fresh hook statuses still short-circuit (preserves the speed advantage on hot paths).
- Stale hook state is treated as "no hook signal" rather than being blindly trusted or being cleared on disk (we do not rewrite the hook file from AoE).
- Freshness window is a compile-time constant in the hooks module (not user-configurable for now).

No breaking changes. Public APIs unchanged.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `status-detection`: Refine "Hook agent skips layers 6-10" to gate on hook-file freshness; add stale-hook fallback requirement so content-based detection runs when the hook file has not been updated within the freshness window.

## Impact

- Code: `src/hooks/status_file.rs` (expose mtime alongside status), `src/session/instance.rs` (`update_status_with_options`: check freshness, fall through on stale), `src/tmux/notification_monitor.rs` (shared hook-read path, same freshness treatment).
- Tests: unit tests in `src/hooks/status_file.rs` for mtime propagation; new tests in `src/session/instance.rs` covering stale-hook fallback to content-based Idle.
- No migration, no config change, no new dependency.
- Behavior change users may notice: a session that previously got "stuck Running" after Esc/interrupt will correctly flip to Idle once the freshness window elapses.
