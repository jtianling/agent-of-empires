# Agent 管理 CLI

`aoe managed` 让本地管理 agent 创建和管理工作 session.  所有变更只允许操作当前管理身份创建的 session, 人工 session 和其他管理者的 session 只提供基本信息与已保存状态.  终端内容读取也受所有权检查保护.

这是接口防误操作机制.  拥有同一系统用户完整 shell 权限的 agent 仍可绕过此入口调用人工 CLI 或 tmux.  首次启用前需要更新并重启原有 TUI, 旧版本的 TUI 不具备并发保存保护.

pane 扩展新增了持久身份和接任元数据, 不支持与旧版 TUI 混用: 旧进程保存 session 时可能丢弃它不认识的字段.  使用 pane 命令前应重新打开访问同一 profile 的所有 AoE TUI, 不需要停止其托管的 agent session.

## 建立管理上下文

每个管理 agent 建立独立上下文, 多次对话和重启使用同一个文件.  上下文包含随机凭据, 不要提交到仓库、输出给其他 agent 或共享给子 agent.

```sh
aoe managed --context /absolute/path/manager.json init
export AOE_MANAGER_CONTEXT=/absolute/path/manager.json
```

上下文父目录必须存在.  `init` 不覆盖已有文件.  返回的 owner 是公开标识, 不能仅凭这个标识获得管理权限.  子 agent 的 xats name/team 与管理 owner 独立.

## 创建与管理

```sh
aoe managed create /absolute/path/project --tool codex --title coder

aoe managed create /absolute/path/project \
  --tool codex --title collaborator \
  --xats-name coder --xats-team project --launch

aoe managed list
aoe managed show <完整-session-id>
aoe managed start <完整-session-id>
aoe managed stop <完整-session-id>
aoe managed restart <完整-session-id>
aoe managed rename <完整-session-id> --title new-title
aoe managed send <完整-session-id> '请检查任务状态'
aoe managed capture <完整-session-id> --lines 100
aoe managed remove <完整-session-id>
```

未指定 `--launch` 时创建 session 记录但不启动 agent 进程; 带 xats 参数时还会预留协作身份.  项目目录必须已存在, 此入口不会创建或删除 worktree、分支.  `remove` 保留代码目录与共享资源.  所有目标操作必须使用返回的完整 16 字符 session ID, 不接受标题和短前缀.

`managed rename` 同步修改准确 tmux session 的名称并保存标题, 保留已有 pane、进程和对话.
TUI 加载外部标题更新后会丢弃旧标题的延迟状态采样, 并重新检查新名称,
避免将仍然存活的 session 误报为红叉.  新名称下的真实故障仍会正常显示.

创建带 xats 身份的 Codex 主 pane 或附加 pane 时可传 `--model <模型 ID>`.  每个 pane 单独保存该值, 新建和恢复线程时均使用它; Codex 返回的实际模型不一致时启动失败.  未指定时沿用 Codex 默认设置.  此参数目前仅支持带 xats 的 Codex pane, 不修改共享配置.

可同时传 `--effort <等级>` 为该 pane 指定推理强度.  启动或恢复时, AoE 显式传入该值并核对线程响应中的强度, 被拒绝或回退时启动失败.  不传时沿用默认值.  模型目录可能滞后, 不作为拒绝显式设置的依据.

传入已有 Git linked worktree 的根目录时, TUI 会在 session 名称后显示其当前分支.  已有的 CLI session 同样适用, 列表刷新时更新显示.  此显示信息仅缓存在 TUI 内存中, 不会让 AoE 接管外部 worktree 的删除或清理.

`managed create` 和 `managed pane create` 接入已有 linked worktree 根目录时,
会从主仓库复用 TUI 的 agent 配置拷贝规则, 补齐未跟踪的 `.agents`、`.codex`、
`.claude` 等目录和已知配置文件.  skills 内的符号链接保持为链接.
已有目标目录整体保留, 单独列出的缺失配置文件仍可补齐; 已有文件和符号链接不覆盖,
也不会穿过目标符号链接目录写入外部文件.  普通仓库及子目录不会触发补齐,
不改变 worktree 的清理归属.  Git 识别或单项拷贝失败记录具体警告, 不阻止创建.

