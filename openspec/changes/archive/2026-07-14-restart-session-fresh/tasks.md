## 1. Core: fresh restart mode in the fan-out

- [x] 1.1 Add a restart mode enum (`RestartMode { Resume, Fresh }`) in `src/session/instance.rs`.
- [x] 1.2 Thread the mode through `build_pane_resume_plan` so `Fresh` forces `resumed = false` (no resume flag) while still building the full launch context via `build_pane_command`.
- [x] 1.3 Thread the mode through `resume_launch_pane` and `resume_all_tracked_panes` (keep resume behavior identical when mode is `Resume`).
- [x] 1.4 Add a fresh single-pane respawn path that does NOT consult `self.resume_token` (bypass `resolved_resume_token`), for the no-tracked-slots fallback.

## 2. Wiring: action + keybindings

- [x] 2.1 In `src/tui/app.rs`, make the `RespawnAgentPane` handler drive either resume or fresh restart (add mode to the action or add a sibling `RestartAgentPaneFresh` action sharing the handler body), honoring `restart_in_flight` and the `Deleting` no-op.
- [x] 2.2 In `src/tui/home/input.rs`, keep `R` -> resume (unchanged); bind `r` (no Shift) -> fresh restart action.
- [x] 2.3 In `src/tui/home/input.rs`, bind `e` -> the existing rename/edit block (session RenameDialog, and GroupRenameDialog when a group is selected); remove the rename/edit trigger from `r`.

## 3. Help overlay

- [x] 3.1 In `src/tui/components/help.rs`, update Actions hints: `R` = "Resume agent panes", add `r` = "Restart agent panes (fresh)", change `r`->`e` entry to "Edit/rename session or group".

## 4. Tests

- [x] 4.1 Unit test: fresh single-pane restart command carries no `--resume` even when `resume_token` is set (Decision 2).
- [x] 4.2 Unit test / assertion: `Fresh` mode produces `DegradedToFresh`-style commands (launch context present, no resume flag) for every pane.
- [x] 4.3 Update existing `src/tui/home/tests.rs` key tests that assume `r` opens rename (now `e`).
- [x] 4.4 E2E (`tests/e2e/`): pressing `r` respawns each tracked pane fresh (no `--resume`); `R` still resumes; `e` opens the rename dialog.

## 5. Verification

- [x] 5.1 Run `cargo fmt`, `cargo clippy`, and `cargo test` (unit + integration).
- [x] 5.2 Confirm the help overlay and status-bar hints match the actual bindings (R/r/e).
