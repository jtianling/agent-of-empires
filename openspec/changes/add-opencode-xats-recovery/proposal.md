## Why

AoE 当前可以双开 OpenCode pane, 但 OpenCode 不能在 `Shift+R` 时恢复各自的原生 conversation, 也不能在 `Shift+C` 或 `Shift+R` 后可靠恢复原 xats 身份。  同 cwd 的两个 pane 还会因为 latest-session 推断而串线, 因此需要以 durable slot 为边界建立准确的 session 与 runtime fencing。

## What Changes

- 将 OpenCode 纳入 host launch 和 pane 级 Cross Agent Team 能力。
- 为每个 OpenCode slot 持久化准确的 native session id 与单调 xats runtime generation。
- 在 OpenCode 进程启动前调用 xats daemon 的 loopback REST control API reserve generation, 并在准确 session ready 后提交 delivery。
- `Shift+C` 为每个 pane 创建全新 OpenCode conversation, 同时复用该 slot 的 xats identity key。
- `Shift+R` 使用该 slot 的准确 session id 恢复原 OpenCode conversation, 同时复用 xats identity key。
- 通过 AoE 管理的 OpenCode server/attach runtime 捕获当前 pane 的准确 session id, 禁止使用 cwd 或 latest session 推断。
- xats reserve、OpenCode server 或 session 初始化失败时显式失败, 不静默降级为无 fencing 或无身份恢复的启动。

## Capabilities

### New Capabilities

- `opencode-xats-runtime`: 定义 OpenCode pane 的 per-slot runtime generation、准确 session 生命周期及 xats reserve/commit launcher 协议。

### Modified Capabilities

- `agent-registry`: OpenCode 支持直接 host launch 和准确 `--session` resume。
- `cross-agent-team`: OpenCode pane 可独立启用 Cross Agent Team, 并使用自己的 identity key、endpoint 和 generation。
- `agent-session-store`: durable slot 保存并原子推进 OpenCode xats runtime generation。
- `pane-session-capture`: OpenCode runtime 按实际 pane 捕获准确 session id, 不按 cwd 猜测。
- `agent-resume-restart`: `Shift+R` 为每个 OpenCode slot 恢复其持久化 conversation。
- `agent-fresh-restart`: `Shift+C` 为每个 OpenCode slot 创建全新 conversation, 但保留 xats identity。

## Impact

- 影响 `src/agents.rs`、pane command builder、restart/recovery、SQLite slot schema、session capture 和 OpenCode runtime wrapper。
- 使用 xats daemon 已提供的 reserve/commit REST adapter, 不增加全局 CLI 运行时依赖。
- 影响 New Session 的 per-pane Cross Agent Team capability 过滤, 不新增视觉样式。
- 需要聚焦单元/集成测试覆盖双 OpenCode pane、C/R 语义、generation fencing、命令安全和旧数据库 schema healing。
