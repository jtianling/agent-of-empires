## Context

kimi code 的 TUI 通过 `kimi --session <id>` 附着到一个已存在的 session, 没有类似 Claude `--session-id` 的预分配 flag。  准确 session 只能由 REST 先铸造, 这与 OpenCode 走 `POST /session` 是同一个形态。

但两者的 server 所有权相反。  OpenCode 的 server 由 AoE 为每个 pane 起在随机 loopback 端口, 用完即杀, 因此 `base_url` 本身就是 pane 的区分维度。  kimi 的 server 是用户常驻的共享单例, 所有 pane 的 `base_url` 完全相同, 只有 `session_id` 能区分 runtime。  该 server 同时服务非 AoE 启动的 kimi, AoE 终止它等同于破坏用户的其他工作。

xats 侧已确认 kimi 的 runtime key 是 `canonical(base_url) \0 session_id`, 所有反查都是 pair-based, 没有任何绑定逻辑假设 `base_url` 能单独区分 runtime。  xats 也已确认 kimi 不需要 OpenCode 那套 reserve 与 generation fence, 只需要一个刷新投递坐标的 commit。

## Goals / Non-Goals

**Goals:**

- 让 host kimi primary/secondary pane 都使用准确、独立的 native session。
- `Shift+C` 创建全新 conversation, `Shift+R` 恢复原 conversation, 两者都保留该 slot 的 xats identity key。
- 在 agent 侧零配置的前提下保证投递坐标正确, 不把正确性押在 agent 行为上。
- 对共享 server 只读不写生命周期: 只发现、只连接, 不启动也不终止。
- 对 server 发现、token、session 铸造、MCP 配置和 xats commit 的每层错误显式失败。
- 把 agent 能力判据收回 `AgentDef`, 使新增 agent 不再需要在调用点追加字符串比较。

**Non-Goals:**

- 不在 AoE 内实现 MCP client, 不使用 `npx` CLI transport。
- 不为 kimi 引入 runtime generation 或 fence。  xats 侧不存在对应机制, 引入只会制造假保证。
- 不代替用户写入 kimi MCP 配置, 不安装 plugin, 不修改 `config.toml`。
- 不 archive 被抛弃的 session。
- 不为 sandboxed kimi 增加支持。
- 不内建 kimi dev repo 分支的知识。  dev 环境通过既有的 command override 配置。

## Decisions

### 1. 用 runtime 形态字段替换调用点字符串比较

`src/session/instance.rs` 现有约十处 `pane.tool == "opencode"` 形态的判据, 实际只表达四个概念: 使用 AoE 自有 runtime、identity key 不进 pane 环境、支持 Cross Agent Team、resume 前校验 session id。  kimi 会命中全部四个, 直接追加 `|| == "kimi"` 会让下一个 agent 再重复一次。

改为在 `AgentDef` 上表达能力, 其中 runtime 形态区分两种所有权:

```
ExactSessionRuntime::OwnedServer   AoE 起临时 loopback server, 用完即杀 (opencode)
ExactSessionRuntime::SharedServer  发现共享单例 server, 只连不杀 (kimi)
```

谁可以终止 server、`base_url` 是不是 pane 的区分维度、cleanup 该做什么, 都由这一个字段推出, 而不是散在条件分支里。  该重构行为不变, 由 OpenCode 既有的聚焦测试充当回归网。

### 2. 共享 server 只发现不启动

从 `${KIMI_CODE_HOME:-~/.kimi-code}/server/instances/<server_id>.json` 发现, 结构为 `{server_id, pid, host, port, started_at, heartbeat_at, host_version}`。  该注册表由 kap-server 的 public API 导出并写入用户文档, 是可依赖的公开约定。

选择规则照抄 kimi 自身所有单实例消费方: 按 `started_at` 升序取第一个存活实例, 使 AoE 与 kimi CLI 选到同一个。  存活判据只看 pid, 不看 `heartbeat_at`。  后者是 informational 字段, 只反映"文件在被刷新", 不能反过来当死亡判据。  pid 探测遵循保守规则: ESRCH 判死, EPERM 判活, 其他错误一律判活。

