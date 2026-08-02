# agent-session-store Specification

## Purpose
TBD - created by archiving change agent-session-recording. Update Purpose after archive.
## Requirements
### Requirement: SQLite store is created under the active profile directory
The system SHALL maintain a SQLite database named `aoe.db` in the active profile directory (alongside `sessions.json`). The database SHALL be created and have its schema applied through the existing `src/migrations/` system, not by ad-hoc code in the main path.

#### Scenario: Database created on first run
- **WHEN** AoE starts and `aoe.db` does not yet exist in the profile directory
- **THEN** the migration system SHALL create `aoe.db`
- **AND** apply the schema (all required tables) before any store read or write

#### Scenario: Migration is idempotent
- **WHEN** the schema migration runs and `aoe.db` already has the current schema
- **THEN** the migration SHALL complete without error
- **AND** SHALL NOT duplicate or drop existing rows

#### Scenario: Store path is profile-scoped
- **WHEN** two different profiles are active in turn
- **THEN** each profile SHALL use its own `aoe.db` under that profile's directory
- **AND** records SHALL NOT leak between profiles

### Requirement: Volatile per-pane capture table
The store SHALL provide a `pane_live` table holding the latest capture per tmux pane: `tmux_pane` (text, primary key), `agent` (text), `native_session_id` (text), `cwd` (text), `updated_at` (timestamp). Writes SHALL upsert by `tmux_pane`.

#### Scenario: Upsert by tmux pane
- **WHEN** two captures arrive for the same `tmux_pane` with different `native_session_id`
- **THEN** the row for that `tmux_pane` SHALL reflect the most recent capture
- **AND** there SHALL be exactly one row for that `tmux_pane`

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

### Requirement: Append-only event stream
The store SHALL provide an `events` table recording status and lifecycle events: `id` (autoincrement), `instance_id` (text), `slot` (integer, nullable), `kind` (text, e.g. `status`, `capture`, `adopt`), `detail` (text, nullable), `created_at` (timestamp). Event rows SHALL NOT be modified once written.

The stream SHALL be bounded. On schema application the store SHALL drop event rows older than a retention window, and SHALL keep at most a fixed number of the most recent rows per instance, so neither an old quiet database nor one busy instance can grow the table without limit. When a prune removes rows, the store SHALL reclaim the freed space so an already-oversized database shrinks on disk.

#### Scenario: Event appended
- **WHEN** the system records an event for an instance
- **THEN** a new row SHALL be inserted into `events` with a monotonically increasing `id`
- **AND** existing event rows SHALL NOT be modified

#### Scenario: Events older than the retention window are dropped
- **WHEN** the schema is applied to a store holding event rows older than the retention window
- **THEN** those rows SHALL be removed
- **AND** rows inside the window SHALL be retained

#### Scenario: Per-instance row cap is enforced
- **WHEN** an instance has more recent event rows than the per-instance cap
- **AND** the schema is applied
- **THEN** only the most recent rows up to the cap SHALL be retained for that instance
- **AND** another instance's rows SHALL NOT be removed to make room

#### Scenario: Pruning reclaims space
- **WHEN** a prune removes event rows
- **THEN** the store SHALL reclaim the freed space rather than leaving the file at its previous size

#### Scenario: A store within its bounds is left alone
- **WHEN** the schema is applied to a store whose events are inside the retention window and under the cap
- **THEN** no event rows SHALL be removed
- **AND** no space reclamation SHALL be performed

### Requirement: Store cleanup on session deletion
When a session is deleted, the system SHALL remove that session's `agent_slot` rows, its layout snapshot, its event rows, and any `pane_live` rows whose `tmux_pane` belonged to that session.

#### Scenario: Deleting a session purges its durable records
- **WHEN** a session with `instance_id = X` is deleted
- **THEN** all `agent_slot` rows with `instance_id = X` SHALL be removed
- **AND** all `events` rows with `instance_id = X` SHALL be removed

#### Scenario: Another session's records survive
- **WHEN** a session with `instance_id = X` is deleted
- **THEN** rows belonging to other instances SHALL be left intact

