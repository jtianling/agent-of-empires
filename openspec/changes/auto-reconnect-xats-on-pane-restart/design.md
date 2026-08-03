## Context

AoE 已经为每个开启 Cross Agent Team 的 pane 铺好了身份恢复所需的全部条件: identity key 由 AoE 铸造并持久化在 durable slot 上, 每次启动注入 `XATS_IDENTITY_KEY`, xats daemon 侧也保留着该 key 到身份的映射.  链条上唯一没有执行者的一环是"发起恢复"这个动作本身.

Codex 与 Claude 在这一环上结构不对称.  Codex pane 的启动命令是一段 shell 脚本, 在 `exec codex` 之前先完成 pane pre-registration, 并把同一个 UUID 通过配置项交给 Codex, 绑定全程不经过模型.  Claude pane 的启动命令只是追加一个 development-channel flag, 恢复动作依赖 Claude 自己在对话里调用 xats 工具.

Claude 之所以不能照搬 Codex 的做法, 是因为它的 xats 身份建立在 MCP 会话内部, 绑定依赖 MCP session 与 UI pid, 二者在进程启动前都不存在, 没有任何命令行参数能在启动时声明"你的身份是 X".  实测证据 (22 个 session 全部零工具调用) 表明, 把这一步留给模型自觉在现实中等于没有执行者.

约束:
- 本变更不得依赖 xats 侧任何改动.  xats 维护者已知悉该问题, 但明确不排期.
- `auto_confirm_panes` 的现有注释反复强调按键只能打调用方刚启动的 pane, 绝不能是"session 里所有 pane".  该处历史上踩过坑.
- 本机运行着大量真实 AoE session, 验证手段受限, 不能跑全量测试, 不能执行 ad-hoc tmux 命令.

## Goals / Non-Goals

**Goals:**

- 让重启后的 Claude pane 必然发起一次 xats 身份恢复, 不依赖模型自觉.
- 首次启动的 pane 保持原样, 把 agent 命名权留给用户.
- 一个判据覆盖全部重启路径 (clean restart, resume restart, clean recovery, resume recovery), 不逐条枚举重启模式.
- 不引入新配置项, 不引入数据迁移, 不改变任何已持久化结构.

**Non-Goals:**

- 不替 Claude 调用 xats 工具, 不解析 reconnect 结果, 不让 AoE 感知 agent 名称或 team.  既有要求已明令 AoE 不得读取, 存储, 展示或配置 xats team 与 agent 名称, 本变更不触碰该边界.
- 不改动 Codex 路径.
- 不为 Claude 构建进程外绑定通道.  那需要 xats 或 Claude 侧提供新接口, 超出本变更范围.
- 不解决 xats hint 只推一次且无重试的问题.  那是 xats 侧的事.

## Decisions

### 决策一: 用 identity key 是否为本次新建作为触发判据

选它而不是判断 `RestartMode`.

既有 spec 已经确立了这条语义: `Identity key is stable across relaunch, restart, and recovery` 要求四种重启路径全部复用既有 key; `Cloned and forked sessions receive a fresh identity key` 要求 fork 与 new-from-selection 必须铸造新 key; `Panes AoE never launched receive a key at their first relaunch` 要求被 adopt 的手工 pane 在 AoE 首次亲自启动它时才获得 key, 并明说这类 pane "costs one extra manual registration".

于是"key 是复用还是新建"恰好就是"该 pane 是否可能持有待恢复的身份", 而且四类边界情况的正确行为与既有 spec 逐条吻合, 无需额外特判:

| 场景 | key | 是否提交 reconnect | 与既有 spec 的关系 |
| --- | --- | --- | --- |
| 各类重启与恢复 | 复用 | 是 | 正是要修的场景 |
| pane 首次启动 | 新建 | 否 | 保留用户命名权 |
| fork / new-from-selection | 新建 | 否 | 本就是新身份, 不该冒领旧身份 |
| adopt 后 AoE 首次启动 | 新建 | 否 | 既有 spec 已认可这一次手动注册成本 |

替代方案 `RestartMode::Fresh` 被否: 它只覆盖 clean restart 一条路径, resume restart 与两条 recovery 路径都会漏掉, 且它无法区分 fork (fork 也走 fresh 语义却应当被排除).

替代方案"每次启动都提交"被否: 它会在首次启动时抢走用户的命名权, 与用户的明确要求相悖.

### 决策二: 触发时机绑定在 auto-confirm 的输入提示符信号上

`auto_confirm_panes` 里每个 pane 有两条 settle 分支.  一条是 `shows_claude_input_prompt` 为真, 意味着 Claude 自己的输入提示符已出现在屏幕上且旁边没有待答问题; 另一条是"AoE 已知的确认屏全部答完".

