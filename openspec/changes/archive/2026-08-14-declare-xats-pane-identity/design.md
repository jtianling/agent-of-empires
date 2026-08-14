## Context

xats 的身份恢复建立在 identity key 上: daemon 用 pre-registration 行的 key 反查持有它的 agent 行, 才知道通知里该写哪个 (team, name)。AoE 的 key 是 per-(instance, slot) write-once, 删除并重建会话时随 slot 一起消失并重铸, 于是新 key 查无持有者, daemon 只能发不带身份的 pane token 通知。

座位换代后, 三样东西都指不回旧身份: key 是新的、pane id 是新的、tty 会被回收复用 (这条已有事故先例, 不可用于认亲)。**"这个座位坐的是谁"只有用户知道**, 而 AoE 是唯一能在启动前把这个信息交给 daemon 的组件。

当前实现约束:
- pane 级配置已经全部下沉到 `PaneConfig` 与 `agent_slot` (identity key 本身就是这样从 instance 搬到 slot 的), 新字段应当沿用同一条路。
- codex bootstrap 是一段拼接出来的 `sh -c` 脚本, 用户输入进入它必须转义。
- 既有 spec 已要求 pre-registration "带新 flag 失败时退回不带 flag 重试一次", 但当前代码只写了 `|| pre_register_failed=1` 后直接 `exit 1` —— 该要求未实现。

## Goals / Non-Goals

**Goals:**
- 让用户能按 pane 声明 xats 身份 (team + agent name), 并持久化到该 pane 的 slot。
- 让声明值在每次启动时到达 pane: 环境变量 (通用) + codex pre-registration 参数 (codex 专用通道)。
- 让未声明的 pane 的启动命令与今天**逐字节相同**。
- 补齐 pre-registration 的兼容重试, 使新增 flag 不可能打死 codex pane 的启动。

**Non-Goals:**
- 不实现 daemon 侧对新参数的消费 (归 cross-agent-teams-mcp, 由 xats-main 负责)。
- 不做全局默认 team/name 配置 —— 角色名天然是 per-pane 的, 全局默认没有正确取值。
- 不改 identity key 的铸造与寿命语义 (会话重建换代仍是 by design)。
- 不让 AoE 解释、校验或推断身份语义 (不查重、不猜、不用 session title 兜底)。

## Decisions

### 1. 存储在 slot, 不在 instance

新增 `agent_slot.xats_team` / `agent_slot.xats_agent_name` 两列, 与 `xats_identity_key` 同行。

理由: 同一 session 的两个 pane 是两个不同的 agent (现场就是 `monkeys-coder` / `mvr-coder`), 身份必须各自独立; identity key 当初正是因为同样的理由从 instance record 搬到 slot 的, 沿用既有形状而不是另造一处。

替代方案 (否): 存在 `sessions.json` 的 instance 级字段 —— 无法表达 sibling 独立, 且与 key 的存储位置不一致, 两处状态会漂移。

### 2. schema 演进用既有的幂等补列自愈, 不写 migration

沿用 `ensure_schema` 的 `ADD COLUMN ... DEFAULT ''` 自愈路径。旧行读出为空 = 未声明。

理由: 这是纯增列且有语义安全的默认值, 不涉及数据搬迁。历史上 slot 表缺列导致 upsert 被静默吞掉过 (旧 6 列缺 `tmux_pane`), 所以补列必须走自愈而不是假设 schema 已就位。

### 3. 空值即未声明, 不注入空变量

未声明的部分不注入环境变量, 也不追加到 pre-registration 参数。

理由: daemon 无法区分"没声明"和"声明成空字符串"。注入空值会把一个本可判定的状态变成歧义状态。

### 4. 两条投递通道, 因为 codex 读不到自己的环境

- **环境变量** `XATS_TEAM` / `XATS_AGENT_NAME`: 通用通道, claude / kimi / opencode 的工具进程能读到自己 pane 的环境, 可据此自报身份。
- **pre-registration 参数**: codex 专用。codex 的工具进程跑在共享 app-server 里, `printenv` 读到的是启动那个 app-server 的 shell 的环境, 不是自己 pane 的 —— 这一点已实测确认 (pane 进程环境里有 key, codex 自己却读不到)。所以对 codex 必须由 bootstrap 在**启动前**把值交给 daemon。

两条都做, 而不是只做 codex 那条: 会话重建后 claude pane 同样会丢身份并反问用户, 只是它下一代能靠自报 key 自愈; 通用通道让它连第一次都不用问。

