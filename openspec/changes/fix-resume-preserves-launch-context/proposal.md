## Why

Pressing `R` to resume an agent silently strips the instance's launch context. The slot-based multi-pane resume path (`resume_all_tracked_panes` -> `resume_launch_pane` -> `build_pane_resume_command`) rebuilds each pane's command from only `agent_slot` (binary name + `native_session_id` + cwd), so a YOLO-mode agent comes back **without** its skip-permissions flag. This is a regression against two existing spec contracts (`agent-pane-restart`: "Agent launch command is reusable"; `agent-resume-restart`: "yolo, instruction, env vars SHALL remain present") and hits the common case: reconcile assigns a slot to every live pane, so a normally running instance always takes this path on `R`.

## What Changes

- The slot-based multi-pane resume path SHALL build each pane's command through the instance's shared launch-context command builder, injecting the slot's `native_session_id` as the resume token, instead of the stripped-down `build_pane_resume_command` (binary + resume flag only).
- Each resumed pane SHALL re-apply the full launch context that initial start and single-pane respawn already apply: YOLO mode (CliFlag, EnvVar, and AlwaysYolo variants), required env vars (including `AOE_INSTANCE_ID` for hook-config agents), sandbox/Docker `exec` wrapping, the cross-agent-team flag, custom instruction, and any command override.
- For an instance whose panes run different agents, the instance-level YOLO decision SHALL be applied per pane using that pane's own agent `YoloMode` variant.
- A pane with no usable resume token SHALL still degrade to a fresh launch **with** full launch context (not a bare binary).
- **BREAKING** (behavior, not data): `R`-resumed panes now carry YOLO/env/sandbox/cross-team/instruction context that the current build drops. No data or file-format change.
- Because cold-start recovery (`recover_from_slots`) shares the same per-pane resume core, it inherits the same fix.

## Capabilities

### New Capabilities
<!-- none: the corrected behavior belongs to the existing resume capability -->

### Modified Capabilities
- `agent-resume-restart`: add a requirement that the slot-based multi-pane resume path preserves the full launch context (YOLO flags, env vars, sandbox wrapping, cross-agent-team flag, custom instruction, command override) by building each pane's command through the shared launch-context builder with the slot's `native_session_id` as the resume token.

## Impact

- `src/session/instance.rs`: `build_pane_resume_command` / `resume_launch_pane` reworked (or replaced) so per-pane resume reuses the shared launch-context command construction in `build_agent_command`; `resume_all_tracked_panes` and `recover_from_slots` pass the instance launch context (YOLO, sandbox info, cross-team, command override) and per-slot agent + resume token into that builder. The command-injection validation already in `build_pane_resume_command` (safe-token / valid-resume-token checks) MUST be preserved.
- `src/agents.rs`: existing `YoloMode` variants reused; no new agent metadata.
- `tests/`: unit coverage asserting a YOLO instance's `R`-resume command per slot includes the agent's YOLO flag/env; coverage that a sandboxed instance's resumed pane is Docker-`exec` wrapped; degraded-fresh still carries launch context.
- No schema change to `agent_slot`; no migration.
