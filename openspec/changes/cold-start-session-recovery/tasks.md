## 1. Recoverable detection

- [x] 1.1 Add a helper that, given an instance and the store, returns whether it is recoverable: `read_slots_for_instance(&inst.id)` non-empty AND `tmux_session().exists()` == false. Unit-test the three cases (slots+dead = recoverable, alive = not, no slots = not).
- [x] 1.2 Compute and cache recoverability in the home-view model where instance liveness is already known, refreshing it on the same cadence as session existence.

## 2. Home-view surfacing

- [x] 2.1 Render a recoverable marker on recoverable instances in the home list (follow DESIGN.md for glyph/wording; do not collide with existing markers).
- [x] 2.2 Show a status-bar recovery hint only while a recoverable instance is focused; hide it for non-recoverable focus.

## 3. Recovery action wiring

- [x] 3.1 Pick a home-view key not already bound; add an `Action` variant for recover-focused-instance in `src/tui/home/input.rs` and dispatch it from the home keymap only when the focused instance is recoverable.
- [x] 3.2 Handle the action in `src/tui/app.rs`: re-check recoverability at action time (no-op if not recoverable / now alive), then invoke the instance rebuild, surface a per-instance error note on failure.

## 4. Session + pane rebuild core

- [x] 4.1 Add an `Instance` method (e.g. `recover_from_slots(&mut self, slots: &[AgentSlot])`) that rebuilds the tmux session via the existing start/create path (restoring worktree/sandbox context).
- [x] 4.2 Recreate one pane per slot in ascending slot order: slot 0 as the primary `@aoe_agent_pane`, remaining slots via `tmux::split_window_right_capture_pane` using each slot's `cwd`.
- [x] 4.3 Capture each new pane id at creation time in slot order (NOT via `session_pane_ids`, which orders by `pane_index` and diverges from creation order once 3+ panes exist). A slot whose pane fails to create (e.g. a now-missing `cwd`) degrades to a per-pane error and is skipped, without aborting its siblings.
- [x] 4.4 For each (slot, new_pane) call `resume_launch_pane(slot.agent, slot.native_session_id, new_pane, slot.cwd)`, collecting per-pane outcomes; do not abort siblings on one failure.

## 5. Write-back

- [x] 5.1 After rebuild, `upsert_agent_slot` each slot with its new `tmux_pane`.
- [x] 5.2 Re-pin `@aoe_agent_pane` to the slot-0 pane so reconcile and the `R` resume-all flow keep operating on the rebuilt session.

## 6. Tests

- [x] 6.1 Unit-test recoverable detection (3 cases). (The slot->pane zip is now built at creation time and no longer has a separate count-guard to test.)
- [ ] 6.2 e2e: seed an instance with N persisted `agent_slot` rows (via `aoe __record-pane` + reconcile), kill its tmux session, trigger recovery, and assert: tmux session recreated with N panes, each pane's launch command contains the right `--resume <native_session_id>`, and each slot's `agent_slot.tmux_pane` updated to the new pane id. (deferred to acceptance tester)
- [ ] 6.3 e2e: a slot with an empty/invalid `native_session_id` degrades to a fresh launch for that pane while the others resume; one pane failure does not abort the rest. (deferred to acceptance tester)

## 7. Verification

- [x] 7.1 `cargo fmt`, `cargo clippy`, `cargo test` clean for the touched crates.
- [x] 7.2 Confirm no `agent_slot` schema change and no new migration were introduced (pure consumer + `tmux_pane` write-back).

