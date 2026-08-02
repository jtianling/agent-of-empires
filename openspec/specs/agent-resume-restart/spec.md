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

`R` restart、fresh restart 和 cold recovery SHALL 通过统一 pane command builder 重建每个 tracked slot。  builder SHALL 显式接收目标 slot 的 pane config 和 `native_session_id`, 并 SHALL 使用该 pane 自己的 Tool、cwd、YOLO Mode、Cross Agent Team、Worktree metadata 和 identity key。

session-level hooks、Sandbox wrapping 和 Cross Agent Team channel 等仍属 session 的上下文 SHALL 与 pane config 合并。  pane-specific Tool override、YOLO treatment、Cross Agent Team decoration 或 Worktree cwd MUST NOT 从 sibling pane 或旧 instance-level flag 推断。

没有可用 `native_session_id` 或 Tool 不支持 resume 的 pane SHALL fresh launch, 但仍 SHALL 保留自己的完整 pane config。  unknown agent 和 invalid resume token SHALL 继续通过现有输入验证拒绝或降级, 不得插入 shell command。

#### Scenario: YOLO CliFlag 只用于开启的 pane
- **WHEN**一个 tracked pane 开启 YOLO Mode 且 Tool 使用 CliFlag
- **AND**该 pane 使用 `R` restart
- **THEN**该 pane command SHALL 包含自己的 YOLO flag
- **AND**未开启 YOLO 的 sibling SHALL 不包含该 flag

#### Scenario: YOLO EnvVar 只用于目标 pane
- **WHEN**一个 tracked pane 开启 YOLO Mode 且 Tool 使用 EnvVar
- **THEN**该 pane SHALL 使用对应 env var 启动
- **AND**该 env var SHALL 不因 sibling 的值而添加或删除

#### Scenario: Hook-config agent 保留 instance id
- **WHEN**一个 tracked pane 的 Tool 需要 hook config
- **THEN**该 pane SHALL 继续获得 `AOE_INSTANCE_ID`

#### Scenario: Existing sandbox session 保留 container wrapping
- **WHEN**一个已有 sandbox session 的 tracked pane restart
- **THEN**该 pane SHALL 继续通过 session container exec wrapper 启动
- **AND** pane-level config SHALL 不关闭已有 Sandbox wrapping

#### Scenario: Non-YOLO pane 不获得 sibling flag
- **WHEN**一个 pane 关闭 YOLO Mode, 另一个 pane 开启
- **AND** session restart
- **THEN**关闭 YOLO 的 pane SHALL 不获得任何 YOLO flag 或 env var

#### Scenario: Heterogeneous panes 使用各自 Tool 语义
- **WHEN**一个 session 的 tracked slots 记录不同 Tool 和不同 YOLO Mode
- **THEN**每个 pane SHALL 按自己的 Tool `YoloMode` variant 和 enabled 值构建

#### Scenario: Degraded fresh pane 保留自己的 launch context
- **WHEN**一个 tracked slot 没有可用 resume token
- **THEN**该 pane SHALL fresh launch
- **AND** SHALL 保留自己的 cwd、YOLO Mode、Cross Agent Team、Worktree 和 identity key

#### Scenario: Cross Agent Team 只对开启的 pane 重放
- **WHEN**一个 session 中只有部分 pane 开启 Cross Agent Team
- **AND** session restart 或 cold recovery
- **THEN**只有开启的 pane SHALL 使用 tool-specific Cross Agent Team launch path
- **AND**其他 pane SHALL 普通启动

#### Scenario: Worktree cwd 按 slot 恢复
- **WHEN** primary 与 secondary slot 记录不同 Worktree cwd
- **AND** session restart 或 cold recovery
- **THEN**每个 pane SHALL 在自己 slot 的 cwd 中启动

#### Scenario: Command injection validation preserved
- **WHEN** slot 记录 unknown unsafe agent 或 invalid native session id
- **THEN**系统 SHALL 使用现有验证拒绝或降级 fresh launch
- **AND**不得把未验证值插入 shell command

#### Scenario: Invalid tracked slot is visible during restart
- **WHEN** restart 或 cold recovery 读取到一个结构性无效 slot 和至少一个有效 sibling slot
- **THEN**系统 SHALL 继续重启或恢复有效 sibling pane
- **AND** session error SHALL 显示 skipped pane count

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
