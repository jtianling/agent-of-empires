# background-restart Specification

## Purpose
Run StayOnHome session restarts (`c`/`r`) on a background serial worker so the home view stays responsive, while gating conflicting operations on in-flight instances, merging pipeline results back into the live instance, and protecting in-flight state from background refreshes.
## Requirements
### Requirement: StayOnHome restarts run on a background queue

Pressing lowercase `c` (fresh) or `r` (resume) on a selected session SHALL enqueue the restart onto a background serial worker and return control to the TUI event loop without waiting for any part of the restart pipeline (pane kill/respawn, session recovery, auto-confirm, xats reconnect). The event loop SHALL continue processing input and redrawing while the restart executes. Uppercase `C` and `R` (the attach variants) SHALL retain their existing synchronous behavior.

#### Scenario: c returns immediately while the restart runs in background

- **WHEN** the user presses `c` on a running session whose restart pipeline takes multiple seconds (e.g. a Cross Agent Team Claude pane awaiting auto-confirm)
- **THEN** the home view SHALL accept further key input (e.g. cursor movement) without waiting for the restart to complete
- **AND** the restarted session SHALL eventually transition through `Restarting` to `Starting` exactly as a synchronous restart would

#### Scenario: r on a dead session recovers in background

- **WHEN** the user presses `r` on a recoverable session (persisted `agent_slot` rows, tmux session dead)
- **THEN** the cold-start recovery (session rebuild, per-pane relaunch, settle check) SHALL run on the background worker
- **AND** the home view SHALL remain responsive during the rebuild

#### Scenario: Restarts of different sessions queue serially

- **WHEN** the user presses `r` on session A and immediately `r` on session B
- **THEN** both instances SHALL show `Restarting`
- **AND** the worker SHALL execute the two restarts one after the other, each with the same pipeline semantics as today

### Requirement: 组级批量重启

选中组时按小写 `r` 或 `c`, 系统 SHALL 对当前组及所有子组的非 shell session 分别执行现有的恢复重启或全新重启流程, 并停留在列表.  所有成员 SHALL 先进入后台串行队列, 由 worker 决定原地重启, 按持久化 pane 恢复或启动无 pane 记录的成员, 不得在事件循环执行启动流程.  目标选择 SHALL 不受组折叠状态影响, SHALL 排除 shell session, shell 命令覆盖的 session, 正在删除和已经排队或执行重启的 session.  大写 `R` 和 `C` SHALL 保持仅作用于单个 session 的语义.

#### Scenario: 重启折叠组及其子组

- **WHEN** 用户选中包含 agent 和 shell session 的折叠组并按 `r`
- **THEN** 系统 SHALL 对该组及所有子组的可操作 agent session 执行恢复重启
- **AND** 系统 SHALL 保留 shell session, 相似前缀的其他组及当前列表位置

#### Scenario: 清空组内会话上下文后重启

- **WHEN** 用户选中组并按 `c`
- **THEN** 系统 SHALL 对相同范围内的可操作 agent session 执行全新重启
- **AND** 系统 SHALL 不自动 attach 到任何 session

#### Scenario: 跳过忙碌成员并继续处理失败之后的成员

- **WHEN** 组内存在正在删除或重启的 session, 或某个成员重启失败
- **THEN** 系统 SHALL 跳过正在删除或重启的成员, 并继续处理其他可操作成员
- **AND** 重启失败 SHALL 通过相应 session 的错误状态或 `last_error` 呈现
- **AND** 没有可操作成员时按键 SHALL 不产生重启任务

#### Scenario: 无持久化 pane 的成员在后台启动

- **WHEN** 组内成员的 tmux session 不存在, 且没有持久化 pane 记录
- **THEN** worker SHALL 使用正常启动流程处理该成员, 包括启动 hooks 和自动确认
- **AND** `c` SHALL 使用新的会话身份, 启动失败时 SHALL 回滚预分配身份及原恢复 token
- **AND** TUI SHALL 在启动期间保持响应, 其他成员 SHALL 保持排队状态

### Requirement: In-flight restart gates conflicting operations

From the moment a StayOnHome restart is enqueued until its result is applied, the instance SHALL be marked in-flight (`restart_in_flight`) with status `Restarting`, and the home view SHALL reject the following operations for that instance: attach (Enter and number jump), delete (`d`), and any further restart keypress (`c`, `r`, `C`, `R`). Rejected keypresses SHALL be no-ops that do not enqueue, attach, or open dialogs.

#### Scenario: Attach is rejected while restarting

- **WHEN** a session's background restart is in flight
- **AND** the user presses Enter (or the session's jump number) on it
- **THEN** the system SHALL NOT attach to the session

#### Scenario: Delete is rejected while restarting

- **WHEN** a session's background restart is in flight
- **AND** the user presses `d` on it
- **THEN** no delete dialog SHALL open for that session

#### Scenario: Duplicate restart is rejected while restarting

- **WHEN** a session's background restart is in flight
- **AND** the user presses `c`, `r`, `C`, or `R` on it
- **THEN** no second restart SHALL be enqueued or executed

#### Scenario: Operations are re-enabled after the result is applied

- **WHEN** a session's background restart completes and its result has been applied
- **THEN** attach, delete, and restart keys SHALL work on that session again

### Requirement: Restart results merge back into the instance

The background worker SHALL run the restart pipeline on a snapshot of the instance and report a result to the event loop. Applying the result SHALL update the live instance exactly as a synchronous restart would have: identity fields mutated by the pipeline (`agent_session_id`, `fork_pending`, `resume_token`), per-pane failures aggregated into `last_error`, status set to `Starting` on success, and the instance list persisted. A pipeline failure SHALL surface via `last_error` (and `Error` status where the synchronous path sets it today) instead of leaving the instance parked in `Restarting`.

#### Scenario: Fresh restart commits its new conversation identity via the result

- **WHEN** a background fresh restart (`c`) of a session with a pre-allocated session id completes successfully
- **THEN** the applied result SHALL carry the newly allocated `agent_session_id` and cleared `fork_pending`/`resume_token`, matching the synchronous fresh-identity transaction

#### Scenario: Per-pane errors surface after a background restart

- **WHEN** one pane of a multi-pane session fails to respawn during a background restart
- **AND** its sibling panes respawn successfully
- **THEN** the pane error SHALL appear in the instance's `last_error` after the result is applied
- **AND** the instance SHALL NOT remain in `Restarting`

#### Scenario: Worker failure never wedges the instance

- **WHEN** the background restart pipeline fails entirely (including a worker panic)
- **THEN** a result SHALL still be delivered and applied
- **AND** the instance SHALL leave `Restarting` with the failure recorded in `last_error`

### Requirement: In-flight state is protected from background refreshes

While a restart is in flight, periodic background refreshes SHALL NOT overwrite the instance's `Restarting` status or in-flight flag: the status poller SHALL NOT poll or reclassify instances in `Restarting`, and the periodic disk reload SHALL preserve the in-memory `status`, `last_error`, and `restart_in_flight` fields.

#### Scenario: Disk reload during an in-flight restart

- **WHEN** the periodic disk reload fires while a session's background restart is in flight
- **THEN** the session SHALL still show `Restarting` and remain gated afterwards
