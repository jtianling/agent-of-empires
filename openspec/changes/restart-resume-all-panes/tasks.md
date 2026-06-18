## 1. Pane-parameterized tmux operations

- [x] 1.1 Add `tmux_pane`-target variants in `src/tmux/session.rs` for respawn, process-tree kill, and send-keys (e.g. `respawn_pane_target`, `kill_pane_process_tree_target`), taking an explicit pane id instead of resolving `@aoe_agent_pane`.
- [x] 1.2 Reimplement the existing `respawn_agent_pane` / `kill_agent_pane_process_tree` / `send_keys_to_agent_pane` to delegate to the new target variants using `get_agent_pane_id`, preserving current single-pane behavior.
- [x] 1.3 Add unit coverage that the target variants issue tmux commands against the given pane (e.g. `respawn-pane -k -t <pane>`).

## 2. Reusable per-pane resume-launch core

- [x] 2.1 Add a function (in `src/session/instance.rs` or a focused helper) that, given `(agent, native_session_id, tmux_pane, cwd)`, builds the resume command from `ResumeConfig.resume_flag` and performs kill+respawn for that one pane, returning a per-pane result (ok / degraded-to-fresh / error).
- [x] 2.2 Handle degrade-to-fresh inside the core: empty `native_session_id`, agent without `ResumeConfig`, or custom command -> respawn that pane with a fresh command (no resume flag).
- [x] 2.3 Unit test resume-command construction for claude (`--resume <id>`) and codex (`resume <id>`), and the fresh-fallback branches.

## 3. R handler fans out to all tracked panes

- [x] 3.1 In `Action::RespawnAgentPane` (`src/tui/app.rs`), read `store.read_slots_for_instance(&inst.id)` and iterate slots, calling the per-pane resume-launch core for each `tmux_pane`.
- [x] 3.2 No-slots fallback: when the instance has no `agent_slot` rows, restart the primary `@aoe_agent_pane` with the existing single-pane behavior.
- [x] 3.3 Per-pane failure isolation: a pane error is recorded and does not abort restarts of the remaining panes.

## 4. Retire the single-pane graceful state machine

- [x] 4.1 Remove the `pending_resume` exit-key/wait/scrape state machine from the `R` path: `initiate_graceful_restart`, `tick_pending_resume`, `mutate_pending_resume_phase`, the `PendingResume`/`RestartPhase` types, and the `tick_pending_resume` call site in `src/tui/app.rs`.
- [x] 4.2 Replace in-flight de-duplication (ignore second `R` during a restart) with a lightweight per-instance "restart in flight" flag.
- [x] 4.3 Update or remove tests tied to the old state machine (`test_pending_resume_*`) and reconcile any now-unused imports/helpers introduced by the removal.

## 5. Status aggregation

- [x] 5.1 Set instance status to `Restarting` while the multi-pane restart is in flight and transition to `Starting` once every tracked pane is respawned.

## 6. End-to-end coverage

- [x] 6.1 Add an e2e test in `tests/e2e/` (modeled on `multi_agent_session.rs`): start a session with 2-3 panes, inject capture+reconcile via `aoe __record-pane` so `agent_slot` rows exist, press `R`, and assert each pane's respawn command contains `--resume <native_session_id>`.
- [x] 6.2 e2e assert: a tracked pane whose agent has no `ResumeConfig` restarts fresh without error and without blocking the resume of sibling panes.
- [x] 6.3 Run the e2e suite under an isolated `$HOME` in `~/workspace/test` (per project E2E isolation), use tiled multi-pane sizing (`harness.resize_window`), and add `.timeout` on sqlite CLI reads.

## 7. Finalize

- [x] 7.1 Run `cargo fmt`, `cargo clippy`, `cargo test`, and the e2e suite; fix warnings.
- [x] 7.2 Verify the keybinding/status-bar hint for `R` still matches behavior (status-bar text describes restart correctly for multi-pane).