### 5. 兼容重试: 只退掉本变更新加的那两个 flag, 绝不退掉 identity key

pre-registration 改为: 带上声明身份 flag 调用一次; 非零退出则**保留 `--identity-key-env` 与 `--ttl`, 只去掉 `--team` / `--agent-name`** 重试一次; 两次都失败才判定失败并退出。未声明身份的 pane **不生成任何重试分支**, 脚本与变更前逐字节一致。

理由分两半:

- 需要垫子: xats CLI 自 0.7.7 起把未知 flag 变成硬错误 (exit 2), 而 AoE 走 `npx --no-install ...@latest`, 跑的是本机缓存那一版 —— AoE 无法保证对端认识新 flag。没有垫子, 新增 flag 会让所有已声明身份的 codex pane 启动失败, 只留一行 `[xats] Failed to pre-register the Codex pane.`。
- 但垫子不能垫掉 key: 既有要求 `Codex xats bootstrap failure is explicit` 明令 "A pre-registration that fails SHALL NOT be retried without the pane's identity key", 理由是**没有 key 的 pane 会注册成功但永远收不到重新注册的提示 —— 它看起来是健康的, 却终生留在 Cross Agent Team 之外**。这比"启动失败"隐蔽得多, 也严重得多。

所以退化的边界是"本变更新增的、且不承载身份的 flag", 而不是"退到最小调用"。**只有新加的东西才可以被退掉** —— 这条原则同时解释了为什么未声明的 pane 一次重试都不做: 它没加任何新东西, 也就没有可退的余地, 退了就只能退掉 key。

判定只依据退出码, 不解析 CLI 的错误文本 (文本是对端的实现细节, 会变)。脚本继续用 `|| failed=1` 的写法而非 `set -e`, 以免继承来的 `SHELLOPTS` 改变控制流。

**主 spec 的既有矛盾在此一并消解**: `Codex xats pane bootstrap` 原文写 "retry it once as the exact pre-change call (no identity-key flag, no TTL)", 与上面那条 "SHALL NOT be retried without the identity key" 直接打架, 而实现选择了后者 (代码里根本没有重试, 且有一条断言守着"只能有两个调用点")。本变更按后者定稿并改写前者。

### 6. 用户输入进 shell 前必须转义并校验

声明值来自用户自由输入, 会同时进入 tmux 启动 argv 与 bootstrap 的 `sh -c` 脚本。

- 转义: 复用既有 `shell_escape`, 不手工拼引号。
- 校验 (系统边界): 拒绝控制字符与换行, 限定长度上限。校验发生在写入配置时, 而不是拼命令时 —— 让坏值根本进不了持久层。

不做的: 不校验 team/name 的"合法性"语义 (是否存在、是否重名), 那是 daemon 的判断, AoE 不解释这两个值。

**但要校验字符集, 而且必须在录入时校验** (xats-main 提出, 我认为是本变更最容易被忽略的一个坑): xats 把 `:` 读作 `name:device` 的分隔符, 把 `()` 读作 `name(team)` 的写法, 所以 name 禁 `: ( )`, team 禁 `( )`。而 jt 平时的口头写法恰恰是 `mvr-coder(monkeys)`。

关键在于**这类失败不能靠退化兜底**: bootstrap 只看得到退出码, 分不清"daemon 不认识这个 flag" (该退化) 和"daemon 认识但说你填错了" (不该退化), 于是会去掉声明重试并**成功** —— pane 看起来健康, 声明却被悄悄丢掉, 直到下次会话重建才发现。这正是 `failure is explicit` 那条要求所描述的病的又一种得法。

解法不是让 bootstrap 去分辨这两种失败 (它分辨不了), 而是**让第二种失败不可能发生**: 在 TUI 录入时就按 xats 的规则拒收, 坏值根本进不了持久层, 于是退化路径只可能由"CLI 不认识 flag"触发。这是"在系统边界验证外部数据"的一个具体形态。

这确实让 AoE 编码了一点 xats 的语法规则 —— 但它编码的是**通道能否承载**, 不是**值是什么意思**, 与"AoE 不解释这两个值"不冲突。

### 7. 声明值不是凭据, 与 key 区别对待

identity key 必须避开 argv 与日志 (已有要求)。声明的 team/name 是公开的角色名, 可以出现在 argv 和日志里 —— 这是它能作为 codex 通道的前提。文档与实现都要显式区分这两类值, 避免后来者把它们当成同一种东西一起收紧或一起放宽。

