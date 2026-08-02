## MODIFIED Requirements

### Requirement: Durable per-slot agent record

`agent_slot` SHALL 保存每个 managed pane 的 durable mapping 与 pane-level launch config。  每条记录 MUST 至少包含 `instance_id`、`slot`、`agent`、`native_session_id`、`cwd`、`tmux_pane`、`xats_identity_key`、`yolo_mode`、`cross_agent_team`、可选且经过 schema 验证的 `worktree_info`、`last_seen_at`, primary key 为 `(instance_id, slot)`。

`slot` SHALL 限制在 0 到 3。  `agent` 与 `cwd` SHALL 分别作为 pane Tool 和当前恢复 working directory 的权威持久化值。  `cwd` MAY 随 runtime capture 更新, Worktree metadata 中的不可变 `worktree_path` SHALL 继续作为 cleanup target。  pane-level flags 和 Worktree metadata SHALL 在 reconcile 更新 native session capture 时保留, 不得被 capture 中缺失的值清空。  如果 observed Tool 改变, flags SHALL 按该实际 Tool 的 capability 归一化。

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

### Requirement: An extra pane AoE launches has a durable slot record from launch

AoE 启动额外 managed pane 时 SHALL 在首次 launch 立即写入 durable slot, 不等待首次 capture。  launch-time record SHALL 包含 pane id、Tool、最终 working directory、YOLO Mode、Cross Agent Team、可选 Worktree metadata 和本次创建的 identity key。  `native_session_id` SHALL 为空, 因为 pane 尚未报告 conversation。

secondary pane 的 record SHALL 保存自己的配置, 不得复制 primary pane 的 flags 或 Worktree。  primary pane 在同一时刻没有 slot 0 时, AoE SHALL 写入带 primary pane config 的 slot 0。  已有 slot 0 SHALL 不被 launch-time blank record 覆盖。

shell pane SHALL 同样立即获得 durable slot, 但 SHALL 没有 identity key 或 native session id。

#### Scenario: Extra pane 首次 capture 前已有完整 slot
- **WHEN** AoE 成功启动一个 secondary agent pane
- **THEN**该 pane 的 durable slot SHALL 立即存在
- **AND** slot SHALL 包含该 pane 的 Tool、cwd 和 launch flags

#### Scenario: Primary slot 同步建立
- **WHEN** secondary pane launch 时 slot 0 不存在
- **THEN** AoE SHALL 记录 primary pane 的 durable slot
- **AND** slot 0 SHALL 使用 primary pane config

#### Scenario: Secondary Worktree metadata 从 launch 起可恢复
- **WHEN** secondary pane 启动在 managed Worktree
- **THEN**该 Worktree metadata SHALL 在 split 成功后立即写入 slot
- **AND**首次 capture 前发生 restart 时 SHALL 仍能恢复该配置

#### Scenario: Shell pane 获得无 identity 的 slot
- **WHEN** AoE 启动 managed shell pane
- **THEN**该 pane SHALL 立即获得 durable slot
- **AND** `xats_identity_key` 与 `native_session_id` SHALL 为空

#### Scenario: Launch write 不继承旧 conversation 或 identity
- **WHEN** launch-time record 复用一个此前属于其他 pane 的 slot
- **THEN**该 slot SHALL 保存新 pane 的 identity key 和空 native session id
- **AND**不得继承旧 pane 的 conversation 或 identity key

#### Scenario: Launch write 不沿用同 pane id 的旧 conversation
- **WHEN** launch-time record 写入一个仍带旧 conversation 的 slot, 即使旧 row 使用相同 pane id
- **THEN**该 slot 的 native session id SHALL 清空
- **AND**已到达 volatile capture 的 conversation SHALL 可由下一次 reconcile 恢复

#### Scenario: Capture 只补全观察到的运行时字段
- **WHEN** capture 到达一个已有 launch-time pane config 的 slot
- **THEN** reconciler SHALL 更新 native session id、Tool、cwd 和 tmux pane id
- **AND** SHALL 保留该 slot 已持久化的 identity key 和 Worktree metadata
- **AND** YOLO Mode 与 Cross Agent Team SHALL 保留请求值, 但不受 updated Tool 支持的 flag SHALL 保存为 false

#### Scenario: 单个无效 capture 不阻断 sibling
- **WHEN**一个 pane capture 缺少有效 cwd 或包含不安全 Tool
- **THEN** reconciler SHALL 记录 warning 并跳过该 pane
- **AND**同一 session 的其他有效 pane SHALL 继续同步

#### Scenario: Launch-time slot 保持 sticky
- **WHEN** reconciler 运行时 launch-time record 对应的 pane 仍然存活
- **THEN**该 pane SHALL 保持 launch 时分配的 slot

#### Scenario: 空 native session id 可以 fresh launch
- **WHEN** pane 从 native session id 为空的 launch-time slot 启动或恢复
- **THEN** launch SHALL 降级为 fresh launch
- **AND**不得把空值当作 store corruption

## ADDED Requirements

### Requirement: Pane config schema healing is idempotent

Store schema application SHALL 为旧 `agent_slot` 补齐 pane-level launch config 所需列。  补列 SHALL 幂等执行, SHALL 保留现有 row, 并 SHALL 使用旧 session 的共享 YOLO Mode 和 Cross Agent Team 作为迁移输入, 再按各 slot 的实际 Tool 过滤不支持的开关。  旧 primary Worktree SHALL 连同不可变 cleanup path 迁移到 slot 0, 旧 extra slot 没有 Worktree metadata 时 SHALL 保持为空。

#### Scenario: Legacy store gains pane config columns
- **WHEN** profile database 的 `agent_slot` 缺少 pane config 列
- **THEN** schema application SHALL 添加缺失列
- **AND**原有 row SHALL 保留

#### Scenario: Existing store is left unchanged
- **WHEN** `agent_slot` 已包含所有 pane config 列
- **THEN**重复 schema application SHALL 成功
- **AND**不得添加重复列或清空已有配置

#### Scenario: Legacy shared flags migrate to existing slots
- **WHEN**旧 session 的 YOLO Mode 或 Cross Agent Team 已开启
- **AND**该 session 已有 durable slots
- **THEN** migration SHALL 把旧共享值应用到支持对应能力的 existing pane slots
- **AND**每个 slot, 包括 slot 0, SHALL 使用该 row 的实际 Tool 判断 capability
- **AND**不支持的 Tool SHALL 保存 false
- **AND**之后每个 slot SHALL 可以独立修改