解析失败的条目不删除, 那可能是正在写入的 peer。  单个条目施加 4 KiB 上限, 只认 `.json` 后缀, 从而天然避开原子写的临时文件。

无存活实例时 fail closed 并提示用户启动 server。  AoE 不启动 server 有两个理由: 该 server 是共享单例, 起了就不能杀, 而 AoE 只能在某个 pane 的上下文里启动, 那个上下文恰好带着该 pane 自己的 `KIMI_XATS_SESSION_ID`, 泄漏进 server 会让所有 server 侧 agent 注册到错误 session。

备选方案是允许 AoE 启动 server 并清洗环境, 但共享单例的所有权无法表达"谁该负责终止", 因此不采用。

### 3. 启动前铸造准确 session

顺序固定为四步, 每步失败都显式中止:

```
POST /api/v1/sessions             铸造 S, metadata.cwd 必须等于该 pane 的 working directory
POST /api/v1/sessions/S/profile   设置 model 与 permission mode
GET  /api/v1/sessions/S/messages  触发 main agent 物化
```

`metadata.cwd` 与 pane cwd 必须一致, 因为 `kimi --session <id>` 会校验 session 的 workDir 等于当前 cwd, 不等则硬失败退出。  注意对齐的是 pane 的 working directory 而非 AoE 进程的 cwd。

profile 一步不可省。  server 铸造的 session 不带 model, 未设置时服务端驱动的轮次会立即失败, 而 xats poke 正是服务端驱动的轮次, 也就是 AoE 场景的主路径。

`GET /messages` 用于触发 main agent 物化, 替代 `xats-kimi` 启动函数里"发一条假消息再轮询最多 30 秒"的做法。  该路由内部同步完成物化并返回空消息列表, 因此新 conversation 开头不会留下垃圾消息, 也不需要轮询。  它同时充当"session 已对 GET 可见"的证明。

Resume 时跳过铸造, 直接使用 durable slot 中的 session id。

### 4. 每次启动都提交投递坐标

调用 `POST /api/runtime/kimi/commit`, 严格发送 `{protocol_version: 1, identity_key, base_url, session_id}`, daemon 发现与 bearer 复用 OpenCode 已有实现。

该端点按 identity_key 反查行并刷新投递坐标, 不创建身份, 不接触任何 MCP 连接。  AoE 不持有 name 与 team, 因此必须走这条按 key 反查的路径, 不能直接构造 register 调用。

在**每次** kimi pane 启动时调用, 而不只在 `Shift+C`。  Resume 时坐标未变, 返回幂等结果且不触发对 kimi server 的探活, 代价只有一次本地 REST 往返; 但冲突检查无条件执行, 因此"该 session 已被其他 agent 行认领"会在启动 TUI 之前就暴露, 而不是等到身份没有恢复再去归因。

响应分层严格区分: `session_not_found` 允许有界重试, 首次启动返回的 `need_register` 是正常状态并跳过预注册, 其余 outcome 一律立即失败。  响应中的 `probed` 字段表示本次是否真的验证过 session 存活, 成功不等于会话健康, 不得据此推断。

**commit 必须是最后一个写坐标的人。**  agent 侧的 `reconnect({identity_key})` 保留行上已有的 delivery, 因此只要 commit 先于 TUI 启动完成, 该行为恰好正确。  AoE 在 commit 之后不得触发任何其他注册动作。  该约束在 kimi 支持 handshake 自动绑定之后依然成立, 因此写为 requirement 而非实现注释。

### 5. MCP 配置只校验不代写

xats 身份绑定依赖 kimi 的 MCP 连接在请求上携带 `X-Kimi-Session-Id`。  header 必须由用户配置在 `mcp.json` 中, kimi 不会自动注入。

