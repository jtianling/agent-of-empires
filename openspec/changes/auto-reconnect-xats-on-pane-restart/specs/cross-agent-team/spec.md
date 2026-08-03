## ADDED Requirements

### Requirement: Restarted Claude panes reclaim their xats identity

一个开启 Cross Agent Team 的非 sandboxed `claude` pane 被 AoE 重新启动时, 如果它复用了此前已持久化的 identity key, AoE SHALL 在该 pane 就绪后向它提交一次 `reconnect` 请求, 使 Claude 收到一个真实的用户输入.

该请求 SHALL 表现为提交给 Claude 的字面文本 `reconnect`, 与用户手工键入并回车等价.  AoE SHALL NOT 代替 Claude 调用任何 xats 工具, SHALL NOT 读取或解释 reconnect 的返回值, 也 SHALL NOT 依据该返回值改变 session 状态.  身份能否恢复由 xats daemon 判定: 恢复成功时 Claude 沿用原有 agent 名称, 无可恢复身份时 Claude 转为提示用户注册.

触发判据 SHALL 是该 pane 的 identity key 为复用而非本次新建.  复用意味着该 pane 此前已启动过, 因而可能持有待恢复的身份; 本次新建则意味着这是 AoE 首次启动该 pane, 此时不存在可恢复的身份, AoE SHALL NOT 提交该请求, 以保留由用户自行指定 agent 名称完成注册的机会.  该判据一次覆盖 resume restart, clean restart, resume recovery 与 clean recovery 全部重启路径, 无需分别判断重启模式.

AoE SHALL 只向本次启动的目标 pane 提交该请求.  同一 session 中的 sibling pane, 用户手工切分的 pane, 以及归属其他 agent 的 pane SHALL NOT 收到任何按键.

#### Scenario: Clean restart 的 Claude pane 自动恢复身份

- **WHEN** 一个开启 Cross Agent Team 的非 sandboxed Claude pane 以 clean restart 重新启动
- **AND** 该 pane 复用了此前已持久化的 identity key
- **THEN** AoE SHALL 在该 pane 就绪后向它提交字面文本 `reconnect` 并回车

#### Scenario: Resume restart 的 Claude pane 同样自动恢复身份

- **WHEN** 一个开启 Cross Agent Team 的非 sandboxed Claude pane 以 resume restart 重新启动
- **AND** 该 pane 复用了此前已持久化的 identity key
- **THEN** AoE SHALL 向该 pane 提交 `reconnect`

#### Scenario: Recovery 重建的 Claude pane 同样自动恢复身份

- **WHEN** 一个开启 Cross Agent Team 的非 sandboxed Claude pane 经 recovery 重建
- **AND** 该 pane 复用了 durable slot 上已存的 identity key
- **THEN** AoE SHALL 向该 pane 提交 `reconnect`

#### Scenario: 首次启动的 pane 不提交请求

- **WHEN** 一个开启 Cross Agent Team 的 Claude pane 首次启动
- **AND** 它的 identity key 由本次启动新建
- **THEN** AoE SHALL NOT 向该 pane 提交 `reconnect`
- **AND** 该 pane SHALL 保持由用户自行注册的原有行为

#### Scenario: Fork 出的 pane 不提交请求

- **WHEN** 一个开启 Cross Agent Team 的 Claude pane 由 fork 产生
- **AND** 它按既有要求获得了一个新建的 identity key
- **THEN** AoE SHALL NOT 向该 pane 提交 `reconnect`

#### Scenario: New-from-selection 建立的 pane 不提交请求

- **WHEN** 一个开启 Cross Agent Team 的 Claude session 由 new-from-selection 建立
- **AND** 它按既有要求获得了一个新建的 identity key
- **THEN** AoE SHALL NOT 向该 pane 提交 `reconnect`

#### Scenario: Hand-started pane 被 adopt 后首次启动不提交请求

