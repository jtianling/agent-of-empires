# Capability Spec: Agent Resume Restart

**Capability**: `agent-resume-restart`
**Created**: 2026-03-23
**Status**: Draft

## Purpose

Supports graceful agent restart with session resumption. When an agent supports resume (e.g., Claude, Codex), the restart flow sends exit keys, captures the resume token from pane output, and respawns the agent with the token so conversation context is preserved. The flow is tick-driven to keep the TUI responsive.
## Requirements
### Requirement: Graceful restart captures resume token from agent output
When the user presses `R`, the restart is handled by the multi-pane store-based flow specified in `multi-pane-resume-restart`. For each tracked agent pane that supports resume, the resume token SHALL be sourced from the persisted `agent_slot.native_session_id` and inserted into the agent command directly. The system SHALL NOT send exit keys to the pane nor scrape a resume token from pane output for the `R` keybinding. A pane with no persisted `native_session_id` or no `ResumeConfig` SHALL restart fresh (no resume token). The resume decision is per tracked pane based on its recorded `agent_slot.agent`, independent of the instance's configured `command`: a pane that ran a resumable agent and recorded a session id resumes even when `instance.command` is a custom command.

#### Scenario: R delegates to per-pane store-based resume
- **WHEN** the user presses R on a session whose agent has a `ResumeConfig`
- **AND** the pane has a persisted `agent_slot.native_session_id`
- **THEN** the system SHALL respawn the pane with `resume_flag` filled from `native_session_id`
- **AND** the system SHALL NOT send exit keys to the pane
- **AND** the system SHALL NOT capture or regex-scrape a resume token from pane output

#### Scenario: Pane without persisted session id restarts fresh
- **WHEN** the user presses R
- **AND** a tracked pane's agent has a `ResumeConfig` but no persisted `native_session_id`
- **THEN** the system SHALL respawn that pane with a fresh command (no resume token)

#### Scenario: Resume is decided per recorded pane agent, not the instance command
- **WHEN** the user presses R on a session whose `instance.command` is a custom command
- **AND** a tracked pane recorded a resumable agent (`agent_slot.agent` with a `ResumeConfig`) and a non-empty `native_session_id`
- **THEN** the system SHALL respawn that pane with `resume_flag` filled from its `native_session_id`

#### Scenario: Agent without ResumeConfig restarts fresh
- **WHEN** the user presses R on a pane whose agent has no `ResumeConfig` (resume is `None`)
- **THEN** the system SHALL respawn that pane with a fresh command (no resume token)

### Requirement: Restarting status provides user feedback
The system SHALL set the instance status to `Restarting` during the graceful exit flow so the UI can display appropriate feedback.

#### Scenario: Status shows Restarting during graceful exit
- **WHEN** a graceful restart is initiated
- **THEN** the instance status SHALL be `Restarting`
- **AND** when the respawn completes (with or without resume), the status SHALL transition to `Starting`

### Requirement: Send keys to agent pane
The system SHALL provide a method to send arbitrary key sequences to the agent pane via `tmux send-keys`, targeting the stored `@aoe_agent_pane`.

#### Scenario: Send keys targets the correct pane
- **WHEN** the system sends keys to the agent pane
- **AND** the session has multiple panes (user-created splits)
- **THEN** the keys SHALL be sent only to the pane identified by `@aoe_agent_pane`
- **AND** other panes SHALL not receive the keys

### Requirement: Resume token inserted into agent command
When a resume token is captured, the system SHALL insert the agent's `resume_flag` (with token substituted) into the command immediately after the binary name, before extra_args and other flags.

#### Scenario: Claude restart with resume token
- **WHEN** the system captures resume token `4dc7a3c8-934e-40c1-95f8-8b00fe11cf11` from Claude
- **THEN** the restart command SHALL include `--resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11` after the `claude` binary
- **AND** other flags (yolo, instruction, env vars) SHALL remain present and follow the resume flag

#### Scenario: Codex restart with resume token
- **WHEN** the system captures resume token `019d1af9-a899-7df1-8f7d-a244126e5ded` from Codex
- **THEN** the restart command SHALL include `resume 019d1af9-a899-7df1-8f7d-a244126e5ded` after the `codex` binary
- **AND** other flags (yolo, instruction, env vars) SHALL remain present and follow the resume subcommand

### Requirement: Resume token is persisted to session storage
The Instance struct SHALL include a `resume_token: Option<String>` field that is serialized to sessions.json. The field SHALL use serde default so that old session files without this field deserialize without error.

#### Scenario: Resume token survives AoE restart
- **WHEN** the status poller captures a resume token for an instance
- **AND** AoE is closed and reopened
- **THEN** the stored resume token SHALL be available on the deserialized Instance

#### Scenario: Old sessions.json without resume_token field deserializes correctly
- **WHEN** sessions.json contains Instance entries without a `resume_token` field
- **THEN** deserialization SHALL succeed with `resume_token` set to `None`

### Requirement: Stored resume token is cleared after consumption
After a resume token is used to restart an agent (whether from stored token or live extraction), the system SHALL clear the `resume_token` field on the Instance. The token SHALL also be cleared when the agent pane is freshly started without resume.

#### Scenario: Token cleared after successful resume restart
- **WHEN** the system respawns an agent pane using a stored resume token
- **THEN** the Instance's `resume_token` SHALL be set to `None`
- **AND** sessions.json SHALL be saved with the cleared token

#### Scenario: Token cleared on fresh restart
- **WHEN** the system respawns an agent pane without a resume token (fresh start)
- **THEN** the Instance's `resume_token` SHALL be set to `None`

#### Scenario: Token cleared when new agent session starts
- **WHEN** an Instance transitions to `Starting` status via a new `start()` call
- **THEN** any previously stored `resume_token` SHALL be cleared

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

### Requirement: Fan-out resume restart falls back to the instance's stored resume token

When a resume restart fans out across an instance's tracked panes, and slot 0's durable record carries no native session id, AoE SHALL resume that pane from the instance's stored resume token.

The instance's stored resume token is scraped from the primary pane's own output and is the only resume source available before a capture exists. A restart with no tracked panes already consults it; once every launched pane has a slot record from launch, the fan-out path becomes the one that runs in that window too, and without this fallback a restart that used to reattach the conversation would silently start a fresh one.

The fallback applies to slot 0 alone, which is the pane the instance's resume token describes.

#### Scenario: Slot 0 with no native session id resumes from the stored token

- **WHEN** an instance is restarted in resume mode
- **AND** slot 0's record carries no native session id
- **AND** the instance has a stored resume token
- **THEN** slot 0's pane SHALL be relaunched with that resume token

#### Scenario: A recorded native session id takes precedence

- **WHEN** an instance is restarted in resume mode
- **AND** slot 0's record carries a native session id
- **THEN** slot 0's pane SHALL resume from that native session id
- **AND** the instance's stored resume token SHALL NOT override it

#### Scenario: A fresh restart ignores the stored token

- **WHEN** an instance is restarted clean
- **AND** slot 0's record carries no native session id
- **THEN** the pane SHALL be launched with no resume token

