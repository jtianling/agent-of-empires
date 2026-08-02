## Why

New Session 当前把 right pane 的工具和路径插在 left pane 配置中间, 同时让两个 pane 共享 YOLO Mode、Cross Agent Team 和 Worktree 等 session 级状态。  这会让界面语义和实际启动、重启行为都难以理解, 也无法独立控制两个 pane。

## What Changes

- 将 New Session 对话框重排为 session 元数据、left pane 配置、分割线、可选 right pane 配置四个区块。
- 将 pane 启动配置建模为统一的 pane 级结构, left pane 和 right pane 使用同一种配置模型, 不增加零散的 `right_pane_*` 平行字段。
- 为每个 pane 独立配置 Tool、Path、YOLO Mode、Cross Agent Team 和 Worktree, 并按各自 Tool 决定字段可见性。
- right pane 选择 `none` 时折叠其配置区, 选择 Agent 后才显示完整配置。
- 将 pane 级启动配置持久化到 durable slot, 使 restart、resume 和 cold recovery 保留各 pane 的独立选择。
- 让各 pane 的 managed Worktree 独立创建、复用和清理。
- 为每个 pane 的 Worktree 保存不可变 cleanup path, 防止 runtime cwd 漂移后误删其他目录。
- 已存在的 workspace 目标目录不再隐式复用, 创建会显式失败。  fork 或其他 `cleanup_on_delete=false` 的 Worktree 不再删除共享 branch。
- **BREAKING**: 从 New Session 对话框移除 Sandbox 入口和配置弹层, 并让该入口始终创建非 sandbox session。  CLI、Settings、配置文件、已有 sandbox session 和容器运行能力保持不变。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- `tui`: 调整 New Session 字段顺序、分区、条件显示和 Sandbox 入口。
- `right-pane`: 将 right pane 从共享 session 配置改为独立 pane 配置, 并定义 Path 和 Worktree 的解析行为。
- `cross-agent-team`: 将 Cross Agent Team 开关和 xats identity 生命周期改为 pane 级。
- `git-worktrees`: 支持按 pane 创建、复用、记录和清理 managed Worktree。
- `agent-session-store`: durable slot 保存每个 pane 的启动配置和 Worktree 元数据。
- `agent-resume-restart`: restart、resume 和 cold recovery 使用各 slot 自己的 pane 配置。
- `terminal-category`: shell pane 隐藏 agent 专属开关, 但保留 Path 和 Worktree 能力。
- `sandbox`: New Session 不再暴露 Sandbox 入口, 其他 Sandbox 入口和既有运行能力不变。

## Impact

- 主要影响 `src/tui/dialogs/new_session/`、`src/session/`、`src/tui/home/`、`src/tui/app.rs` 和 `src/db/`。
- durable slot schema 需要向后兼容的幂等列补全或等价迁移, 旧 slot 使用明确的默认 pane 配置。
- Worktree cleanup 改为只信任 pane metadata 中的不可变路径, 不再借用可能由 capture 更新的 `cwd`。
- 需要更新 New Session 单元测试、slot/restart 测试、Worktree 生命周期测试和真实 tmux E2E 验收。
- 不改变 CLI Sandbox、Settings Sandbox、现有 sandbox session 数据和容器生命周期。
