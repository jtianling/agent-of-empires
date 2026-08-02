## Context

New Session 当前用一个扁平状态对象同时承载 session 元数据、primary pane 配置和少量 right pane 字段。  `NewSessionData` 只有 `right_pane_tool` 与 `right_pane_path`, YOLO Mode 和 Cross Agent Team 仍是 session 级值。  `PendingRightPane` 同样只携带 tool 与 path, 而 slot restart 通过 `Instance` 的共享启动上下文重建所有 pane。

Worktree 也只有 primary session 的单份解析和清理状态。  因此仅调整渲染顺序会制造一个看似独立、实际仍共享的界面。

Sandbox 是现有 session 级能力。  本变更只移除 New Session 入口, 不删除 CLI、Settings、配置、既有 session 数据或容器生命周期。

## Goals / Non-Goals

**Goals:**

- 建立统一的 pane 级 draft、resolved config 和 durable slot 启动模型。
- 让 primary pane 与可选 secondary pane 独立控制 Tool、Path、YOLO Mode、Cross Agent Team 和 Worktree。
- 让初次启动、restart、resume 和 cold recovery 使用同一份 pane 配置。
- 让每个 managed Worktree 有明确的 pane 所有权和精确清理目标。
- 按已确认顺序重排 New Session, 并在 right pane 未选择 Agent 时折叠其配置。
- 从 New Session 完全移除 Sandbox 状态和入口, 防止隐藏字段继续受默认配置影响。

**Non-Goals:**

- 不删除 Sandbox 子系统, 不改变 CLI `--sandbox`、Settings 或已有 sandbox session。
- 不把 New Session 扩展为任意数量 pane 的编辑器, UI 仍只创建 primary 与一个可选 secondary pane。
- 不改变 tmux 的四 slot 上限、primary pane 跟踪规则或手工创建 raw pane 的语义。
- 不新增 right pane 的额外参数或 command override UI。

## Decisions

### 1. 使用统一 PaneDraft 和 PaneConfig

对话框提交结构使用 session 元数据加两个 pane 字段:

```text
NewSessionData
├── profile
├── title
├── group
├── primary: PaneDraft
└── secondary: Option<PaneDraft>

PaneDraft
├── tool
├── path
├── yolo_mode
├── cross_agent_team
└── worktree_request
```

创建流程把 `PaneDraft` 解析为不可变的 `PaneConfig`, 包含最终 working directory 和可选 `WorktreeInfo`。  primary 和 secondary 走同一组验证及解析函数, 不增加 `right_pane_yolo_mode` 等平行字段。

备选方案是继续扩展 `NewSessionData` 和 `PendingRightPane`。  该方案改动较小, 但会复制字段可见性、提交验证、启动和恢复逻辑, 与本变更要消除的耦合相冲突。

### 2. session 与 pane 的所有权边界

`title`、`group`、profile、hooks、tmux session identity、Cross Agent Team channel 和既有 Sandbox 状态继续属于 session。  Tool、working directory、YOLO Mode、Cross Agent Team enabled 和 Worktree ownership 属于 pane。

primary pane 的持久化 session 数据改为持有 `PaneConfig`, 替代这些值作为多个独立 session 字段的权威来源。  durable slot 保存运行时 pane identity 和恢复所需的 pane 配置。  旧 instance 字段只作为反序列化迁移镜像, primary identity key 在 slot 0 建立前可以作为首次启动 bootstrap, 一旦 slot 存在则以 slot 为准。

### 3. durable slot 保存 pane 启动策略

`agent_slot` 在现有 `agent`、`cwd`、`native_session_id`、`tmux_pane` 和 `xats_identity_key` 之外, 保存 pane 自己的 YOLO Mode、Cross Agent Team 和 Worktree 元数据。  `cwd` 可以被 runtime capture 更新, Worktree metadata 必须单独保存不可变 cleanup path, 两者不得互相替代。  schema 通过现有幂等补列路径升级, 外部持久化数据在读取边界使用 serde schema 验证。

旧 slot 的迁移以旧 session 的共享 YOLO Mode 和 Cross Agent Team 作为输入, 再按每个 slot row 的实际 Tool 过滤不支持的开关, slot 0 也不例外。  旧 primary Worktree 转入 primary pane 配置并记录当时的不可变 Worktree 路径, 旧额外 slot 没有 Worktree 元数据时保持 `None`。  legacy Instance 镜像和 pane struct literal 在进入权威模型时统一经过 `PaneConfig` capability normalization。

持久化值只有 capability flags 与实际 Tool 不一致时属于可恢复错误, 读取边界接受该 row, 将不支持的值归一化为 false 并写回。  无效 Tool、cwd 或 Worktree metadata 属于不可恢复 row, 读取结果返回 skipped count, restart 与 recovery 将该诊断展示到 session error, 同时继续处理 sibling pane。

备选方案是仅在内存中保存 secondary 配置。  该方案会在 AoE restart、`R` restart 和 cold recovery 后丢失独立选择, 不满足需求。

