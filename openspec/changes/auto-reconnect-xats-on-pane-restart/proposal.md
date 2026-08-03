## Why

开启 Cross Agent Team 的 Claude pane 在重启后不会恢复它原有的 xats 身份.  AoE 已经把 `XATS_IDENTITY_KEY` 正确注入进程环境, xats daemon 也仍保留着该 key 对应的身份, 但恢复动作依赖 Claude 自己在对话里主动调用 xats 的 reconnect 工具, 而这一步实测从不发生.

扫描本机 Claude transcript 得到的实据: 22 个收到 xats `startup_bind_hint` 的 session, 首个 assistant turn 全部零工具调用, 100% 失效, 时间跨度 2026-07-27 至 2026-08-03.  hint 只推送一次且无重试, 因此漏掉即永久失联 —— 一个本该叫 `reviewer` 的 pane 会在长达数小时里对整个 team 不可见, 直到用户手工输入 `reconnect`.

根因是两条 tool 路径的结构性不对称.  Codex 路径在 pane 启动命令里先执行 pane pre-registration 完成绑定, 完全不经过模型; Claude 路径只追加一个 development-channel flag, 之后全靠模型自觉.  Claude 没有等价的进程外绑定接口, 因为它的 xats 身份建立在 MCP 会话内部, 依赖启动前尚不存在的 MCP session 与 UI pid.

## What Changes

- AoE 在重启一个开启 Cross Agent Team 的 Claude pane 后, 自动向该 pane 提交一次 `reconnect` 请求, 使 Claude 收到一个真实的用户输入而必然执行 xats 身份恢复.
- 触发判据取 pane 的 identity key 是复用还是本次新建: 复用意味着该 pane 此前启动过, 可能持有待恢复的身份, 提交 reconnect; 新建意味着这是该 pane 首次启动 (含 fork, clone, 以及 hand-started pane 被 adopt 后的首次 AoE 启动), 不提交, 保留由用户自行指定 agent 名称完成注册的机会.
- 提交时机复用既有的 auto-confirm 流程, 且只在其确认 Claude 输入提示符已出现 (启动确认屏均已翻过) 之后进行.
- 恢复结果的分支判断留在 xats daemon 侧, AoE 不解释也不感知 reconnect 的返回值.
- 非破坏性变更.  首次启动的 pane 行为完全不变; Codex 路径, 其他 agent, sandboxed session, 以及未开启 Cross Agent Team 的 pane 均不受影响.

## Capabilities

### New Capabilities

无.  本次不引入新能力, 变更落在既有的 Cross Agent Team 能力内.

### Modified Capabilities

- `cross-agent-team`: 新增"重启后自动恢复 xats 身份"这一要求, 并扩展既有的 `Auto-confirm Claude startup screens` 要求, 使其在确认 Claude 就绪之后额外承担一次条件性的 reconnect 提交.  既有的 identity key 生命周期要求 (`Identity key is stable across relaunch, restart, and recovery`, `Cloned and forked sessions receive a fresh identity key`, `Panes AoE never launched receive a key at their first relaunch`) 语义不变, 本变更把它们既有的"复用还是新建"语义复用为触发判据.

## Impact

- `src/session/instance.rs`: `auto_confirm_panes` 及其调用链 (`run_auto_confirm`, `auto_confirm_launched_pane`), 以及 identity key 是否为本次新建这一信息的传递路径 (`ensure_xats_identity_key` 及 slot 侧对应逻辑).
- `src/tmux/session.rs`: 需要一个按 pane id 提交字面文本的发送能力; 既有的 `Session::send_keys` 只能打 `{name}:^.0`, `send_keys_to_pane_target` 只能送 key 名.
- 行为面: 重启开启 Cross Agent Team 的 Claude pane 时, 对话里会多出一个 `reconnect` 用户输入及其回应, 这是用户可见的.
- 不涉及数据迁移, 不涉及配置项新增, 不改变任何已持久化的数据结构.
- 不依赖 xats 侧任何变更.  xats 维护者已知悉该问题但不排期, 因此本变更必须在现有 xats 接口下自洽.
- 测试面: 触及 session lifecycle, 需要考虑 e2e 覆盖.  任何触碰 tmux 的测试必须走 `TuiTestHarness` 的私有 socket 隔离.