`managed create` 与 `managed pane create` 默认保存 YOLO 模式,
启动和恢复时复用各 runtime 的自动批准机制.  该模式仅作用于创建的 pane,
既有记录仍按保存的模式恢复.  YOLO 不改变 manager owner、xats fence,
也不替代用户对生产发布、转账或隐私操作的授权.

带 xats 身份的 YOLO Codex pane 会在自身配置读取进程、托管服务和独立客户端 profile 中,
显式信任该 pane 的工作目录, 避免新目录停在 Folder access 提示.
该设置不写共享 Codex 配置, 不新增父目录或其他项目的信任条目, 非 YOLO 启动保留原有目录确认流程.

已有 session 可由原管理者执行
`aoe -p <profile> managed --context <原context绝对路径> restart <完整session-id> --yolo`,
显式启用主 pane 和全部自有附加 pane 的 YOLO 模式并持久保存.
该操作先验证受影响 pane 的 owner 和实际占用, 随后停止并恢复整个目标 session,
会中断其中全部 pane 的当前进程, 并保留逻辑 ID.  含人工或其他 owner pane 时拒绝操作.
不带 `--yolo` 的重启保留原保存模式.

原共享 app-server 方案在 Codex 0.164.0 和 0.165.0 的部分 remote resume
路径中会丢失 MCP 配置和权限.  当前候选改为每个启用 xats 的 managed Codex pane
启动自有 loopback app-server, 使用该进程专属的 MCP 与权限配置,
并按工作目录解析任务模型.  TUI 退出及受检 stop/remove/restart 会准确回收自有服务.
旧 shared 记录仅在受检恢复中切换, 不停止共享 app-server; 活跃旧 thread 会拒绝迁移.

自有服务候选已通过真实 Codex 0.165.0 的 Sol 原生双 pane 矩阵,
包括真实 xats poke/steer、任务及后续工具、冷恢复、接任和旧代拒绝.
旧 shared 0.164/0.165 到 owned 0.165 的迁移也验证了原 native thread、agent ID
和收件游标保留, 共享服务及旁路线程继续可用.
创建成功、Full Access 回显或 xats ready 仍不能代替任务验收.

启用 xats 的 managed Codex 在创建、resume 和 restart 时, 会将 Codex 原生会话名设为该 pane 的 `name@team`, 与 `/rename` 的命名效果一致.  主 pane、附加 pane 和接任后的 pane 分别使用自己的协作身份, 不使用 AoE 的 session 标题.  此操作不创建模型回合, 不改变 thread ID 和对话历史; 无 xats 身份时不自动命名.

新旧 AoE 的生命周期操作不能混用.  client_config 作为 opaque JSON 保存,
普通并发保存可以保留内部字段, 但旧版本不认识 owned 标记和私有服务清理逻辑.
启用自有服务前应由用户正常退出并用新版本重开访问该 profile 的旧 TUI,
随后使用同版 managed CLI 操作.  这不要求停止全部 agent session 或重启共享 Codex 服务.

旧 shared 迁移在停止旧 TUI 后, 仅将准确目标 thread 的卸载延迟设为零,
退订并观察它从 loaded 列表消失后才交给自有服务.
不会修改共享 server 的全局保留策略.  若有其他客户端保持订阅或服务无法卸载,
会带准确目标和最后状态超时失败; 保留原记录, 使用同一 context 和完整 ID 重试,
不要重复 create、改存储或回滚旧 fence/token.

完整运行验收为 Linux arm64 隔离结果; macOS 本轮只做编译及只读登记核对,
没有在业务现场启用.  资源采样的一对 Sol/high agent 含 TUI/wrapper 与独立 daemon
约用 PSS 1395.47 MiB, 5 秒空闲窗口约占单核 CPU 0.8%.
实际只测 1→2 agent, 不能据此承诺宿主容量或真实云模型、复杂 MCP 的成本.

