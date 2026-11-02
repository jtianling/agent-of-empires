# Managed Codex 每 pane 自有服务候选

2026-10-28, steward 基于 jt 已授权持续推进 AoE 开发的范围,
选择在 AoE 内实现每个启用 xats 的 managed Codex pane 自有 app-server.
不修改 Codex 维护仓库, 不引入 WS 配置中介, 不全局安装或操作业务 session.
原共享服务失败证据保留在 `managed-codex-remote-resume.md`.

本候选复用现有 managed owner、pane ID、xats prepare/bind fence,
以及平台进程启动身份检查.  私有服务的 MCP 配置与权限固定在该进程的
配置覆盖层, 因此无需 TUI 或 xats poke 在每次 resume 时重传.
模型继续按工作目录的有效配置解析, 只固定当前 provider、准确 MCP URL、
本代 managed header 的环境映射及保存的权限模式.

每个服务使用 `ws://127.0.0.1:0` 请求内核分配端口,
由本进程成功监听后的 stderr banner 取得实际端点.
不探测并接管任意已有端口, 不依赖共享 app-server.
token 仅进入该服务的环境和权限为 600 的私有 profile,
命令参数只包含环境变量名称等非敏感值.  TUI 继续使用其私有 profile.

真实 0.165.0 窄验先后发现并修复了两个具体问题:

- app-server 子命令不接受 `-p`, 服务端改用自身 `-c` 覆盖,
  私有 profile 只传给支持它的 TUI.
- `-c` dotted key 按点拆分, 不去掉键名中的引号.
  header 名必须直接作为末级键, 不能写成带字面引号的 HTTP header 名.
  原失败的标准 env header 映射缺失, 进程环境本身存在且匹配, 有独立布尔观测证据.

内部 `ClientConfig` 新增默认 false 的 owned 标记, 旧记录仍可读取.
新启动在写 profile 和创建进程前先记录可重试的配置归属,
再记录实际端点及准确 thread ID.  已创建 thread 的权限校验失败会保留准确 ID,
阻止后续 profile 发布和 bind, 不把失败重试变成重复创建.

自有服务记录位于对应 AoE profile 的
`managed-codex/<完整pane-id>.server.json`, 包含 launch ID、实际端点、
PID、进程启动身份及已观察的进程组成员身份, 不包含 token.
同 pane 的启动、检查及回收使用文件锁.
成功启动后由实际父进程的后台 wait 线程回收 child,
避免长期运行的 AoE TUI 留下 zombie.

停止服务前检查进程启动身份和精确进程组中的可信成员,
不能仅凭 PID、端口、进程名或 leader 消失判断归属或清理完成.
先 SIGTERM, 仍有已验证成员才升级 SIGKILL,
无匹配身份则明确拒绝, 不误杀其他进程组.
旧代 wrapper 仅允许回收其自身 launch, 不能终止新代服务.

清理接入主/子 pane 的 stop、restart、remove、接任及 TUI 退出路径.
创建、监听、绑定、respawn 或最终保存失败也清理本次服务,
保留原操作、保存与清理的具体原因.  仅清理失败 pane,
不因一个附加 pane 失败而回收仍正常工作的邻 pane.

旧 shared 记录迁移仍须经过受检 managed 生命周期.
使用准确 previous fence 换代, 对旧端点的准确 thread 检查状态;
活跃或无法确认状态时拒绝迁移.  连接明确被拒绝时记录具体离线原因,
可从持久 thread 在自有服务中恢复.  不终止共享 server,
不普通 register 重绑, 不释放后另建身份绕过代次保护.

在线迁移发现了真实 writer 保留问题, 旧 FAIL 均保留.
精确 0.164.0 和 0.165.0 的配置源码默认保留时间均为 1800 秒,
旧参考 checkout 的 60 秒不能代入这些 release.
最终候选的 preflight 仅只读检查, 不订阅或改模式/fence.
受检停止旧 TUI 后, 只对准确 idle thread 的 resume 请求传入
`thread_unload_delay_secs=0`, 然后退订, 通过 `thread/loaded/list`
确认该 thread 实际退出加载集合后再恢复同一个 native ID.
这利用 release 的每 thread 配置, 不修改共享 server 的全局默认,
也不等待固定时长或停止共享进程.  超时保留准确 ID 和最后观测,
不回滚旧 token 或把失败改判通过.  最终独立复测已通过在线 0.165→0.165,
以及旧 shared 0.164→owned 0.165.  目标约 0.5 秒退出 loaded 集合,
旁路 thread 仍 loaded, 共享 PID 存活.  保留同 native thread 和 agent ID,
真实 daemon poke 后 Sol 只读到迁移后的新任务, 旧游标任务不重放.