多 pane 不需要多份配置。  `${KIMI_XATS_SESSION_ID}` 的展开走 per-session overlay 且优先于进程环境, 值恒为该连接所属 session 的真实 id, 因此一份用户级配置即可覆盖所有 pane。

AoE 校验配置存在且满足硬要求, 不满足则 fail closed 并打印需要粘贴的配置。  硬要求包括 `scope` 为 `session`。  默认的 workspace 作用域会让同 workspace 的所有 session 共用一条连接, 导致多个 agent 共享一个 MCP 身份。

不代写的理由是该文件属于用户的 kimi 配置, 而配置错误的后果 (身份串绑) 静默且难以归因, 由用户显式确认一次比 AoE 静默改写更安全。

### 6. 环境注入先删后设, 且不包含 identity key

向 kimi 子进程注入 `KIMI_XATS_BASE_URL`、`KIMI_XATS_SESSION_ID` 与 `KIMI_REMOTE=auto`, 注入前先移除同名变量, 确保 pane 之间不串值。

**不注入 `XATS_IDENTITY_KEY`。**  kimi 在远程引擎下由长命共享 server 派生工具进程, 该进程继承的是 server 的环境。  kimi 为此建立了 per-session overlay, 但 overlay 只回放 `KIMI_XATS_SESSION_ID`; `XATS_IDENTITY_KEY` 是 AoE 与 xats 之间的约定, kimi 不知道它存在, 因此永远不会被回放。  结果是该 server 上每个 kimi agent 都读到同一把 key, 也就是启动 server 那个 shell 携带的值。  由于 AoE 会向 Claude 与 Codex pane 注入 identity key, 从任一 AoE 托管 pane 启动 kimi server 都会让这把 key 泄漏给该 server 上的所有 kimi agent, 使其可能恢复成他人身份。

因此 identity key 全程由 AoE 持有: 从 durable slot 读出, 只用于 xats commit 请求体, 不进入 pane 环境, 不进入 argv, 不进入任何 agent 可读的位置。  这与 OpenCode runtime 从 durable slot 回读 key 的做法一致, 区别在于此处不是洁癖而是必需。

`KIMI_XATS_BASE_URL` 同样不在 overlay 中, 但共享单例下所有 pane 本就应当读到同一个值, 因此无害。

`KIMI_REMOTE=auto` 不可省。  TUI 默认运行进程内引擎, 而 xats 的投递打到共享 server, 两者会形成两个引擎操作同一 session。  `auto` 让 TUI 通过 klient IPC socket 附着到共享 server 的引擎, 找不到时回落进程内引擎。

`KIMI_XATS_BASE_URL` 存在时 kimi 会按其端口精确定位实例而不走注册表选择, 因此 AoE 选定的实例会传导给 TUI, 两侧不会各选各的。

`KIMI_XATS_SESSION_ID` 的泄漏后果比 OpenCode 的 identity key 更重: 若它进入 server 启动环境, 所有 server 侧 agent 都会注册到错误 session。

### 7. 以 kimi 版本作为准入门槛, 不支持降级运行

kimi 发布版的 TUI 只能运行进程内引擎, 而 xats 的投递打到共享 server。  已沿投递路由查证: server 收到 prompt 后 resume 该会话到**自己的**引擎 (注释明写"由其他进程创建的持久化会话会从磁盘加载"), 在自己的引擎里取得 main agent, 然后入队到一个**进程内**队列并由本进程驱动。  TUI 侧引擎持有的是另一个队列实例, 代码中不存在跨进程移交。  因此在发布版配置下, 投递跑在一个用户看不到的 server 侧实例里: 发送方收到成功而用户什么也看不到; 与此同时两个引擎同时打开同一 session 并写同一份记录, 而引擎层没有跨进程 session 锁。

xats 的投递前置 gate 用 TUI 写入的记录时间戳判断"TUI 正在跑一轮", 这一点曾被当作反证 (若真有双引擎, 该 gate 早该暴露问题)。  实际相反: 该 gate 的注释自陈这是"REST 探针观测不到的 TUI 侧轮次"的启发式, 而若只有 server 一个引擎, server 用自己的状态即可判定, 无需去读文件时间戳。  需要靠文件系统才能观测, 恰恰说明写文件的是 server 观测不到的另一个进程。  该 gate 的存在是承认冲突而非否认冲突。

