## Why

当前 New Session 的 Cross Agent Team 选项只支持 Claude, 已配置 xats 的 Codex 用户仍需离开 AoE 手工运行专用启动流程.  AoE 应让 Codex 会话从创建、恢复和 fork 开始就连接本机 xats app-server, 同时保持现有 YOLO 选择语义不变.

## What Changes

- 在非 Sandbox 的 Codex New Session 中显示并提交 Cross Agent Team 选项.
- 为 Codex 增加工具专用的 xats 启动路径, 在 tmux pane 内完成 pane 预注册, 再通过本机 Codex app-server 启动 remote TUI.
- 保持 Cross Agent Team 与 YOLO Mode 独立, 并让 fresh launch、resume restart 和 fork 复用同一 Codex xats 启动路径.
- 当本机 xats launcher 依赖或 app-server 不可用时显式失败, 不静默退化为普通 Codex.
- 保持 Claude development-channels 和 auto-confirm 行为不变, 并更新 New Session 帮助文本与覆盖测试.

## Capabilities

### New Capabilities

无.

### Modified Capabilities

- `cross-agent-team`: 将 Cross Agent Team 启动能力从 Claude 扩展到已配置本机 xats 的 Codex, 并定义 Codex 的启动、失败和生命周期行为.

## Impact

- `src/tui/dialogs/new_session/`: Codex 字段可见性、帮助文本、提交和交互测试.
- `src/session/instance.rs`: Claude 与 Codex 的工具专用 Cross Agent Team 命令构造、resume、fork 和错误行为.
- `src/session/builder.rs`: 持久化字段语义从 Claude 专用扩展为受支持工具.
- `openspec/specs/cross-agent-team/spec.md`: 增加 Codex xats 场景并保留 Claude 现有契约.
- 运行时依赖本机 xats daemon、Codex app-server 和 cross-agent-teams-mcp 预注册入口, 不读取或写入 token 值.
