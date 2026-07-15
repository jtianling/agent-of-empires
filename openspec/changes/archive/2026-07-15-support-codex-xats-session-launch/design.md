## Context

AoE 当前把 `cross_agent_team` 持久化在 `Instance` 中, 但运行时通过 `is_cross_agent_team()` 将它限制为 Claude.  Claude 的实现是在普通命令上追加 development-channels flag, 并自动确认启动页面.  Codex 没有对应 flag, xats 的 Codex 集成依赖另一条链路: 在目标 tmux pane 内生成一次性 UUID, 调用 `pre-register-codex-pane`, 再使用 `codex --remote` 连接本机共享 app-server, 并通过 `xats.agent_id` 将进程与 pane claim 关联.

本机现有 `free-xats-codex` shell function 证明这条协议可用, 但它固定开启 YOLO, 不是 PATH 中的可执行文件, 也不能直接作为 AoE command override 使用, 因为 command override 会关闭原生 resume 和 fork 构造.  因此 AoE 需要工具专用的 Codex xats 命令装饰逻辑, 而不是复用 Claude flag 或替换整个命令.

## Goals / Non-Goals

**Goals:**

- 在 New Session 中为非 Sandbox 的主 Codex tool 提供 Cross Agent Team 选项.
- 在目标 pane 内完成 xats 预注册并连接本机 Codex app-server.
- 保持 YOLO Mode 与 Cross Agent Team 独立.
- 让 fresh launch、resume restart、fresh restart 和 fork 复用同一启动装饰逻辑.
- 失败时保留可诊断输出并退出, 不启动不带 xats 的普通 Codex.
- 不改变 Claude development-channels 和 auto-confirm 行为.

**Non-Goals:**

- 自动为 Codex 选择 xats name、team 或 role.
- 自动启动、停止或升级 xats daemon 与 Codex app-server.
- 支持 Sandbox 内访问 host xats 服务.
- 改变 Codex MCP 配置、认证 token 或用户 shell 配置.
- 为右侧辅助 pane 单独增加 Cross Agent Team 选择.

## Decisions

### D1: 将持久化状态与工具专用行为分离

保留现有 `cross_agent_team: bool` 和 `cross_agent_team_channel`, 避免数据迁移.  新增工具判断 helper, 将 Claude flag、Claude auto-confirm 和 Codex bootstrap 分开调用.  `cross_agent_team_channel` 继续只服务 Claude, Codex 不解释该字段.

直接放宽现有 `is_cross_agent_team()` 会让 Codex 收到 Claude development-channels flag, 因此不采用.

### D2: 在 Codex 基础命令外包一层 pane 内 bootstrap

Codex xats bootstrap 在 tmux 创建的目标 pane 内运行, 因为只有该进程环境中的 `TMUX_PANE` 能准确标识待绑定 pane.  bootstrap 按顺序执行:

1. 验证 `TMUX_PANE`、UUID 生成器、本机 app-server 和 xats 预注册入口可用.
2. 生成一次性 `xats_agent_id`.
3. 调用 cross-agent-teams-mcp 的 `pre-register-codex-pane` 入口提交 pane id 与 UUID.
4. `exec codex --remote ws://127.0.0.1:8799 -C <project-path> -c xats.agent_id=<UUID> ...`.

基础 Codex 命令仍由现有 builder 产生, 包括 resume、fork、extra args 和可选 YOLO flag.  xats 装饰器只插入 remote、cwd 和 agent id 上下文, 从而避免复制生命周期逻辑.

### D3: 依赖缺失是启动失败, 不是降级条件

用户显式勾选 Cross Agent Team 后, 缺少 app-server、pane id、UUID 工具或预注册入口时, bootstrap SHALL 输出具体错误并以非零状态退出.  AoE 不回退到普通 Codex, 否则 UI 选择与实际 delivery 能力不一致.

检查发生在 pane 内的实际启动时刻.  在 dialog 打开时探测端口会产生过期状态, 不能替代 launch-time validation.

### D4: Codex xats 与 YOLO 保持正交

YOLO flag 继续由现有 `AgentDef` 和 `yolo_mode` 处理.  Codex xats bootstrap 不隐式添加 `--dangerously-bypass-approvals-and-sandbox`.  这与现有 UI 中两个独立 checkbox 的模型一致, 也避免复用本机固定 YOLO 的 `free-xats-codex` function.

### D5: 只扩展主 tool 的 UI 可见性

`has_cross_agent_team_field()` 支持 `claude` 或 `codex`, 并继续要求 Sandbox 关闭.  帮助文本改为工具中立描述.  提交时仍通过同一个 helper 清除不可用组合, 防止隐藏字段残留状态进入 builder.

### D6: Codex 启动后仍由 agent 完成身份注册

AoE 只建立 remote transport 和 pane claim.  `CODEX_THREAD_ID` 在 Codex 运行后才存在, name 与 team 也不是 New Session 字段, 因此 `register_agent` 继续由 Codex 根据用户指令调用.  自动注册身份不属于本 change.

## Risks / Trade-offs

- [Codex app-server WebSocket 仍是实验接口] -> 将所有 remote 构造集中在一个 helper, 添加命令级测试, 并在失败时保留原始诊断.
- [cross-agent-teams-mcp CLI 行为变化] -> 只依赖公开的 `pre-register-codex-pane` 入口和标准参数, 不读取 daemon 数据库或内部文件.
- [pane bootstrap shell quoting] -> 所有 project path、URL 和动态参数复用现有 shell escaping helper, 测试包含空格和引号路径.
- [预注册成功但 Codex 随后启动失败] -> pending claim 依赖 xats TTL 自动过期, AoE 不执行跨 session 清理.
- [本机工具配置不完整] -> 明确失败并显示缺少的依赖, 不吞掉 stderr, 不静默回退.
- [现有 Claude 行为回归] -> Claude helper 与 Codex helper 分离, 保留现有 Claude unit tests 并增加双工具矩阵测试.

## Migration Plan

现有持久化字段不变, 不需要数据 migration.  升级后已有 Claude session 行为保持不变, 已保存但 tool 为 Codex 且 `cross_agent_team=true` 的 session 会在下次启动时采用新 Codex xats 路径.  回滚只需恢复旧版本, 未引入新的配置或存储字段.

## Open Questions

无阻塞问题.  实现阶段应以本机已配置的 cross-agent-teams-mcp CLI 调用形式为准, 但不得读取 token 值或把 token 写入命令日志.