自动 xats 身份启动首批支持 `codex`、`claude` 和 `opencode`.  `kimi` 的共享引擎目前不能注入会话独立的 managed 凭据, 因此 `--tool kimi` 与 xats 参数组合会在创建前拒绝.  不带 xats 参数的 managed Kimi session 可正常创建.

自动注册需要配套支持 `/api/managed/{prepare,bind,status,release}` v1 的 xats daemon.  旧 daemon 不会被自动更新, 接口不可用时会明确失败.  配套实现已完成独立容器中的 Codex、Claude、OpenCode 实际启动和重启验收, 尚未替换生产 daemon.

使用 `aoe -p <profile> managed ...` 指定 profile.  管理上下文可以跨 profile 使用, 每个 session 的 owner 仍独立校验.

## 单独管理 pane

上一轮新增 pane 的 Claude 冷恢复连续性曾暂缓验收.
当前已补齐 Claude 主/附加 pane 及接任后的恢复, 包括没有 xats 和没有 AoE TUI 的场景.
已完成真实对话的原生 ID 需要在停止或换代前准确保存,
不把预分配 UUID、进程就绪或 xats ready 单独当作对话可恢复的证明.
独立验收使用真实 Claude Code 2.1.299 和隔离的本地模型协议夹具,
验证了原生对话 ID、实际工具结果、接任后对话连续性及 Fresh 失败后的同 ID 重试.
Claude 的 HTTP MCP 读信与 Channels 唤醒分别判断.
本轮离线夹具的 Channels 受 host 可用性门控阻塞, 不宣称通道唤醒端到端通过.
官方支持 Console API key 或 claude.ai 认证使用 Channels, 还受 rollout、协议和组织策略控制;
不能仅凭 xats ready 判定 Claude 已接收唤醒并读信.

先列出 pane, 后续使用返回的完整 `pane_id`, 并同时传完整 session ID.  `slot`、tmux `%pane` 和左右位置仅用于观察, 不能作为受限命令的操作 ID.  人工或未知 pane 的 `pane_id` 为 null, 不会因位于自有 session 中而自动获得管理权限.

```sh
aoe -p default managed --context /absolute/private/manager.json pane list <session-id>
aoe -p default managed --context /absolute/private/manager.json pane create <session-id> \
  --tool codex --path /absolute/project --xats-name task-reviewer --xats-team project
aoe -p default managed --context /absolute/private/manager.json pane send <session-id> <pane-id> '请报告进度'
aoe -p default managed --context /absolute/private/manager.json pane capture <session-id> <pane-id> --lines 100
aoe -p default managed --context /absolute/private/manager.json pane remove <session-id> <pane-id>
```

创建 pane 要求父 session 已运行且归当前 context 所有, 最多四个 pane.  默认在右侧分屏, 未指定 `--path` 时使用 session 工作目录.  每个 pane 独立拥有 xats name/team、凭据及运行配置, 不继承主 pane 的协作身份或自定义启动命令.  `send` 会原样输入并提交, `capture` 只读取准确目标, 不受焦点和左右布局变化影响.

关闭附加 pane 不影响邻 pane 或代码目录.  关闭主 pane 时, 从同 owner 且已完成启动的附加 pane 中选择接任者, 保留接任者的进程、逻辑 ID 与 xats 身份.  旧主 pane ID 失效, 不会转指接任者.  若没有可接任的自有 pane, 操作明确失败, 不接管人工 pane; 关闭最后一个 pane 则回收整个 session.  准备尚未完成的 pane 可以稍后再试, 不要改用 slot 或 tmux ID 绕过检查.

父 session 的 stop/remove/restart 也会检查受影响 pane, 遇到人工、其他 owner 或归属无法确认的 pane 时拒绝, 防止通过父级命令绕过保护.  人工 `set-owner` 转移 session 时只同步转移原 owner 的受管 pane, 人工 pane 保持人工所有.