### Requirement: Schema heals columns missing from legacy databases
The store's schema application SHALL be safe to run against databases created by earlier versions whose tables predate later-added columns. Because tables are created with `CREATE TABLE IF NOT EXISTS` (which does not alter an existing table), the schema application SHALL, after creating the tables, ensure that `agent_slot` has the `tmux_pane` column and add it (`ALTER TABLE agent_slot ADD COLUMN tmux_pane TEXT NOT NULL DEFAULT ''`) when it is absent. This backfill SHALL be idempotent (a no-op when the column already exists), SHALL NOT recreate the table or lose existing rows, and SHALL run on every store open so that every profile's database (active and lazily created) self-heals.

#### Scenario: Legacy agent_slot gains the missing column on open
- **WHEN** a database has an `agent_slot` table without the `tmux_pane` column (created by an earlier version)
- **AND** the store schema is applied (store opened)
- **THEN** the `agent_slot` table SHALL afterward have a `tmux_pane` column
- **AND** existing `agent_slot` rows SHALL be preserved (column added, table not recreated)

#### Scenario: Durable upsert succeeds after backfill
- **WHEN** a legacy database has been opened and its `agent_slot` column backfilled
- **AND** the reconciler upserts an `agent_slot` record (with a `tmux_pane` value)
- **THEN** the upsert SHALL succeed and the row SHALL be readable
- **AND** the reconciler SHALL no longer fail with `no such column: tmux_pane`

#### Scenario: Backfill is idempotent
- **WHEN** the store schema is applied to a database whose `agent_slot` already has the `tmux_pane` column (a fresh database, or one already healed)
- **THEN** the schema application SHALL succeed without error
- **AND** SHALL NOT attempt to add a duplicate column

### Requirement: An unreadable store is quarantined, not fatal
When the schema cannot be applied because the database is corrupt or is not a database, the store SHALL move the file aside under a timestamped name and create an empty database in its place, so the profile remains usable.

The store SHALL NOT attempt to repair or salvage the quarantined file, and SHALL NOT delete it. Failures that are not corruption (permissions, locking, a missing directory) SHALL continue to surface as ordinary errors.

#### Scenario: Corrupt database is moved aside and recreated
- **WHEN** the store is opened with schema application against a corrupt database file
- **THEN** the corrupt file SHALL be preserved under a timestamped name
- **AND** a new empty database SHALL be created in its place
- **AND** the open SHALL succeed

#### Scenario: A file that is not a database is quarantined the same way
- **WHEN** the store is opened with schema application against a file that is not a SQLite database
- **THEN** the file SHALL be preserved under a timestamped name
- **AND** the open SHALL succeed against a new empty database

#### Scenario: Startup survives a corrupt store
- **WHEN** AoE starts in a profile whose database is corrupt
- **THEN** it SHALL start
- **AND** it SHALL NOT abort with a database error

#### Scenario: Non-corruption failures are not quarantined
- **WHEN** the store cannot be opened for a reason other than corruption
- **THEN** the file SHALL NOT be moved aside
- **AND** the error SHALL be returned to the caller

### Requirement: Quarantine is surfaced to the user
When a database is quarantined, the system SHALL warn the user, naming the path the unreadable file was preserved at, rather than recovering silently.

#### Scenario: User is told where the quarantined file went
- **WHEN** a profile's database is quarantined during startup
- **THEN** the user SHALL be shown a warning identifying the preserved file's path

### Requirement: Reconcile preserves a slot's identity key
The reconciler assigns panes to slots and rewrites durable slot records from pane captures. A capture carries no identity key, so reconcile SHALL preserve the key already recorded on the slot rather than overwriting it with an empty value.

#### Scenario: Capture does not clear the slot's key
- **WHEN** a durable slot record carrying an identity key is rewritten from a new pane capture
- **THEN** the record SHALL retain its identity key

#### Scenario: Slot with no key stays empty
- **WHEN** a durable slot record with no identity key is rewritten from a pane capture
- **THEN** the record's identity key SHALL remain empty

### Requirement: Schema healing covers the identity key column
Databases created before the identity key column existed SHALL be healed by the existing idempotent schema path rather than by ad-hoc fallback logic in the main code path, matching how the durable slot pane column is healed.

#### Scenario: Legacy database gains the column
- **WHEN** the store opens a database whose `agent_slot` table predates the `xats_identity_key` column
- **THEN** the schema path SHALL add the column
- **AND** subsequent slot writes SHALL succeed

#### Scenario: Healing is idempotent
- **WHEN** the schema path runs against a database that already has the column
- **THEN** it SHALL leave the table unchanged

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