只有前者可作为提交依据.  后者仅说明 AoE 问完了它认识的问题, Claude 完全可能仍在启动中, 此时送进去的文本会被当时正在运行的东西吞掉或错解.  这是 `auto_confirm_panes` 注释里已经点明的同一类风险, 本变更沿用它的判断而不是另立标准.

选择这个宿主还有两个现成好处: 该函数同步运行在 attach 之前, 与 `tmux attach` 不存在争抢 capture/send 子进程的并发问题; 且它天然只持有调用方刚启动的那批 pane, 满足"只打本次启动的 pane"这条硬约束, 不需要重新推导目标集合.

替代方案"另起一个轮询等待 Claude 就绪"被否: 需要复制一份就绪判定与超时逻辑, 且会与 attach 并发争抢 tmux 子进程.

替代方案"写进 pane 启动命令"被否: 启动命令跑在 Claude 之前, 那时没有可接收输入的对象.

### 决策三: 以字面文本提交, 而非按键序列

送的是字面文本 `reconnect` 再加一次提交.  `Session::send_keys` 已经用 `tmux send-keys -l` 建立了这个模式, 但它的目标写死为 `{name}:^.0`, 无法指定具体 pane; `send_keys_to_pane_target` 能指定 pane 但只送 key 名.  因此新增一个按 pane id 提交字面文本的函数, 复用 `-l` 的既有写法, 不新造机制.

选 `reconnect` 这个词是因为 xats 的 reconnect 工具本身就以它作为触发语, 用户手工恢复时输入的也是它, 实测有效.  AoE 不需要拼接 identity key 或任何参数: key 已经在 pane 环境里, 由 Claude 自己读取.

### 决策四: 把"key 是否新建"沿调用链传递到 auto-confirm

当前 `ensure_xats_identity_key` 与 `ensure_slot_identity_keys` 都不报告它们是否真的铸造了新 key, 而 `auto_confirm_panes` 只接收 pane id 列表, 两端信息不通.

因此让两个铸造函数报告本次是否新建, 并把 `auto_confirm_panes` 的入参从裸 pane id 扩展为携带该标记的条目.  五个调用点各自提供答案: 单 pane 启动与 respawn 路径取 `ensure_xats_identity_key` 的结果; 多 pane restart 与 recovery 路径按 slot 取 `ensure_slot_identity_keys` 的结果; `auto_confirm_launched_pane` 服务的是新增 pane, 恒为新建.

替代方案"在 auto-confirm 内部重新查询 key 状态"被否: 那时 key 已经写好, 无法再区分它是本次写的还是原有的.

## Risks / Trade-offs

**用户在提交前抢先输入** → 提交发生在 attach 之前, 用户此刻通常还没看到该 pane.  即便发生, 后果是两条输入先后到达 Claude, 不会损坏状态.

**输入提示符识别误判导致提交过早** → 沿用既有的 `shows_claude_input_prompt` 判定, 不新增识别逻辑.  该判定已经在生产里承担 auto-confirm 的收尾职责, 误判风险与现状同级, 不因本变更升高.

**对话里多出一条 `reconnect` 用户输入** → 这是可见噪音, 但语义明确, 且换来的是身份不再静默失联.  实测数据显示静默失联的代价 (一个 pane 数小时对 team 不可见) 远高于一行噪音.

**pane 从未就绪, reconnect 不会被提交** → 与超时后不再 auto-confirm 的既有行为一致: 保持 pane 可交互, 不使 session 失败, 用户仍可手工输入 `reconnect`.  本变更不降低任何现有能力.

**identity key 持久化失败会让 pane 永远走"新建"分支** → 这是既有缺陷而非本变更引入: `ensure_slot_identity_keys` 在 upsert 失败时只记 warn 并保持 key 为空, 下次启动会再铸一个.  本变更不扩大范围去修它, 但会因此少提交一次 reconnect, 退化为当前行为 (用户手工恢复), 不产生新的错误状态.

**测试手段受限** → 本机有大量真实 AoE session, 不能跑全量测试套件, 不能执行 ad-hoc tmux 命令.  纯逻辑部分 (触发判据, 信号选择) 用单元测试覆盖; 任何触碰 tmux 的测试必须走 `TuiTestHarness` 的私有 socket 隔离, 并同时清除 `TMUX` 与 `TMUX_PANE`, 否则会连到真实 server.

## Migration Plan

无数据迁移.  不新增配置项, 不改变已持久化结构, 无需 schema 版本变更.

回滚方式是恢复代码: 去掉提交动作后, Claude pane 回到"重启后需手工输入 reconnect"的现状, 不留下任何需要清理的持久化痕迹.

## Open Questions

无.  方案的两个关键取舍 (触发判据用 key 复用性, 首次启动保留用户命名权) 已与用户确认.
