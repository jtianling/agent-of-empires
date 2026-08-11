## MODIFIED Requirements

### Requirement: Durable per-slot agent record

`agent_slot` SHALL 保存每个 managed pane 的 durable mapping 与 pane-level launch config。  每条记录 MUST 至少包含 `instance_id`、`slot`、`agent`、`native_session_id`、`cwd`、`tmux_pane`、`xats_identity_key`、`yolo_mode`、`cross_agent_team`、可选且经过 schema 验证的 `worktree_info`、可选的 `model`、可选的 `model_fingerprint`、`last_seen_at`, primary key 为 `(instance_id, slot)`。

`slot` SHALL 限制在 0 到 3。  `agent` 与 `cwd` SHALL 分别作为 pane Tool 和当前恢复 working directory 的权威持久化值。  `cwd` MAY 随 runtime capture 更新, Worktree metadata 中的不可变 `worktree_path` SHALL 继续作为 cleanup target。  pane-level flags 和 Worktree metadata SHALL 在 reconcile 更新 native session capture 时保留, 不得被 capture 中缺失的值清空。  如果 observed Tool 改变, flags SHALL 按该实际 Tool 的 capability 归一化。

`model` SHALL 保存该 pane 最近一次观测到的 agent 模型标识, 默认为空字符串。  它 SHALL 是 slot 的属性而非会话的属性: fresh restart 换掉 `native_session_id` 之后 `model` SHALL 保持不变。  当一轮 reconcile 无法观测到模型时, 已有的 `model` SHALL 被保留, 不得被空值覆盖。

`model_fingerprint` SHALL 保存 `model` 最近一次是从哪一份 transcript 状态读出来的, 默认为空字符串。  它 SHALL 在每次探测后被写入 (包括未观测到模型的那次), 使下一轮 reconcile 能在不打开文件的情况下判断有没有新内容。  它按 slot 持久化而不是保存在进程内, 因为 reconcile 由多个进程驱动。  capture 写入路径 SHALL NOT 触碰 `model` 与 `model_fingerprint`。

#### Scenario: Upsert by instance and slot
- **WHEN**同一个 `(instance_id, slot)` 被再次写入
- **THEN**已有记录 SHALL 原地更新
- **AND**不得产生重复 row

#### Scenario: Slot range enforced
- **WHEN**写入的 slot 不在 0 到 3
- **THEN** store SHALL 拒绝该写入

#### Scenario: Pane config survives process restart
- **WHEN**一个 slot 保存 Tool、cwd、YOLO Mode、Cross Agent Team 和 Worktree metadata
- **AND** AoE 被关闭后重新打开
- **THEN**所有 pane config SHALL 以相同值读回

#### Scenario: Native session id survives process restart
- **WHEN**一个 slot 保存非空 `native_session_id`
- **AND** AoE 被关闭后重新打开
- **THEN**该 slot SHALL 读回同一个 `native_session_id`

#### Scenario: Identity key survives process restart
- **WHEN**一个 slot 保存非空 `xats_identity_key`
- **AND** AoE 被关闭后重新打开
- **THEN**该 slot SHALL 读回同一个 key

#### Scenario: Model survives process restart
- **WHEN**一个 slot 保存非空 `model`
- **AND** AoE 被关闭后重新打开
- **THEN**该 slot SHALL 读回同一个 `model`

#### Scenario: 空 model capture 不清空已有值
- **WHEN**一个 slot 已保存非空 `model`
- **AND** reconcile 以空 model 再次 upsert 该 slot
- **THEN**该 slot 的 `model` SHALL 保持原值

#### Scenario: Invalid pane metadata is isolated at read boundary
- **WHEN** persisted `worktree_info` 不能通过预期 serde schema
- **THEN** store read SHALL 记录包含 instance 与 slot 的 warning 并跳过该 row
- **AND**同一 instance 的其他有效 slots SHALL 继续返回
- **AND**主启动路径 SHALL 不使用该无效 row 的部分数据
- **AND** store read SHALL 返回 skipped row count
- **AND** restart 或 cold recovery SHALL 在 session error 中展示该诊断

#### Scenario: Incompatible capability flags are repaired at read boundary
- **WHEN** persisted pane 对其实际 Tool 保存了不支持的 YOLO Mode 或 Cross Agent Team
- **THEN** store read SHALL 将不支持的 flag 归一化为 false 并写回该 row
- **AND**该 row SHALL 作为有效 slot 返回, 不得计入 skipped count

#### Scenario: Capture cwd 不改变 Worktree cleanup path
- **WHEN**一个持有 managed Worktree 的 slot 收到不同 `cwd` 的 capture
- **THEN** slot `cwd` SHALL 更新为 capture 值
- **AND** Worktree metadata 中的不可变 `worktree_path` SHALL 保持不变

## ADDED Requirements

### Requirement: Schema healing covers the model columns

早于 `model` 与 `model_fingerprint` 列存在的数据库 SHALL 由既有的幂等 schema 自愈路径补齐, 而不是在主代码路径里加 fallback 逻辑, 与 durable slot pane 列和 identity key 列的处理方式一致。

#### Scenario: Legacy database gains the model columns
- **WHEN** store 打开一个 `agent_slot` 表早于 `model` 与 `model_fingerprint` 列的数据库
- **THEN** schema 路径 SHALL 添加这两列
- **AND**已有 row SHALL 保留
- **AND**这两列 SHALL 默认为空字符串
- **AND**后续 slot 写入 SHALL 成功

#### Scenario: Model column healing is idempotent
- **WHEN** schema 路径对一个已经含有这两列的数据库运行
- **THEN**它 SHALL 保持该表不变
- **AND** SHALL NOT 添加重复列