创建后出错会返回 `pane_create_incomplete` 与原 `session_id`、`pane_id`.  保留这两个 ID, 查看状态后按具体错误回收失败记录, 不盲目重复创建.  删除或晋升中途失败会保留恢复信息, 重试原 pane ID, 不编辑存储、key 或 fence.  已移到其他 session 或被替换的目标会被拒绝, 不误删新的占用者.

## 人工接管与转移

人工 TUI 和原有 CLI 保持完整管理权限.  人工可以显式接管或转移一个 session:

```sh
aoe session set-owner <完整-session-id>
aoe session set-owner <完整-session-id> --context /absolute/path/other-manager.json
```

省略 `--context` 表示改为人工所有.  旧管理者随后不能再修改或读取该 session 的终端内容.  接管不改变现有工作内容或 xats 身份.

## JSON 和恢复

成功输出形如 `{"ok":true,"data":...}`, 失败输出形如 `{"ok":false,"error":{"code":"permission_denied","message":"...","session_id":"..."}}`, 失败退出码非零.  诊断写入 stderr.  空列表返回 JSON 数组, 不返回说明文字.

创建可能在记录保存后启动或注册失败.  此时 `create_incomplete` 错误携带 `session_id`, 应使用这个 ID 查看和重试, 不要盲目重复创建.  session 进程状态与 xats 注册就绪是两个独立结果, 不应把创建成功当作已经加入协作.  xats 身份冲突会失败, 不自动改名或覆盖已有协作者.

列表中的 `status_source: "stored"` 表示状态来自 AoE 保存的数据, 不是实时进程探测.  xats 就绪由精确身份状态查询确认.  并发修改同一记录可能返回冲突, 应重新读取后再操作, 不要直接覆盖存储文件.

普通 `aoe send` 与两种 managed send 共用具名临时缓冲区传输,
向准确目标执行显式粘贴后再发送提交键.  应用启用 bracketed paste 时,
文本作为一次粘贴事件输入, 避免快速逐字输入导致 Enter 被当作粘贴内换行.
缓冲区在粘贴后删除, 加载或粘贴失败不会继续发送提交键.
`sent=true` 表示传输和提交键命令成功, 不表示应用已执行任务.

发送修复已在隔离容器中通过真实 Codex 0.164.0 local/remote TUI 的文本、
`/status`、Unicode 和多行输入验收, 并验证 Bash、准确目标、缓冲区隔离及故障处理.
AoE 保留传输字节, Codex 提交时会去掉消息尾部换行.  此项验收仅覆盖发送链路,
不代表 managed Codex 创建、MCP 身份保持或其他 runtime 的完整生命周期验收.

自动 xats Codex 暂不接受配置中的自定义启动命令或额外启动参数, 避免参数覆盖受控 thread 或 profile.  模型使用当前 Codex 默认配置.

本次执行了编译、严格 clippy、隔离 CLI 与 xats 协议回归.  `tests/managed_cli_smoke.py` 和 `tests/managed_xats_cli_smoke.py` 使用临时 HOME 与 tmux 替身.  `tests/managed_xats_integration.mjs` 接受 AoE、Codex、xats CLI 的绝对路径, 验证独立 daemon 与 app-server 的协议绑定; 它不替代实际 launcher 验收.

此前 session 级功能由独立 tester 在断网、无宿主挂载的一次性 Linux 容器中验证了实际 shell 生命周期、TUI 并发保存、人工重启删除、三种 agent 的自动绑定与重启身份保持、旧 fence 拒绝、Claude 首次失败恢复及资源回收.  最终用例没有模型请求.  版本为 tmux 3.4、Codex 0.164.0、Claude Code 2.1.299、OpenCode 1.19.4.  该结果不替代本轮新增 pane 的验收, 未运行宿主真实 tmux 测试或全量测试套件.