已取得的独立窄验与中间结果, 尚不代表完整生命周期通过:

| 范围 | 当前证据 |
| --- | --- |
| 0.165.0 单主 pane high | 真实独立 xats daemon 的 get_inbox 读到 TASK_A, 结果回模型, 无审批框 |
| 单主 pane 启动耗时 | 一次测量 0.971 秒 ready, create 到实际工具成功 1.303 秒, 不是性能上界 |
| 双 pane | 不同 PID、端点与 thread, 分别读不同任务并有后续工具调用 |
| 真实 idle poke | daemon 的原生 resume 与 turn/start 触发实际任务消费, 不是手工 managed send |
| Sol 工作区模型 | 实际请求确认 gpt-6.1-sol、high 与正确 cwd; 原生 code-mode 调用真实 MCP, 任务进入 custom_tool_call_output, 无审批 |
| Sol 原生完整矩阵 | 38/38 通过, 包含 active steer、换代 cold 恢复、父 stop/start、旧代拒绝及主 pane 接任 |
| 清理、迁移、越权 | guards 15/15, offline 6/6, 在线同版与 0.164→0.165 跨版迁移通过; 最终汇总报告收尾中 |
| 资源成本 | 已汇总 Linux arm64 的 1→2 agent 采样, 数字见下表, 不宣称生产容量 |

Sol 的公开模型元数据为 `tool_mode=code_mode_only`.
离线 function_call 夹具不能仅凭 `tools=[]` 判断 AoE 丢失 MCP,
也不能用 local-mock 的工具通过冒充 Sol 的实际工具调用通过.
本轮已用原生 `custom_tool_call` 的 exec 路径完成单主 pane 实际调用,
没有修改 feature 或模型目录, 没有安装额外 host.
原先 BLOCKED 属于测试夹具使用 legacy function_call 的环境不匹配,
不是产品不支持 Sol.  Sol 原生矩阵已 38/38 通过,
local-mock 正交矩阵也 38/38 通过, 两组证据独立保留.

资源采样环境为 Linux arm64 Docker、Codex 0.165.0、debug AoE,
使用 Sol/high 的本地模型响应夹具和真实独立 xats daemon,
只采 5 秒空闲窗口.  精确根进程的子树去重计数, server 与 TUI/wrapper 分开:

| 两 agent 实测范围 | RSS MiB | PSS MiB | 空闲单核 CPU |
| --- | ---: | ---: | ---: |
| 自有 server 树合计 | 971.38 | 800.21 | 0.4% |
| TUI 与 AoE wrapper 树合计 | 649.80 | 515.34 | 0.2% |
| 独立 daemon | 103.50 | 79.92 | 0.2% |
| 合计 | 1724.68 | 1395.47 | 0.8% |

1→2 agent 的总增量为 RSS 800.99 MiB、PSS 566.92 MiB,
其中 server 树增量为 471.98/331.36 MiB.
该数字是本候选新增一个 agent 的测量, 不是与旧 shared 策略的净增对照.
RSS 会重复计算共享页, PSS 共享比例也可能随数量非线性变化.

按双 pane 实测锚点线性外推, 5 个双 pane 任务约用 PSS 5930.8 MiB,
10 个约 11600 MiB.  实际只测了 1 和 2 agent, 外推不包含 tmux server、
Docker VM、模型夹具和控制器开销, 不代表忙时负载、真实云模型、复杂 MCP、
release 构建或 macOS 成本, 也没有据此给出容量通过结论.

证据目录包括 `.e2e-test/runs/owned-codex-single-20261028-aoe-tester/`,
早期失败与修正后的 v3 分开保留; 完整矩阵由 tester 单独固定源码 hash.
宿主仅运行编译、clippy 和不连接真实 tmux 的 CLI 回归,
真实 TUI、daemon、进程故障与资源测量均在一次性隔离容器进行.

当前旧 TUI 的兼容边界已按源码核对.  managed_xats.client_config 是
opaque JSON, 没有新增 Instance 顶层字段, 普通 load/save 可保留内部 owned 值.
但旧 managed_codex 的 ClientConfig 使用 deny_unknown_fields,
且旧 stop/remove/restart 不具备自有服务回收逻辑, 因此生命周期不支持混用.
启用候选前需由 jt 正常退出对应旧 TUI, 使用新版本重开同 profile;
不是停止所有 agent session, 也不要求重启共享 app-server.