"上线已久未见问题"这一观察也不构成反证: 本机运行的是具备远程引擎能力的构建, 该配置下只有 server 一个引擎, TUI 只是它的 RPC 客户端, 因而轮次当然可见。  观察结果由配置解释, 不需要推断为假。

让 TUI 附着到共享 server 引擎的机制, 以及在缺少该通道时兜底显示 server 侧轮次的观察器, 与 per-session 环境 overlay、MCP header 模板同属一批能力, 它们只存在于用户本地的分支构建, 上游发布版中不存在对应文件。  因此不存在可以等待的官方版本, 也不能用引擎版本号作为判据: 分支构建报告的是它所基于的上游版本号。

AoE 因此在启动 pane 之前判定所选 server 是否具备承载该 pane 会话的能力, 不具备则明确拒绝并说明缺什么, 而不是降级到"看起来能用但投递进入影子会话"的状态。  静默半通比明确不支持更难排查, 而本 change 的整体取向就是让失败显式。

判据必须是来自运行中 server 的正向信号, 而非版本号推断。  这批能力不会合并回上游, 因此不存在"等某个版本"的选项, 版本号也永远区分不出构建来源。

判据分两段, 缺一不可, 因为 server 与 CLI 是两个独立产物:

- **server 侧**: 所选实例的 IPC socket 存在。  该 socket 只由具备这批能力的 server 挂载, 上游 server 从不挂载它。  socket 名中携带的端口与 instance 注册表 advertise 的是同一个, 因此两者必须对上; 崩溃会遗留 socket 文件, 所以存在性必须与该实例的 pid 存活判断绑定使用。
- **CLI 侧**: socket 无法证明用户启动的 kimi 二进制具备对应能力。  读取远程引擎模式的代码在 CLI 一侧, 因此"发布版 CLI 搭配具备能力的 server"这一组合会通过 server 侧判据却仍然运行进程内引擎。  AoE 知道自己启动的是哪个二进制, 因此把"kimi pane 必须显式配置指向具备能力的构建"做成显式配置要求, 而不是默认取 PATH 上的 `kimi`。

两段都满足才放行, 任一不满足明确拒绝。  socket 缺失也可能是挂载被显式关闭或失败, 该方向是 fail closed, 符合预期。

**验证这一门槛时必须写死被测配置。**  在具备能力的构建上做端到端测试必然通过, 但该结果只证明这条路径本身可用, 不能用来否定发布版配置下的双引擎问题 —— 那需要显式使用发布版二进制并确认远程引擎模式未生效才能观察到。  拿一个配置的绿去否定另一个配置的风险, 会让该风险从文档中被错误地抹掉。

达到门槛之后, 身份绑定走 handshake header, 而 header 走 per-session overlay, 是全链路唯一 session-scoped 的传递方式, 也是唯一不受共享 server 环境影响的通道。  投递本身只依赖行上坐标正确, 与是否绑定无关。

`host_version` 不能作为能力判据。  dev build 报告的是上一个已发布版本号, 因此版本比较只能用于确认支持, 不能用于确认不支持。

### 8. 绑定可观测性

commit 端点不刷新 `last_seen_at`, 而 xats 只在调用方解析出已注册 agent 时才刷新该字段。  因此 `last_seen_at` 越过 commit 时刻等价于"一个已绑定的 MCP 会话真的调用过工具", 可作为 positive confirmation。  该判据确认的是"已绑定且活动过", 不确认"已绑定"。  `online` 字段不可用于此目的, 其窗口为四天, 语义与连接绑定无关。

## Risks / Trade-offs