### 8. 配置入口跟随 Cross Agent Team 开关

新字段出现在用户能开关 Cross Agent Team 的同一处 (New Session 与 pane 配置对话框), 按 pane 呈现, 关闭时不可编辑。

理由: 仓库硬规则要求"每个可配置字段都能在设置 TUI 编辑"针对的是全局 config 结构体; 本字段是 per-pane 的 session 状态, 与 YOLO / Cross Agent Team 开关同类, 归属 New Session 对话框。不新增全局默认项。

**由此暴露的两条真实边界 (实现时核实, 非推测)**:

- AoE 今天**没有**"重新配置一个已创建 pane"的流程 —— 产出 `PaneConfig` 的对话框只有 `new_session` 一个。所以声明只能在创建会话时录入, 之后不可改也不可清。本变更**不新增**编辑流程 (那是独立的一块工作), 但 DB 层"空值不覆盖非空"的规则与这个现实是自洽的: 今天没有任何合法路径需要清空。
- `add_pane` 对话框没有 Cross Agent Team 开关, 额外 pane 的该开关由 pane 默认值解析而来, 所以**后加的 pane 目前无法录入声明**。这对本变更的原始动机 (一个 session 里两个 codex, 各自一个角色名) 只在"两个 pane 在创建会话时一次性配好"的形态下成立。后加 pane 的声明入口是已知缺口, 留给后续。

## Risks / Trade-offs

- **新 flag 让本机缓存的旧版 xats CLI 拒绝, 打死所有 codex xats pane 启动** → 决策 5 的兼容重试; 且未声明身份的 pane 根本不追加 flag, 把爆炸半径限制在"已声明"的 pane 上。
- **用户输入注入 bootstrap 脚本** → 决策 6 的转义 + 边界校验; 测试须覆盖含引号与空格的值。
- **声明的身份与 daemon 里一个活着的载体重名, 可能被误判为 takeover** → 这是 daemon 侧的仲裁规则, 已明确提请 xats-main 定为"拒绝并记日志, 不静默交接"。AoE 侧只负责如实传递, 不自行查重 (它没有全局视图)。
- ~~flag 名与 xats 最终定稿不一致~~ → 已对齐 (`--team` / `--agent-name`)。
- **非法字符导致声明被静默丢弃** → 决策 6 的录入期字符集校验; 这是本变更最隐蔽的失败模式, 见该决策。
- **声明值随 slot 消失**: 删除会话仍会连声明一起删掉, 下次重建需要用户重填一次。这是可接受的 —— 重填一次配置, 与今天在 agent 聊天框里报一次名字成本相同, 但此后每次重启都不再需要。

## Migration Plan

1. `agent_slot` 幂等补列 (`ADD COLUMN ... DEFAULT ''`), 旧行读作未声明。
2. 无回填、无数据搬迁: 现存 pane 保持今天的行为, 直到用户主动声明。
3. 回滚: 移除新列的读写即可, 列本身留着无害 (读作空 = 未声明)。
4. 与 xats 的发布顺序解耦: AoE 先发不会打断任何东西 (未声明的 pane 命令不变, 已声明的 pane 在旧 CLI 上退回不带声明的调用); daemon 侧上线后声明值即刻生效。已由 xats-main 确认: 旧 CLI 的 `rejectUnknownPreRegisterFlags` 在**联系 daemon 之前**就 exit 2, 所以第一次调用根本不落库, daemon 只会看到一次干净写入, 不会产生双候选。

## 已定稿 (原 Open Questions)

- ~~pre-registration 的最终 flag 名~~ 已与 xats-main 定稿: `--team` / `--agent-name`。后者由 `--name` 改名而来, 理由是同一个 CLI 里已有语义完全不同的 `--agent-id` (装的是 codex 的 launch uuid), 光秃秃的 `--name` 读起来像是它的名字; 改名后 flag 与环境变量一一对应。
- ~~环境变量名~~ 已定稿: `XATS_TEAM` / `XATS_AGENT_NAME` 不变, xats 侧会在 `register_agent` 的工具描述里让所有 runtime 自读这两个变量自报身份。
- 退化重试保留 `--identity-key-env` 还有第二个理由 (xats-main 提供): xats 侧规则是"未过期且带 key 的 pre-reg 行, 只有拿着同一把 key 才能替换"。丢了它, 重试有可能被判成"别人要抢这个 pane 的行"而拒掉。
