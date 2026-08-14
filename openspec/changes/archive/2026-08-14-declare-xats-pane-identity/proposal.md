## Why

xats 身份恢复在"会话被删除重建"之后永久失效, 而 AoE 是唯一有能力修好它的一侧。

恢复链路的唯一身份指针是 identity key: daemon 拿 pre-registration 行上的 key 反查哪个 agent 行持有它, 才知道该通知里写哪个 (team, name)。AoE 的 key 是 per-(instance, slot) write-once, 会话一旦删除重建就换代, 这把新 key 从未被任何 agent 行持有, daemon 于是查不到持有者, 只能发不带身份的 pane token 通知, agent 只好反问用户"你要我叫什么"。

现有 spec 明确写着 `AoE SHALL NOT read, store, display, or configure a xats team or agent name`, 当时的设计假设是"key 足以承载身份"。这个假设在座位换代场景下不成立: key 换代后, pane id 是新的、tty 会复用、key 查无持有者 —— **没有任何一样东西能把新 pane 连回旧身份**, 而"这个座位坐的是谁"这件事只有用户知道。因此本变更反转该条 spec 决定: 让身份成为 pane 的一项声明式配置。

## What Changes

- **BREAKING (spec 决定反转)**: 撤销 `AoE SHALL NOT read, store, display, or configure a xats team or agent name`。AoE 从此可以存储并传递 xats team / agent name, 但仍**不解释**它们 (与 identity key 相同的不透明值待遇)。
- 每个开启 Cross Agent Team 的 pane 可**声明** xats 身份 (team + agent name), 二者均可留空; 留空表示未声明, 行为与今天完全一致。
- 声明值随 pane 的 durable slot 持久化, 与 identity key 同寿命, 并在 restart / resume / cold recovery 后保持稳定。
- 声明值以环境变量注入启动的 pane, 供能读到自身环境的 agent (claude / kimi / opencode) 在注册时自报身份。
- codex pane 的 pre-registration 调用额外携带声明值, 使 daemon 在 key 查无持有者时仍能发出**带身份**的通知 —— 这是 codex 唯一可用的通道, 因为 codex 的工具进程跑在共享 app-server 里, 读不到自己 pane 的环境。
- **补齐既有 spec 未实现的兼容重试**: `Codex xats pane bootstrap` 已要求 "pre-registration 带新 flag 失败时退回不带 flag 重试一次", 但当前实现直接置 `pre_register_failed=1` 退出。不补齐它, 新增 flag 会让**旧版 xats CLI 拒绝未知 flag (exit 2) 直接打死所有 codex xats pane 的启动**。
- 身份声明值**不是**凭据, 与 identity key 不同, 它可以出现在 argv 上; 但同样不写入 AoE 的持久日志正文。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `cross-agent-team`: 撤销"不得存储 xats team/agent name"的禁令; 新增 pane 级身份声明的存储、注入、传递与稳定性要求; 补齐 codex pre-registration 的兼容重试要求的实现约束。

## Impact

- `src/session/pane.rs`: `PaneConfig` 新增两个字段 (serde 默认空)。
- `src/db/mod.rs`: `agent_slot` 新增两列 + 幂等自愈式 schema 补列; `upsert_agent_slot_config` 与读取路径带上新字段。
- `src/session/instance.rs`: pane 启动命令的环境注入前缀; codex xats bootstrap 脚本新增 flag 与失败重试; restart / resume / cold recovery 路径保留声明值。
- `src/tui/`: New Session 与 pane 配置对话框新增两个输入项 (仅在 Cross Agent Team 开启时可见/可编辑)。
- **跨仓协作**: daemon 侧消费新 flag 的逻辑由 cross-agent-teams-mcp 负责 (xats-main 已确认待 AoE 给出字段形状)。AoE 侧的兼容重试保证两侧发布顺序不再互相阻塞。
- 无数据破坏性迁移: 旧 slot 行读出为"未声明", 行为不变。
