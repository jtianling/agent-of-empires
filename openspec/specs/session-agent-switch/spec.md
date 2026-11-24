# session-agent-switch Specification

## Purpose
允许在主界面将 pane 在 Claude, Codex 和 OpenCode 之间切换.  切换只重启所选 pane 并创建新会话, 保留原 worktree 目录和 xats name/team, 后续 resume 使用切换后的 agent 与会话.
## Requirements
### Requirement: Agent switch keybindings
The home view SHALL bind `A` (Shift+a) and `a` on a selected session to open an agent switch dialog.  Both keys SHALL open the same dialog; they differ only in what happens after the switch is confirmed: `A` SHALL fresh-restart the switched panes and attach to the session, `a` SHALL fresh-restart the switched panes in the background and keep the user on the home view.  The keys SHALL do nothing when no session is selected, when the session is deleting, or when a restart is already in flight for it.

#### Scenario: Shift+a opens the dialog
- **WHEN** a claude session is selected and the user presses `A`
- **THEN** the agent switch dialog opens for that session

#### Scenario: Lowercase a opens the dialog
- **WHEN** a codex session is selected and the user presses `a`
- **THEN** the agent switch dialog opens for that session

#### Scenario: Busy session is ignored
- **WHEN** the selected session is deleting or has a restart in flight and the user presses `a` or `A`
- **THEN** no dialog opens and nothing changes

#### Scenario: Group row is ignored
- **WHEN** a group row is selected and the user presses `a` or `A`
- **THEN** no dialog opens

### Requirement: Switchable agents
切换 SHALL 支持 `claude`, `codex` 和 `opencode`, 仅对运行这些 agent 的 pane 提供选项.  A session with no such pane, or a sandboxed session, SHALL NOT open the dialog; the home view SHALL instead show an info dialog explaining why the agent cannot be switched.  A session whose tmux session is not running and that has more than one switchable pane SHALL NOT open the dialog either, since bringing it back would restart every pane; the info dialog SHALL ask the user to start the session first.

#### Scenario: Unsupported agent
- **WHEN** 用户选中运行 `shell` 的 session 并按下 `a`
- **THEN** 不打开切换对话框, 并提示仅支持 Claude, Codex 和 OpenCode

#### Scenario: Sandboxed session
- **WHEN** a sandboxed claude session is selected and the user presses `a`
- **THEN** the switch dialog does not open and an info dialog explains that sandboxed sessions cannot switch agents

#### Scenario: Stopped split session
- **WHEN** a session with a claude pane and a codex pane is not running and the user presses `a`
- **THEN** the switch dialog does not open and an info dialog asks the user to start the session first

### Requirement: Agent switch dialog
The dialog SHALL show one row per switchable pane of the session, each row offering the switchable agents with the pane's current agent marked `(now)`; a row set to its current agent keeps that pane, a row set to another agent switches it.  With more than one switchable pane, rows SHALL be named by where the panes sit when there are exactly two of them (`Left`/`Right`, or `Top`/`Bottom` when stacked), and `Pane N` (slot + 1) otherwise, and SHALL be listed in on-screen order; every row SHALL start set to its current agent.  单 pane 行 SHALL 使用 `Agent:` 标签, 并默认选中列表中第一个不同于当前 agent 的选项.  Up/Down SHALL move between rows; Left/Right/Tab/Space SHALL cycle the agent of the focused row.  Enter SHALL confirm every row set to a new agent; with no such row Enter SHALL behave like cancel.  Esc SHALL cancel.

#### Scenario: Confirm switch
- **WHEN** the dialog for a single-pane claude session is open and the user presses Enter without changing the selection
- **THEN** the session is switched to codex

#### Scenario: Switch only the right pane
- **WHEN** a session has a claude pane on the left and a codex pane on the right, the user presses Down, Left and then Enter
- **THEN** only the right pane is switched, to claude, and the left pane is left as it is

#### Scenario: Switch both panes
- **WHEN** a session has a claude pane on the left and a codex pane on the right, the user presses Right, Down, Left and then Enter
- **THEN** the left pane is switched to codex and the right pane to claude

#### Scenario: Nothing set to switch
- **WHEN** a split session's dialog is open and the user presses Enter without changing any row
- **THEN** the dialog closes and nothing changes or restarts

#### Scenario: Cancel
- **WHEN** the user presses Esc in the dialog
- **THEN** the dialog closes and the session is unchanged

#### Scenario: Choosing the current agent
- **WHEN** the user cycles a single-pane session's row back to the current agent and presses Enter
- **THEN** the dialog closes and the session is unchanged and not restarted