- **WHEN** 一个用户手工启动的 pane 被 reconciler adopt 进 slot 且该 slot 尚无 identity key
- **AND** AoE 首次亲自启动该 slot 并为其新建 identity key
- **THEN** AoE SHALL NOT 向该 pane 提交 `reconnect`
- **AND** 此后该 pane 的重启 SHALL 按复用 key 的路径提交请求

#### Scenario: 只有本次启动的 pane 收到按键

- **WHEN** AoE 为一个多 pane session 中的某一个 Claude pane 提交 `reconnect`
- **THEN** 该 session 中其他 pane SHALL NOT 收到任何按键

#### Scenario: Codex pane 不受影响

- **WHEN** 一个开启 Cross Agent Team 的 Codex pane 重新启动
- **THEN** AoE SHALL NOT 向它提交 `reconnect`
- **AND** 它 SHALL 继续使用既有的 pane pre-registration 与 remote bootstrap 路径

#### Scenario: 未开启 Cross Agent Team 的 Claude pane 不受影响

- **WHEN** 一个关闭 Cross Agent Team 的 Claude pane 重新启动
- **THEN** AoE SHALL NOT 向它提交 `reconnect`

#### Scenario: Sandboxed session 不受影响

- **WHEN** 一个 sandboxed Claude session 重新启动
- **THEN** AoE SHALL NOT 向它提交 `reconnect`

#### Scenario: 提交失败不使 session 失败

- **WHEN** 向目标 pane 提交 `reconnect` 的过程失败
- **THEN** AoE SHALL 记录该失败
- **AND** session SHALL NOT 被标记为失败
- **AND** 该 pane SHALL 保持可交互

## MODIFIED Requirements

### Requirement: Auto-confirm Claude startup screens

After launching a Cross Agent Team enabled `claude` pane, AoE SHALL detect Claude's
startup confirmation screens and confirm them by sending Enter, repeating until
Claude is ready or a timeout elapses.

AoE SHALL recognize at least the development-channels warning screen (identified by
text such as "Loading development channels" / "I am using this for local
development") and the workspace-trust screen (identified by text such as "trust
this folder" / "Quick safety check"). For both screens the safe-to-proceed option
is the default selection, so confirmation is a single Enter keystroke.

If the confirmation screens do not appear within the timeout, AoE SHALL stop
auto-confirming and leave the pane interactive without erroring the session.

AoE SHALL treat the appearance of Claude's own input prompt with no question beside
it as the pane's readiness signal, and SHALL make that signal available to actions
that must not run before Claude accepts input. Having answered every known
confirmation screen SHALL NOT stand in for that signal: a pane can run out of known
questions while Claude is still starting, and an action taken then is delivered to
whatever the pane is doing instead. A pane that never produces the readiness signal
before the timeout SHALL NOT have such an action taken on it, and SHALL still be
left interactive without erroring the session.

#### Scenario: Dev-channels screen confirmed

- **WHEN** the launched claude pane shows the "Loading development channels" warning
- **THEN** AoE sends Enter to confirm the highlighted "I am using this for local
  development" option

#### Scenario: Trust-folder screen confirmed

- **WHEN** the launched claude pane shows the workspace-trust confirmation screen
- **THEN** AoE sends Enter to confirm the highlighted "Yes, I trust this folder"
  option

#### Scenario: Timeout leaves pane interactive

- **WHEN** no recognized confirmation screen appears within the auto-confirm timeout
- **THEN** AoE stops auto-confirming
- **AND** the session is not marked as failed

#### Scenario: Readiness follows the input prompt, not the exhausted question list

- **WHEN** a pane has been answered every confirmation screen AoE knows about
- **AND** Claude's own input prompt has not yet appeared
- **THEN** AoE SHALL NOT treat the pane as ready
- **AND** SHALL NOT take a readiness-gated action on it

#### Scenario: Never-ready pane takes no readiness-gated action

- **WHEN** a launched Cross Agent Team claude pane never shows Claude's input prompt
  before the auto-confirm timeout
- **THEN** AoE SHALL NOT take a readiness-gated action on that pane
- **AND** the session SHALL NOT be marked as failed
