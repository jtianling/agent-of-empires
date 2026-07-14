## Why

Today `R` resumes every tracked agent pane (replaying history) and `r` opens the rename/edit dialog. The pairing feels arbitrary, and there is no way to restart a session's agents fresh (drop history, keep the same agents/panes) short of manually killing and re-creating everything. Users want a clean "restart from scratch, same layout" action.

## What Changes

- Rebind the home-view keys so intent reads consistently:
  - `R` = **resume** (unchanged behavior: fan out to every tracked pane and resume each from its persisted `native_session_id`).
  - `r` = **restart fresh** (new): fan out to every tracked pane and respawn each in place with a fresh command (no `--resume`), preserving each pane's agent, cwd, and full launch context (YOLO flag, hook env).
  - `e` = **edit** (moved from `r`): open the rename/edit dialog for the selected session, and the group-rename dialog for a selected group.
- Implement the fresh restart action by reusing the existing multi-pane fan-out (`resume_all_tracked_panes` -> `resume_launch_pane` -> `build_pane_resume_plan`), parameterized with a restart mode that forces the "no resume flag" path for every pane. This is the already-tested `DegradedToFresh` launch (full launch context, no `--resume`), promoted from an internal fallback to a first-class user action.
- The single-pane fallback (no tracked slots) for `r` SHALL force a fresh respawn that does NOT inject the stored `resume_token` (the current single-pane path falls back to `self.resume_token`; the fresh path must bypass that).
- Update the help overlay text: `R` = "Resume agent panes", `r` = "Restart agent panes (fresh)", `e` = "Edit/rename session or group".
- **BREAKING** (keybinding): `r` no longer opens the rename dialog; rename/edit moves to `e`. Group rename also moves from `r` to `e` for consistency.

## Capabilities

### New Capabilities
- `agent-fresh-restart`: pressing `r` restarts every tracked agent pane of the selected session with a fresh command (no history resume), preserving each pane's agent, cwd, and launch context, with per-pane failure isolation and a fresh single-pane fallback that ignores any stored resume token.

### Modified Capabilities
- `tui`: home-view keybindings change so `R` = resume, `r` = fresh restart, `e` = edit/rename; the help overlay reflects the new bindings.
- `group-rename`: the group rename dialog opens on `e` instead of `r`.

## Impact

- `src/session/instance.rs`: add a restart-mode parameter to `resume_all_tracked_panes` / `resume_launch_pane` / `build_pane_resume_plan` (Resume vs Fresh); Fresh forces `resumed = false`. Add a fresh single-pane respawn that forces no resume token.
- `src/tui/app.rs`: parameterize / split the `RespawnAgentPane` action so it can drive either resume or fresh restart through the same fan-out.
- `src/tui/home/input.rs`: `R` -> resume (unchanged), `r` -> fresh restart action, `e` -> open rename/edit dialog (session and group).
- `src/tui/components/help.rs`: update the Actions overlay hints for `R`, `r`, `e`.
- `tests/e2e/`: add coverage that `r` respawns each pane fresh (no `--resume`), `R` still resumes, and `e` opens the rename dialog.
