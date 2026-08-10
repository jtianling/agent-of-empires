## Why

AoE 目前不支持 kimi code。  用户只能用 shell pane 手工跑 `xats-kimi` 启动函数, 因而拿不到 `Shift+R` 恢复原 conversation、`Shift+C` 开新 conversation 和 xats 身份恢复。  该启动函数按 `metadata.cwd` 加标题前缀从候选池里捞 session 并用 pid lockdir 互斥, 同 cwd 双开时依赖推断而非准确所有权, 与 AoE 已为 OpenCode 建立的 durable slot 模型冲突。

kimi 与 OpenCode 在 xats 眼中是同一类 runtime, 都以 `(base_url, session_id)` 定位。  但 kimi 的 server 是用户常驻的共享单例而非 AoE 自有的 per-pane 临时进程, 因此不能照抄 OpenCode 的 runtime 所有权与 generation fencing, 需要单独建模。

## What Changes

- 将 agent 能力判据从 `src/session/instance.rs` 的字符串比较收回 `AgentDef`, 用 runtime 形态字段区分 AoE 自有 server 与共享 server。
- 将 kimi 纳入 host launch 和 pane 级 Cross Agent Team 能力。
- 为每个 kimi slot 持久化准确的 native session id, 复用既有 durable slot 与 identity key 模型。
- 通过 kimi server 的 instance 注册表发现共享 server, 只连接不启动也不终止, 无活实例时显式失败。
- 在启动 TUI 之前通过 REST 铸造准确 session, 并调用 xats daemon 的 `POST /api/runtime/kimi/commit` 刷新投递坐标。
- `Shift+R` 使用该 slot 的准确 session id 恢复原 conversation, `Shift+C` 铸造新 session 并保留同一个 identity key。
- 校验用户级 kimi MCP 配置, 缺失或不符合要求时显式失败并打印需要粘贴的配置, 不代替用户写入配置文件。
- server 发现、token 读取、session 铸造、MCP 配置校验或 xats commit 任一失败时显式失败, 不静默降级为无身份恢复的启动。

## Capabilities

### New Capabilities

- `kimi-xats-runtime`: 定义 kimi pane 的共享 server 发现、准确 session 生命周期、MCP 配置校验及 xats commit launcher 协议。

### Modified Capabilities

- `agent-registry`: 新增 kimi 条目, 支持 host launch 与准确 `--session` resume; agent 能力判据由注册表字段而非调用点字符串比较决定。
- `cross-agent-team`: kimi pane 可独立启用 Cross Agent Team, 并使用自己的 identity key 与准确 session。
- `agent-session-store`: durable slot 保存 kimi 的准确 native session id。
- `pane-session-capture`: kimi pane 的 session id 由 AoE 在启动前铸造并持久化, 不按 cwd 或 latest 推断。
- `agent-resume-restart`: `Shift+R` 为每个 kimi slot 恢复其持久化 conversation。
- `agent-fresh-restart`: `Shift+C` 为每个 kimi slot 创建全新 conversation, 但保留 xats identity。

## Impact

- 影响 `src/agents.rs`、`src/session/instance.rs` 的能力判据、pane command builder、restart/recovery、session 铸造与 xats 客户端。
- 复用 `src/opencode_xats.rs` 已有的 daemon 发现、bearer、严格 schema、错误分层与脱敏基础设施, 新增 kimi 专用端点客户端。
- 依赖 xats daemon 新增 `POST /api/runtime/kimi/commit`。  该端点契约已与 xats 侧定稿, 但上线前本 change 无法端到端验证。
- 依赖具备所需引擎能力的 kimi 构建, 且该能力不会出现在上游发布版中。  不具备时 AoE 明确拒绝启动 Cross Agent Team kimi pane, 不提供降级模式。
- 不引入 MCP client, 不使用 `npx` CLI transport。
- 不为 sandboxed kimi 增加支持, 不改变 Claude、Codex、OpenCode 或 shell pane 的行为。
- 需要聚焦单元/集成测试覆盖能力判据重构后的既有行为、双 kimi pane、C/R 语义、共享 server 发现和 MCP 配置校验。
