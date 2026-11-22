# Capability Spec: Agent Pane Restart

**Capability**: `agent-pane-restart`
**Created**: 2026-03-20
**Status**: Draft

## Purpose

Supports restarting only the AoE-managed agent pane within a tmux session, preserving the session layout and all user-created panes. Uses a reusable command builder and scoped process cleanup to ensure restarts are safe and consistent.
## Requirements
### Requirement: Respawn agent pane without destroying session
The system SHALL support restarting only the AoE-managed agent pane within a tmux session, preserving the session layout and all user-created panes. The respawn SHALL use `tmux respawn-pane -k -t <pane_id>` targeting the stored `@aoe_agent_pane`.

#### Scenario: Respawn dead agent pane in multi-pane session
- **WHEN** the agent pane is dead (process exited)
- **AND** the session has user-created split panes
- **AND** the user triggers agent pane restart
- **THEN** the system SHALL respawn only the agent pane with the original agent command
- **AND** all user-created panes and the session layout SHALL be preserved
- **AND** the session status SHALL transition to `Starting`

#### Scenario: Force-restart running agent pane
- **WHEN** the agent pane is alive (process running)
- **AND** the user triggers agent pane restart via `R` keybinding
- **AND** the agent has a `ResumeConfig`
- **AND** the instance does not use a custom command
- **THEN** the system SHALL initiate the graceful resume restart flow
- **AND** set the instance status to `Restarting`

#### Scenario: Force-restart without resume support
- **WHEN** the agent pane is alive (process running)
- **AND** the user triggers agent pane restart via `R` keybinding
- **AND** the agent has no `ResumeConfig` OR the instance uses a custom command
- **THEN** the system SHALL kill the agent pane's process tree
- **AND** respawn the pane with the original agent command
- **AND** all user-created panes SHALL be preserved

#### Scenario: Respawn single-pane session falls back to full restart
- **WHEN** the agent pane is dead
- **AND** the session has only 1 pane (no user-created splits)
- **AND** the restart is triggered via attach-time recovery (not `R` keybinding)
- **THEN** the system SHALL use the existing kill-session + recreate flow

#### Scenario: Respawn re-runs on-launch hooks
- **WHEN** an agent pane is respawned
- **THEN** the system SHALL execute on-launch hooks before starting the agent
- **AND** the hooks SHALL run in the same manner as during initial session creation

#### Scenario: Respawn re-applies tmux session options
- **WHEN** an agent pane is respawned
- **THEN** the system SHALL call `apply_tmux_options()` after respawn
- **AND** tmux options (status bar, mouse, etc.) SHALL remain correct

### Requirement: Agent launch command is reusable
The agent launch command construction (binary, extra_args, yolo flags, env vars, custom instruction, model flag) SHALL be extracted into a reusable method so both initial session creation and pane respawn can share the same command-building logic.

### Requirement: Codex 长会话的配置恢复

恢复 Codex pane 时, 若原 thread 已卸载, SHALL 从该精确 thread 的持久化历史取得最后一份完整且归属匹配的配置快照.  快照不得仅因后续会话记录超过固定 tail 窗口而被忽略.  文件 SHALL 流式读取, 保留快照中的 model, effort 和 service tier, 不从其他 pane 或共享配置猜测缺失值.

#### Scenario: 快照位于大型历史文件的较早位置

- **WHEN** 右 pane 的 Codex thread 已卸载, 最后配置快照之后累计超过 4 MB 会话记录
- **THEN** 恢复流程仍能取得该 thread 的最后完整快照, 并使用原会话 ID 与配置恢复
- **AND** 不因末尾窗口内缺少快照而仅恢复左 pane

#### Scenario: 忽略其他 thread 的配置

- **WHEN** 历史中存在归属于其他 thread 的更新快照
- **THEN** 只使用与目标 thread ID 精确匹配的最后配置

The model flag is the pane's observed model, appended after `extra_args`, and applies only to `claude` panes as specified in `claude-model-continuity`. It SHALL be produced by this same reusable method rather than by any per-keybinding or per-restart-path special case, so every caller of the builder gets identical composition.

#### Scenario: Respawn uses same command as initial start
- **WHEN** an agent pane is respawned
- **THEN** the respawn command SHALL be identical to what `start_with_size_opts()` would produce for the same instance configuration
- **AND** env vars, yolo flags, custom instructions, and the model flag SHALL all be applied

#### Scenario: Model flag comes from the shared builder
- **WHEN** any restart path builds a command for a `claude` pane that has an observed model
- **THEN** the model flag SHALL be produced by the shared command builder
- **AND** no restart path SHALL add or omit the model flag on its own

### Requirement: Scoped process cleanup for respawn
When respawning the agent pane, the system SHALL only kill the process tree of the agent pane, not processes in user-created panes.

#### Scenario: Process cleanup targets only agent pane
- **WHEN** the system respawns the agent pane
- **AND** user-created panes have running processes
- **THEN** only the agent pane's process tree SHALL be terminated
- **AND** processes in user-created panes SHALL not be affected

