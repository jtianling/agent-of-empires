## 1. Unify the command builder

- [x] 1.1 In `src/session/instance.rs`, extract the launch-context decoration pipeline (resume-flag insertion, YOLO match on the target agent's `YoloMode`, cross-team flag, `AOE_INSTANCE_ID` for hook-config agents, sandbox `docker exec` vs host wrap, command override) into a single per-pane builder method on `Instance` that takes the target agent name, an optional resume token, and an `is_primary` flag.
- [x] 1.2 Refactor `build_agent_command(resume_token)` to delegate to the new builder for the primary agent (`self.tool`, `is_primary = true`); confirm the produced command is byte-identical to today for the single-pane path.
- [x] 1.3 Apply YOLO per the target slot agent's own `YoloMode` variant (CliFlag/EnvVar/AlwaysYolo) gated on the instance-level `is_yolo_mode()`.
- [x] 1.4 Scope command override (`self.command`) to `is_primary` only; secondary slots build from their own agent binary.

## 2. Route the slot/multi-pane resume path through the unified builder

- [x] 2.1 Rework `build_pane_resume_command` / `resume_launch_pane` so command *construction* comes from the new `Instance` builder (with the slot's `native_session_id` as the resume token), while tmux kill + respawn mechanics stay in place.
- [x] 2.2 Preserve the existing command-injection validation (`is_safe_command_token`, `is_valid_resume_token`): an unknown/unsafe slot agent name or an invalid resume token must still be refused or degrade to fresh, never interpolated raw into the shell command.
- [x] 2.3 Ensure a slot with no usable resume token degrades to a fresh launch that still carries full launch context (not a bare binary).
- [x] 2.4 Update `resume_all_tracked_panes` to drive the builder per slot (per-slot agent + token), preserving per-pane failure isolation and aggregated `Restarting` status.
- [x] 2.5 Confirm `recover_from_slots` (cold-start recovery) shares the same per-pane builder and thus inherits the launch-context fix.

## 3. Tests

- [x] 3.1 Unit test: a YOLO CliFlag instance (e.g. Claude) resumed via the slot path produces a command containing the YOLO flag and the resume flag built from `native_session_id`.
- [x] 3.2 Unit test: a YOLO EnvVar agent (e.g. opencode) resumed via the slot path sets the YOLO env var.
- [x] 3.3 Unit test: a hook-config agent resumed via the slot path sets `AOE_INSTANCE_ID`.
- [x] 3.4 Unit test: a sandboxed instance resumed via the slot path is `docker exec` wrapped, not a bare host binary.
- [x] 3.5 Unit test: a non-YOLO instance resumed via the slot path has no YOLO flag/env.
- [x] 3.6 Unit test: a degraded-fresh pane (no valid token) still carries full launch context.
- [x] 3.7 Unit test: an unsafe slot agent name / invalid resume token is still refused (injection guard intact).
- [x] 3.8 Unit test: heterogeneous slots apply each pane's own agent `YoloMode` variant.

## 4. Finalize

- [x] 4.1 Run `cargo fmt` (clean), `cargo clippy --all-targets` (no new warnings; one pre-existing doc-list warning in unmodified `tests/e2e/cold_start_recovery.rs` left untouched), and the new pure command-builder unit tests (`cargo test --lib -- build_pane_resume_plan slot_resume`, 15 passed). NOTE: the full `cargo test` suite was deliberately NOT run -- many non-e2e unit/integration tests operate on the default tmux socket and would kill the user's live aoe sessions (see AGENTS.md "Tmux Session Safety"). The new tests are pure string builders and tmux-safe.
