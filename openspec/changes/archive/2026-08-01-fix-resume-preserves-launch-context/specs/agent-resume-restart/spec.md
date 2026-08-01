## ADDED Requirements

### Requirement: Slot-based multi-pane resume preserves full launch context

When the `R` restart resumes tracked panes from the persisted `agent_slot` store (the multi-pane path through `resume_all_tracked_panes`), each pane's launch command SHALL be built through the instance's shared launch-context command builder with the slot's `native_session_id` injected as the resume token. The resumed command SHALL carry the same launch context that initial start and single-pane respawn apply: YOLO mode (CliFlag, EnvVar, and AlwaysYolo variants), required env vars (including `AOE_INSTANCE_ID` for hook-config agents), sandbox/Docker `exec` wrapping, the cross-agent-team flag, custom instruction, and command override. The slot path SHALL NOT rebuild a stripped command from only the binary name and resume flag.

For an instance whose panes run different agents, the instance-level YOLO decision SHALL be applied per pane using that pane's own agent `YoloMode` variant. A pane with no usable resume token (empty/invalid `native_session_id`, or an agent without a `ResumeConfig`) SHALL still launch fresh **with** the full launch context rather than a bare binary. The cold-start recovery path (`recover_from_slots`) shares this per-pane resume core and SHALL apply the same launch context.

The existing command-injection safeguards SHALL be preserved: a slot whose recorded agent is unknown and not a safe command token, or whose `native_session_id` is not a valid resume token, SHALL be handled by the existing validation (refuse to build / degrade to fresh) and never interpolate unvalidated text into a shell command.

#### Scenario: YOLO CliFlag agent keeps its flag on slot resume
- **WHEN** the user presses `R` on a running YOLO-mode instance whose agent uses a `CliFlag` YOLO variant (e.g. Claude `--dangerously-skip-permissions`)
- **AND** the instance has a tracked `agent_slot` with a valid `native_session_id`
- **THEN** the resumed pane command SHALL include the agent's YOLO `CliFlag`
- **AND** SHALL include the resume flag built from the slot's `native_session_id`

#### Scenario: YOLO EnvVar agent keeps its env var on slot resume
- **WHEN** the user presses `R` on a running YOLO-mode instance whose agent uses an `EnvVar` YOLO variant (e.g. opencode `OPENCODE_PERMISSION`)
- **THEN** the resumed pane SHALL be launched with that YOLO env var set

#### Scenario: Hook-config agent keeps AOE_INSTANCE_ID on slot resume
- **WHEN** the user presses `R` on an instance whose agent has a hook config (requires `AOE_INSTANCE_ID`)
- **THEN** the resumed pane SHALL be launched with `AOE_INSTANCE_ID` set to the instance id

#### Scenario: Sandboxed instance stays Docker-wrapped on slot resume
- **WHEN** the user presses `R` on a sandboxed instance with a tracked slot
- **THEN** the resumed pane command SHALL be wrapped to run inside the instance's Docker container (`docker exec ...`) rather than executing the agent binary directly on the host

#### Scenario: Non-YOLO instance gains no YOLO flag on slot resume
- **WHEN** the user presses `R` on a running non-YOLO instance with a tracked slot
- **THEN** the resumed pane command SHALL NOT include any YOLO flag or YOLO env var

#### Scenario: Heterogeneous panes apply per-agent YOLO variant
- **WHEN** the user presses `R` on a YOLO-mode instance whose tracked slots record different agents
- **THEN** each resumed pane SHALL apply the YOLO treatment of its own slot agent's `YoloMode` variant

#### Scenario: Degraded-fresh pane still carries launch context
- **WHEN** a tracked slot has no usable resume token (empty/invalid `native_session_id` or an agent without `ResumeConfig`)
- **THEN** that pane SHALL launch fresh with the instance's full launch context (YOLO, env vars, sandbox wrapping, cross-agent-team flag, custom instruction) applied
- **AND** SHALL NOT be launched as a bare binary

#### Scenario: Command-injection validation preserved
- **WHEN** a tracked slot records an agent name that is unknown and not a safe command token, or a `native_session_id` that is not a valid resume token
- **THEN** the system SHALL apply the existing validation (refuse to build the pane command or degrade to fresh) and SHALL NOT interpolate the unvalidated value into the shell command

#### Scenario: Cold-start recovery applies the same launch context
- **WHEN** an instance is recovered from persisted slots via `recover_from_slots`
- **THEN** each rebuilt pane SHALL apply the same full launch context as the `R` slot-resume path
