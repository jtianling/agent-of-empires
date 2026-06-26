## Context

`R` restart has two command-building paths:

- Single-pane / no-slots: `respawn_agent_pane` -> `Instance::build_agent_command(resume_token)`. This applies the full launch context (YOLO via `is_yolo_mode()`, env vars incl. `AOE_INSTANCE_ID`, sandbox `docker exec` wrapping, cross-agent-team flag, custom instruction, command override).
- Multi-pane / slots present: `resume_all_tracked_panes(slots)` -> free function `resume_launch_pane(agent, native_session_id, tmux_pane, cwd)` -> `build_pane_resume_command(agent, native_session_id)`, which produces only `"{binary} {resume_flag}"`.

Reconcile assigns a slot to every live pane (the agent pane becomes slot 0), so a normally-running instance always has >= 1 slot and `R` takes the second path (`app.rs` branches on `slots.is_empty()`). The second path drops YOLO, env vars, sandbox wrapping, cross-team flag, instruction, and override. The same free function is also the per-pane core of cold-start recovery (`recover_from_slots`), so recovery has the same gap.

This violates existing spec contracts: `agent-pane-restart` ("Agent launch command is reusable" -- one shared builder for start and respawn) and `agent-resume-restart` ("yolo, instruction, env vars SHALL remain present").

## Goals / Non-Goals

**Goals:**
- Collapse the two command builders into one decoration pipeline so the slot/multi-pane resume path produces the same launch context as initial start and single-pane respawn.
- Inject each slot's `native_session_id` as the resume token while preserving YOLO, env vars (`AOE_INSTANCE_ID`), sandbox wrapping, cross-team flag, instruction, and command override.
- Preserve the existing command-injection validation (`is_safe_command_token`, `is_valid_resume_token`).
- Fix `recover_from_slots` for free by sharing the same builder.

**Non-Goals:**
- No `agent_slot` schema change, no migration.
- No change to slot assignment / reconcile logic.
- No change to the now-superseded tick-driven scrape state machine.
- No new agent metadata.
- Not reopening whether custom-command instances should resume at all (pre-existing behavior of `build_agent_command` is reused as-is for the primary pane).

## Decisions

### Decision 1: One decoration pipeline keyed by agent + resume token

Introduce a single per-pane command builder on `Instance` that takes the target agent name, an optional resume token, and whether the pane is primary, and runs the existing decoration pipeline (resume-flag insertion -> YOLO match on the target agent's `YoloMode` -> cross-team flag -> `AOE_INSTANCE_ID` for hook-config agents -> sandbox `docker exec` wrap vs host wrap -> command override). `build_agent_command` is refactored to delegate to it for the primary agent (`self.tool`). This removes `build_pane_resume_command`'s parallel logic rather than patching it.

- **Why:** the root cause is two divergent builders. Patching `build_pane_resume_command` to also add YOLO would re-create the same drift for the next launch knob (it already misses sandbox/env/cross-team). A single pipeline keeps `agent-pane-restart`'s "reusable command" contract true.
- **Alternative considered (rejected):** add a `yolo_mode` column to `agent_slot` and add the flag in `build_pane_resume_command`. Rejected: data duplication + migration, and it would still miss sandbox/env/cross-team/instruction/override.

### Decision 2: Per-slot agent, instance-level YOLO

YOLO is an instance-level bool (`is_yolo_mode()`), but slots record a per-pane `agent`. The pipeline applies the instance YOLO decision using the target slot agent's own `YoloMode` variant (CliFlag/EnvVar/AlwaysYolo), so a heterogeneous multi-agent instance resumes each pane with the correct YOLO mechanism.

### Decision 3: Command override scoped to the primary pane

`instance.command` (custom command override) is an instance-primary concept. The pipeline applies the override only for the primary pane (slot 0 / `self.tool`); secondary slots build from their own agent binary. This matches today's single-pane behavior and avoids forcing a primary override onto a different secondary agent.

### Decision 4: Keep tmux mechanics and validation where they are

`resume_launch_pane` keeps its responsibility for killing the pane process tree and respawning via tmux, plus the safe-token / valid-resume-token validation. Only the command *construction* moves into the `Instance` builder. The builder is called by `resume_all_tracked_panes` / `recover_from_slots` (which have `&self`/`&mut self`), and the validated, fully-decorated command string is handed to the tmux respawn step.

## Risks / Trade-offs

- **Heterogeneous secondary agent + sandbox** -> each secondary pane `docker exec`s into the same shared container using the instance `sandbox_info`; verify the wrap is built per slot agent, not hardcoded to `self.tool`.
- **Resume flag + YOLO flag ordering** -> the existing single-pane builder already inserts the resume flag right after the binary and appends YOLO after; reuse that ordering so resumed commands match the start-path shape (covered by the existing `agent-resume-restart` scenarios).
- **Behavior change is intentional and broad** -> every `R` on a YOLO/sandboxed/cross-team instance now relaunches with that context. Surface as a BREAKING (behavior) note; no data/format change so no rollback data concern.
- **Validation regression risk** -> moving construction must not bypass `is_safe_command_token` / `is_valid_resume_token`; a dedicated unit test asserts an unsafe slot agent name is still refused.

## Open Questions

- None blocking. (Pre-existing inconsistency: the multi-pane path does not honor the "custom-command instances skip resume" scenario from `agent-resume-restart`; left as-is for the primary pane via `build_agent_command` and out of scope here.)