### 4. 命令构建显式接收 PaneConfig

pane 命令构建器不再通过 `Instance::is_yolo_mode()` 或 instance 级 Cross Agent Team 判断装饰所有 pane。  调用方必须传入目标 pane 的 `PaneConfig` 和对应 slot identity。

Claude development channel、Codex xats bootstrap、YOLO flag/env 和 startup auto-confirm 都根据目标 pane 配置计算。  只有该 pane 开启 Cross Agent Team 时才创建和注入 identity key。  primary 与 secondary 可以任意组合启用或关闭。

### 5. secondary Worktree 在 split 前解析

primary Worktree 继续在 session build 阶段解析。  secondary Worktree 在创建 right pane 前, 根据其显式 Path 或 primary 的最终路径解析。  成功创建后才执行 tmux split 和 durable slot 写入。

如果 split 或 slot 持久化失败, 创建流程只回滚本次新建且尚未被 durable state 接管的 secondary Worktree。  复用的 Worktree 永远不由失败回滚删除。  session 删除遍历每个 pane 的 `WorktreeInfo`, 只对 `managed_by_aoe=true` 且 `cleanup_on_delete=true` 的不可变精确路径执行删除。  runtime capture 更新 `cwd` 不得改变 cleanup target。

multi-repo workspace 的计算目标已经存在时, 创建显式失败且不改动该目录。  fork 继承父 session Worktree 时设置 `cleanup_on_delete=false`, 删除 fork 时保留共享 Worktree 和 branch。

### 6. New Session 布局和条件字段

字段顺序固定为 Title、Group、primary Tool、primary Path、primary YOLO Mode 与 Cross Agent Team、primary Worktree、分割线、Right Pane Agent。  选择 Agent 后追加 secondary Path、secondary YOLO Mode 与 Cross Agent Team、secondary Worktree。

YOLO Mode 和 Cross Agent Team 的可见性只由同一 pane 的 Tool 决定。  shell 隐藏两项 agent 专属开关, 但保留 Path 和 Worktree。  Worktree 的 Create new branch 从旧主界面独立行迁入该 pane 自己的 `Ctrl+P` overlay, extra repositories 同样位于该 overlay, 主界面只保留 Worktree 行。

在一次打开的对话框中把 Right Pane 切换为 `none` 只隐藏 secondary draft, 再次选择 Agent 时恢复未提交值。  以 `none` 提交时不产生 secondary pane 配置。

### 7. New Session 不携带 Sandbox 隐藏状态

从 `NewSessionDialog` 和 `NewSessionData` 移除 Sandbox 字段、overlay 和 field layout。  TUI 创建路径显式构造非 sandbox `InstanceParams`, 不读取 `sandbox.enabled_by_default`。  这样恢复入口时可以重新接线, 当前则不存在隐藏但生效的 Sandbox 状态。

CLI 和其他显式 Sandbox 创建路径继续向 `InstanceParams` 传递原有配置。

## Risks / Trade-offs

- [Risk] persisted session 与 SQLite slot 同时迁移可能产生短暂不一致。  → 迁移先补 schema, 再以旧 Instance 值生成 pane 默认值, 并用 focused migration tests 覆盖重复运行。
- [Risk] secondary Worktree 创建成功但 split 失败会留下目录。  → 在 ownership 转交 durable slot 前保留精确的 created-worktree guard, 失败时只清理该 guard 记录的路径。
- [Risk] primary 与 secondary 使用相同 branch 时 Git 可能拒绝第二个 Worktree。  → 沿用现有 reuse confirmation 和显式错误, 不自动改 branch 或复用路径。
- [Risk] 对话框动态高度增加, 小终端可能放不下。  → 继续使用 responsive width, 动态计算可见字段高度, 并为 collapsed 与 expanded 两种状态增加渲染测试。
- [Risk] 旧代码仍读取 instance 级 launch flags。  → 将命令构建入口改为必须传 `PaneConfig`, 通过编译错误穷举迁移调用点。
- [Risk] 历史 slot 的 capability flags 与实际 Tool 不一致会让 pane 在读取时消失。  → 读取边界按实际 Tool 归一化并写回可恢复 flags, 只跳过结构性无效 row, 并把 skipped count 暴露给 restart。

## Migration Plan

1. 增加 pane 配置类型和持久化 schema, 完成旧数据幂等迁移测试。
2. 改造命令构建、slot restart 和 recovery 使用 pane 配置。
3. 改造 Worktree 创建、回滚与删除生命周期。
4. 重构 New Session 状态、布局、输入路由和提交数据。
5. 移除 New Session Sandbox 入口, 保留其他 Sandbox 路径。
6. 运行格式化、静态检查、focused tests 和隔离 tmux E2E。

回滚代码时保留新增 slot 列, 旧版本会忽略它们。  不删除已有 pane 或 Worktree 数据。

## Open Questions

无。