- [被抛弃的 kimi session 在共享 server 上永远看起来健康] → xats 的探活只检查 id 匹配与未归档, 而本 change 不 archive 旧 session, 因此过期坐标的探活仍会通过, 投递会静默落入无人查看的会话。  唯一防线是 `Shift+C` 严格串行: 先终止旧 pane 并确认进程退出, 再铸造新 session 并 commit, 最后启动新 pane, 使唯一可能写入过期坐标的主体在 commit 时已不存在。  该风险不被消除, 只被时序约束压住; 若串行被破坏, 失败模式是静默误投而非报错。
- [kimi session 无法删除] → 每次 `Shift+C` 铸造的 session 永久累积。  本 change 不做归档或复用, 该成本由用户承担。
- [xats commit 端点尚未上线] → kimi Cross Agent Team 启动明确失败并保留脱敏诊断, 非 Cross Agent Team 的 kimi pane 不受影响。
- [共享 server 崩溃影响所有 kimi pane] → 与 OpenCode 的 per-pane server 不同, kimi 无崩溃隔离。  AoE 只在启动时发现并报错, 不做运行期监控。
- [用户在 pane 内改名会在 xats 侧留下 ghost row] → AoE 不持有名字也无法感知改名, 该问题由 xats 侧修复, AoE 侧仅通过 commit 的冲突检查在启动时暴露。
- [kimi 远程引擎下 agent 读到的环境属于共享 server] → AoE 不通过 pane 环境传递 identity key, 因此不依赖 agent 读取任何凭证。  身份绑定只走 session-scoped 的 handshake header, 该通道不受共享 server 环境影响。
- [功能可用性依赖 kimi 发布节奏] → 本 change 可以完整实现与单元验证, 但端到端可用需要达到版本门槛的 kimi 发布版。  门槛之下 AoE 明确拒绝启动 kimi pane, 因此不存在"部分可用"的中间状态。
- [identity key 可能并非来自 AoE 注入] → `XATS_IDENTITY_KEY` 也可由启动 kimi server 的 shell 继承而来, 而那个 shell 极可能就是某个 AoE 托管的 pane, 因此即使 AoE 不注入, 该变量仍可能出现在共享 server 的环境中并被所有 kimi agent 读到。  AoE 无法控制他人如何启动该 server, 对应防线在 xats 侧: 其工具描述已禁止 `kimi-code` 读取该变量作为身份凭证。  AoE 侧的对应义务是让 key 全程不进入 agent 可读范围。
- [能力判据重构触及 OpenCode 既有路径] → 该重构行为不变, 以 OpenCode 既有聚焦测试作为回归网, 并在同一 change 内验证。

## Migration Plan

无数据迁移。  kimi slot 复用既有 `agent_slot` schema 的 `native_session_id` 与 `xats_identity_key` 列, 不新增列, 不使用 `xats_runtime_generation`。

## Open Questions

**AoE 用什么 handle 让 xats 找到该 slot 对应的 agent 行?**

决策 6 确定 identity key 不进入 kimi pane, 因此 kimi agent 注册时不会携带它, 该 key 永远不会出现在 xats 的行上。  按 identity key 反查的 commit 契约在 kimi 上因此无法命中, 需要在两种形态中择一, 由 xats 侧决定:

- 采纳形态: commit 在 identity key 未命中时改按 `(base_url, session_id)` 反查, 唯一命中则把 key 绑定到该行并继续刷新坐标。  key 仍是能扛住坐标漂移的稳定 handle。
- 迁移形态: commit 不使用 identity key, 改为 `(base_url, old_session_id) → new_session_id` 的坐标迁移。  AoE 在覆盖 durable slot 之前一定持有旧 session id。  该形态下 kimi slot 不再需要铸造 identity key。

两种形态对本 change 的其余部分没有影响, 仅影响 xats commit 客户端的请求体与 durable slot 是否保留 identity key。

无。  原先挂起的 shadow turn 问题已澄清, 见下。

其余问题已关闭。  handshake 语义、kimi REST 行为、共享 server 发现约定与投递门现状已分别与 `xats-main` 和 `claude(kimi)` 对齐, AoE 只实现消费端。