### Requirement: Switch commits before restart
Confirming a switch SHALL durably commit the new agent of every switched pane before any restart begins.  When a switched pane is the primary pane (slot 0), the instance's primary pane tool SHALL become the target agent; its command SHALL be reset to what a new session of the target agent would get (the agent's default binary when it has one, or the configured command override for that agent); its extra args SHALL be reset to the configured extra args for the target agent; its resume token, agent session id and fork token SHALL be cleared.  仅切换副 pane 时, 主 pane 配置 SHALL 保持不变, 托管副 pane 的持久化记录 SHALL 同步更新.  Each switched durable slot SHALL take its target agent, and its native session id, observed model and model fingerprint SHALL be cleared; its pane capture and Codex binding SHALL be dropped.  Every slot not switched SHALL be left unchanged.  When the session's tmux session is live and it has no durable slot yet, the primary pane SHALL first be recorded as slot 0 so the restart follows the slot's agent rather than the process still running in the pane.  The instance SHALL then be saved.  If committing any pane fails, no restart SHALL start and the error SHALL be shown on the session, naming any panes already switched.

#### Scenario: Switch claude to codex
- **WHEN** a single-pane claude session with a recorded slot 0 is switched to codex
- **THEN** the instance tool and slot 0 agent are `codex`, the instance command is `codex`, slot 0 has an empty native session id and empty model, and slot 0 keeps its xats identity key

#### Scenario: Switch codex to claude
- **WHEN** a codex session is switched to claude
- **THEN** the instance command is reset to empty (no override) or to the configured claude command override, and the codex binding of its slots is removed

#### Scenario: Other panes untouched
- **WHEN** the right pane (slot 1) of a session with claude in both slots is switched to codex
- **THEN** slot 1 becomes codex with an empty native session id, slot 0 keeps its agent and native session id, and the instance tool, command and agent session id are unchanged

#### Scenario: No slot yet
- **WHEN** a live claude session with no durable slots is switched to codex
- **THEN** slot 0 is recorded for the primary pane with agent `codex` before the restart runs

### Requirement: Fresh restart after switch
After the switch is committed, when the session's tmux session is running, only the switched panes SHALL be respawned with fresh mode, launching its slot's agent with a new conversation and no resume; every other pane SHALL keep running untouched.  If a switched slot is no longer tracked, nothing SHALL be restarted and the error SHALL be shown on the session.  When the session is not running (only possible with a single switchable pane), the session SHALL be restarted with fresh mode through the existing restart paths (cold recovery, or plain start).

#### Scenario: Background restart
- **WHEN** the user confirms a switch opened with `a` on a running session
- **THEN** only the switched panes are queued on the background restart worker with fresh mode and the user stays on the home view

#### Scenario: Attach restart
- **WHEN** the user confirms a switch opened with `A` on a running session
- **THEN** only the switched panes are fresh-restarted and the user is attached to the session

#### Scenario: Sibling pane keeps running
- **WHEN** the right pane of a running two-pane session is switched
- **THEN** the left pane's process is not restarted and keeps its conversation

### Requirement: xats identity survives the switch
The switch SHALL keep every switched slot's xats identity key, every other slot's key, and the instance's own identity key unchanged, so the new agent launches with the same identity key as the old one and recovers the old agent's xats name and team.

#### Scenario: Identity key unchanged
- **WHEN** a Cross Agent Team session with slot 0 identity key `K` is switched from claude to codex
- **THEN** slot 0 still holds `K` and the codex launch is bootstrapped with `K`

### Requirement: Identity preview before switching
For Cross Agent Team panes, opening the switch dialog SHALL ask the xats daemon (`POST /api/identity-key/lookup`) who holds the identity key of every switchable pane, and the dialog SHALL show, for every pane currently set to switch, the identity the new agent will come back as, prefixed with the pane's name.  A pane whose key no identity holds, a pane without a key, a daemon without the lookup endpoint (HTTP 404), and a failed lookup SHALL each be shown as a warning instead of an identity.  A 404 SHALL NOT be treated as an unheld key.  The lookup SHALL NOT block the switch.

#### Scenario: Held key
- **WHEN** the left pane is set to switch and the daemon reports its key is held by `mie-main@mie`
- **THEN** the dialog shows `Left: comes back as mie-main@mie`

#### Scenario: Unheld key
- **WHEN** the daemon answers `not_found` for the key of a pane set to switch
- **THEN** the dialog warns that no xats identity holds the key and the pane must register again

#### Scenario: Old daemon
- **WHEN** the daemon answers HTTP 404 for the lookup
- **THEN** the dialog warns that the daemon is too old to check the identity, and the switch can still be confirmed

### Requirement: 切换保留 pane 目录与托管身份

切换 SHALL 保留目标 pane 的 working directory, worktree 元数据和 xats identity key.  托管 pane SHALL 同步保存新的 tool, 清空旧 native session ID 及旧 agent 的 model, effort 和 client 配置, 同时保留 xats name/team.  新 agent 的运行状态 SHALL 通过已有启动流程记录, 后续 resume SHALL 使用该 agent 的新会话, 不得恢复切换前的会话.