新增 pane 功能的独立验收在 Linux arm64 隔离容器完成, Codex/OpenCode 与通用范围 13 组通过, 0 失败、0 阻塞; Claude 另列 user-deferred.  覆盖真实双 pane 收发、主 pane 接任、原始与接任后主 session 恢复、人工/其他 owner保护、旧 ID/helper拒绝、跨 session 移动零副作用以及数据库/JSON/资源清理失败恢复.  运行源码 hash 已与交付工作区核对, 测试容器已清理.  macOS 宿主只执行编译及无真实 tmux 的回归, 不将隔离 Linux 结果声称为宿主实时终端验收.

## 只读总览与 TUI 实例

无需 manager context 即可盘点人工 session 和其它 owner 的 session.  总览不读取终端内容, 不赋予控制权限, 不修改 owner 或补写 pane ID.

```sh
aoe -p default overview
aoe overview --all-profiles
aoe profile instances
```

`overview` 固定输出 JSON envelope.  `data.profiles` 包含每个 profile 的 `groups` 和 `sessions`, session 下的 `panes` 包含完整持久 ID、主从关系、tool、布局坐标和 live 观测.  人工 pane 无持久 ID 时返回 `pane_id: null`.  空 group 和停止 session 仍保留.

`status_source: stored` 表示保存的 session 状态, `status_observed_at` 是本次读取时间, 不是状态更新或进展时间.  `live: true` 仅证明观察时 pane 存在, 不证明 agent 在推进任务; `live: null` 表示观测失败, 不能当作停止.

每个已观测 pane 的 `xats` 独立提供身份和 runtime 状态.  仅精确绑定验证成功才返回 `identity.status: verified` 和 name/team.  xats 旧版缺少接口、离线或响应无效时返回 `unavailable`, 历史绑定证据不足时保持 `unverifiable` 等状态, 不按 title/cwd 推断.  `runtime.status: ready` 与 pane 存活、身份已验证分别表示不同证据.  输出不包含 token、identity key、fence、环境、完整命令或聊天内容.

`profile instances` 列出各 profile 的 TUI 登记.  `verified` 表示 PID 与登记的进程启动时间匹配, `stale` 表示进程退出或 PID 被复用, 旧版空登记为 `unverified`, 损坏登记为 `invalid`.  多个 TUI 分别展示, 不推测唯一当前窗口.  重开旧 TUI 后才会写入新版启动证据; 查询本身不清理登记.  TUI 内成功切换 profile 会迁移本实例登记, 正常退出清理当前登记, 不影响其它 TUI 的记录.

正常与局部失败的查询均 exit 0, 局部失败由 `data.partial: true`、具体来源和错误标识.  致命错误 exit 1 并返回 `ok: false`, 参数错误 exit 2.  结果是带观测时间的多源盘点, 不是跨文件、tmux 和 xats 的原子快照.  活动 SQLite 数据库与 WAL 复制到私有临时目录后读取, 原目录不会生成查询锁、schema 或 SQLite sidecar; 复制期间检测到写入则显式降级, 可重新查询.

每个 profile 的 `errors` 按相同 `source` 和完整 `message` 聚合, `affected_count` 表示受该原因影响的观测条数, `observed_at` 保留首次观测时间.  不同 profile、来源或原因分别计数, 每个 pane 仍保留自己的原因.  lookup 场景的计数是受影响 pane 数, 不等于 profile 的 pane 总数.  xats lookup 返回 HTTP 404 时会明确提示端点不受支持, 不受旧服务的 HTML、纯文本或 JSON 响应体影响, 不自动升级服务或重试.

只读总览的独立 Linux 容器验收共 12 组通过.  真实身份验证使用原生 OpenCode 前台进程、真实 tmux/xats 和预置登记夹具, 不代表注册流程回归; ready 分支仅有接口模拟证据.  Claude 按用户要求暂缓, 真实 Codex launch 行及 remote 403 未实测.  精确身份查询需要 xats 提供 `/api/runtime/pane/lookup` v1, 未升级的 daemon 将明确返回 unavailable, 本功能不会自行部署或重启 xats.