UTC 2026-10-28 06:04:49 的只读登记查询中,
auto-coworkspace-bedc 的 PID 2706 为 stale,
PID 29459 为 legacy unverified, 没有可验证的登记启动身份.
另只读 OS 查询确认 29459 为 aoe, cwd 是
`/Users/jtianling/workspace/coworkspace`, 无显式 profile 参数,
启动时间为本地 2026-10-26 09:07:34.
这仅供 jt 识别对应前端, 不作为自动 kill 或重启依据.
正常 TUI 退出源码只清快捷键、通知 UI 选项并恢复终端,
没有停止 agent session 的操作; 本轮未在 macOS 业务现场执行该动作,
不把源码核对说成实际零影响验证.

最终干净上下文 review 又发现了两个需要独立回归的边界,
旧候选的冻结已暂停, 不能把旧 manifest/hash 套到修正后的源码:

- 附加 pane 校验失败、尚未换代时, 旧异常路径仍可能清理原 backend.
  当前主/附加生命周期在操作前记录 claim launch marker,
  只在操作确实切到新 launch 时清理 current launch, 校验拒绝不动原服务.
- TERM 之后出现新成员 N, 若已观察到可信 W 与 N 重叠,
  后续 W 退出时不能仍只认旧见证.  当前每次可信重叠都会更新内存和持久成员,
  保存失败仍依据已确认的内存见证清理, 并组合保存、信号和清理错误.

同轮还检查了 manager 环境的隔离边界.  CLI 支持通过 AOE_MANAGER_CONTEXT
隐式选择 context, 工作进程不应继承该变量而隐式获得 manager CLI 身份.
当前 managed helper 入口、TUI child 和 owned server 明确剥离此变量,
server 也剥离父 generic managed MCP token, 仅注入它自己的 token.
检查仅使用源码或无敏感 sentinel 的存在布尔, 不读取真实 context 内容或打印全环境.
最终独立 re-review、移动 pane 的真实 private backend 拒绝路径、
TERM 生成新后代及环境隔离窄回归均已完成, 各阶段证据保留准确 hash.

普通 human CLI 的合法 stop 回归还暴露了 profile 路由错误:
`resolve_profile(None)` 会根据 cwd 生成 auto-profile,
不能替代已经由 CLI 选定并写入环境的 profile.
当前 server 记录路径、物理位置校验和 managed runtime 配置路径统一复用
`Instance::managed_profile()`, 优先使用记录的 source_profile,
否则读取当前已选 profile.  独立 tester 已确认显式 `-p` 覆盖不同环境值和 cwd,
移动主/子 pane 的 stop 精确拒绝且两原 runtime 继续实际读信,
合法 stop 后原组不活且记录消失, start 恢复后两个 Sol agent 各自实际读到新任务.
最终干净单测、无 shim smoke 与完整残留审计报告已收齐,
停止成功的结论包含真实进程状态与端口证据.

启动记录保存失败的单一负例又发现了实际 Child 泄漏:
初始记录被替换为空目录后, CLI 虽返回具体 I/O 错误,
但清理函数读取记录时提前返回, 只交给 reaper 等待不能终止仍活的进程.
当前启动流程在内存保留 root 的启动身份和每次已确认的成员,
在写记录前先更新该状态.  失败清理与 reaper handoff 失败均使用内存见证,
不依赖损坏记录可读, 原进程组身份验证与具体持久错误保留.
该单一负例的内存态修正版取得 7 项独立 PASS:
故障恢复前 root 与同组 child 的进程条目均消失, 真实监听端口关闭,
随后同一 ID 重试能通过原生 Sol 工具读取任务.
独立 review 又补充两处退出窗口: 已有内存见证时无需重新查询 root 身份,
避免 root 在观测间退出而提前跳过组清理; 启动失败返回前同步 try_wait,
组合具体回收错误, 尚未退出则保留 reaper 接管且不无界等待.
两处修正后的最终 review 已 PASS, 最终源同负例 7/7、干净单测 16/16、
无 shim Sol/env smoke 6/6 均通过.  最终统一报告 130 PASS、0 FAIL、
1 BLOCKED、3 SKIPPED, Linux clippy 缺镜像组件以及三项未做的精细失败注入
明确保留边界.  旧候选不可套用本轮通过结论, 本轮没有激活业务.
验收必须在恢复人为文件故障或外部清理之前,
确认原 Child、已知同组 child 与监听端口确实退出,
不能把恢复 fixture 后的正常 remove 当作产品失败回收通过.

最终清理前被动核对 14 个 fixture 和 23 条晚生后代事件,
确认零活跃测试进程、零观察到的监听端口、零 zombie 后,
再删除准确的一次性 review 容器.  完整产物和启用步骤见
`managed-codex-owned-handoff-20261028.md`.