#### Scenario: 在 worktree 中切换为 OpenCode

- **WHEN** 用户将 worktree 目录中的 Claude 或 Codex pane 切换为 OpenCode
- **THEN** OpenCode 在该 pane 原目录启动, worktree 元数据保持不变
- **AND** 该 pane 保留原 xats identity key 和 name/team
- **AND** 新 OpenCode 会话记录后, resume 使用新的 `ses_...` ID

#### Scenario: 切换托管副 pane

- **WHEN** 用户将托管副 pane 切换为 OpenCode
- **THEN** 持久化 pane 配置中的 tool 更新为 `opencode`, 原目录和 xats name/team 不变
- **AND** 主 pane 的 tool, 会话与配置不变

#### Scenario: 从 OpenCode 切换后忽略旧回调

- **WHEN** OpenCode pane 切换为 Claude 或 Codex 后, 旧 OpenCode runtime 尝试写回会话
- **THEN** 旧回调不得覆盖新 agent 的状态, 后续 resume 以切换后的配置为准

### Requirement: xats runtime 类型受控交接

已有 xats 身份的跨类型切换 SHALL 使用显式 prepare/commit/status/abort 交接, 不得将同类型 OpenCode reserve 当作跨类型迁移.  prepare 成功且源配置与 fence 持久保存前, SHALL 不停止旧 pane 或清理旧 owned server.  本地 pending 文件 SHALL 仅当前用户可读写.  启动提示和 CLI 参数 SHALL 只携带交接 ID, attempt ID 及新 runtime 的非敏感坐标, 本地 helper 从 pending 读取身份 key 和 fence.

旧版 xats 明确返回 prepare 路由不存在时, 尚无服务端响应, 新 runtime, 启动 attempt 或受控身份标记的非托管 Claude/Codex 交互切换 SHALL 沿用既有 identity-key 恢复机制.  仅清理该次未被接受的本地 pending, 保留原目录与身份 key.  OpenCode 切换, 已接受的交接, 鉴权失败, 网络错误和业务 not_found SHALL 不使用此路径.

#### Scenario: 旧 daemon 保留原有 Claude/Codex 切换

- **WHEN** 用户将非托管 Claude pane 切换为 Codex, xats 返回精确的 prepare 路由不存在 404
- **AND** 本次交接尚无服务端 fence, 新 runtime, 启动 attempt 或受控身份标记
- **THEN** AoE 清理未被接受的本地 pending, 按原 identity-key 流程切换
- **AND** 不要求重启共享 xats daemon, 原目录与身份 key 保持不变

#### Scenario: 已接受交接不可降级

- **WHEN** 交接已有服务端响应或运行证据, 或失败属于鉴权, 网络, 业务 not_found
- **THEN** AoE 保留交接状态并显示具体错误, 不使用旧切换路径

#### Scenario: Codex 切换为 OpenCode

- **WHEN** pane 的 xats 当前类型仍为 Codex, 用户选择切换为 OpenCode
- **THEN** AoE 先准备保留原身份的交接, 再替换该 pane
- **AND** OpenCode 新 session 验证且交接提交后, 才记录新会话并恢复 xats 连接
- **AND** 后续恢复保留 xats 返回的 OpenCode runtime generation

#### Scenario: commit 响应丢失

- **WHEN** 新 runtime 的 commit 请求结果不确定
- **THEN** AoE 查询原 fence 的 status, 相同 runtime 已 committed 时按成功处理
- **AND** 未确认结果时保留目标 runtime, 不通过重复停止进程来重试

#### Scenario: 新 runtime proof 失败

- **WHEN** proof 持续失败且 status 仍为 prepared
- **THEN** AoE 确认 abort 后才允许结束失败的目标 runtime, 清理该启动 attempt 并允许重试
- **AND** stale 或 aborted 终态 SHALL 明确退出, 即使当前地址与请求地址相同也不得永久重试

#### Scenario: 托管身份的下一次重启

- **WHEN** 托管 pane 完成跨类型 prepare, 后续再次启动或重启
- **THEN** AoE 以已 prepared 的类型推导下一代源类型, 不复用上一轮跨类型的旧 previous_agent_type
- **AND** 相同 prepare 请求重放时保留原 previous fence 与类型参数, 不重复推进代次

#### Scenario: 只重试未完成的切换

- **WHEN** pane 已保存目标 agent 且 native 为空, xats 仍报告原类型, 用户重新打开 `a` 对话框
- **THEN** 该 pane 的当前目标显示为可重试, 即使选择同一目标也提交重试
- **AND** Enter 只重启该 retry pane 与显式修改的其他 pane, 未修改的 sibling 保持运行
