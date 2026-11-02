# Codex 双 pane 恢复绑定

本次修复覆盖启用 xats 的 Codex pane.  非 xats Codex 保留原有采集和恢复行为,
其按目录和启动时间匹配 rollout 的风险不在本次修复范围内.

## 问题

同目录同时启动两个 Codex 时, rollout 的产生顺序不代表左右 pane 的归属.
按启动时间选择最早未认领文件, 可能把两个 thread 写入对方的 slot.
后续 `r` 会按错误记录恢复上下文, 而 xats 按 pane 身份恢复的 name 仍然正确.
`c` 创建新上下文, 不涉及恢复旧对话, 但新上下文的绑定同样必须准确.

## 修复

启用 xats 的 pane 使用每次启动产生的 `xats.agent_id` 作为 launch ID.
AoE 从该 pane 的 Codex 进程树读取 launch ID, 调用本机
`POST /api/codex/binding/lookup`, 严格校验协议版本, pane ID, launch ID 和
thread UUID.  查询前后必须仍是同一 pane 进程和同一次启动.

启用 xats 的普通 TUI Codex pane 在每次启动、resume 和 restart 后, 会等待准确的 launch 绑定和已注册身份, 通过 `/rename` 底层的 `thread/name/set` 将原生会话名设为 `name@team`, 再用 `thread/read` 校验.  身份来自该 pane 的 launcher key 查询, 不从 AoE 标题或目录猜测.  命名助手最多等待 120 秒, 成功一次即退出; 用户之后手动改名不会被持续覆盖.  查询及命名前后重新核对启动代次, 不符合时保留原名, 具体失败原因写入 AoE 数据目录的 `debug.log`.  没有注册身份的会话不命名, 其他 agent 和非 xats Codex 不受影响.

managed Codex 在自己的服务准备阶段直接使用该 pane 已配置的 `name@team`, 不启动普通 TUI 的命名助手.  命名不发送模型输入, 不改变 thread UUID 和已有对话内容.

成功结果与 slot 的 generation, 身份和原线程一起进行 CAS 校验.
slot, pane capture 和绑定证明在一个数据库事务中写入.
普通 capture 不得覆盖启用 xats 的 Codex slot.

重启分为准备, 启动和完成三个阶段.  准备阶段递增 generation 并关闭采集写入.
`c` 同时清除旧 thread, `r` 只接受有证明的 thread.  启动成功后等待新的
launch ID, 旧 launch 的结果仍被拒绝.  启动失败恢复原有已验证来源,
但不回退 generation, 防止旧请求重新获得写入资格.

Codex 宿主机 resume 始终使用当前工作目录下生效的 provider, 不沿用历史线程
保存的旧 provider.  远程恢复先通过同一 app-server 的 `config/read` 解析配置,
再向 `thread/resume` 显式传入 `modelProvider`, 并校验返回的 provider 与原 UUID.
本地恢复确保 Codex daemon 已启动, 通过其 stdio proxy 查询当前配置和准确历史,
再向原 Codex resume 命令追加 `--config model_provider=...`.  此操作不重启已经
运行的 daemon.  未指定 provider 时使用 Codex 默认的 `openai`.
此行为不改写用户配置文件, 不清空历史, 不应用于容器和完整自定义命令覆盖.

`r` 和 `R` 的普通本地, 原有 xats 和 managed xats 恢复均采用上述规则,
覆盖主 pane 和附加 pane.  远程恢复在目标旧 TUI 退出后执行, 避免旧订阅阻止
切换.  managed xats 直接退订, 不先恢复可能已从配置删除的历史 provider,
并将本次选定的 provider 写入 AoE 生成的客户端 profile, 保持后续 TUI 一致.

Codex 0.166.0 会在恢复前拒绝历史 provider 不匹配, 不能只依靠恢复参数覆盖.
AoE 在 Resume 和保留上下文的 Restart 中比较 `thread/read` 的历史 provider 与
当前配置.  不一致时, 必须由同一服务确认 thread 为 `notLoaded`, 并核对原 UUID,
规范化 cwd、rollout 路径及本地 `state_5.sqlite` 的准确记录.  仅本机回环地址的
服务和本地 daemon proxy 可执行此步骤, 非本机服务不修改本机历史数据.

更新前在 AoE 数据目录的 `codex-provider-backups/` 保存只含准确 UUID、数据库
路径和新旧 provider 的私有备份.  然后通过事务和旧值校验, 仅更新原 thread
的 provider 字段, 不重写 rollout 或其他 thread.  同一 thread 的 AoE provider
迁移使用文件锁.  数据库版本不支持、目录或路径不匹配、历史归档及 thread
仍加载时明确拒绝修改.  这里的加载状态只反映被查询的服务, 不构成不同
app-server 之间的全机器排他保证.

恢复失败后, 只有同一服务再次确认原 thread 未加载, 才回滚 provider.  如果请求
超时、连接中断或结果无法确认, 保留当前 provider 和备份, 明确记录原因, 避免
把服务端可能已经加载成功的 thread 改回旧 provider.  不通过归档线程或重启
共享服务绕过限制.  新建上下文的 Restart 直接使用当前配置, 不修改旧历史.

Codex 远程 resume 不接受 CLI 权限覆盖.  `r`/`R` 恢复时不追加
`--approve-for-me` 或 `--dangerously-bypass-approvals-and-sandbox`.
对已验证且开启 YOLO 的 slot, AoE 在旧 TUI 退出后, 新 TUI 启动前通过同一
app-server 的 `thread/settings/update` 设置 `never` 和 `dangerFullAccess`,
再读取目标线程确认权限.  请求失败或校验不通过时终止本次恢复, 不静默降级.
未开启 YOLO 的 slot 保留服务端权限.  新建会话和 `c` 仍按 pane 的 YOLO
设置选择权限, 普通本地 Codex resume 保留原有行为.

本机 Codex 0.162.1 实测中, 已加载线程的 `thread/resume` 会忽略权限覆盖,
不能代替 `thread/settings/update`.  后者属于实验接口, 旧服务端不支持时
恢复会明确失败.  服务端重启可能丢失 sandbox 设置, 所以每次恢复都重新应用,
不假设配置永久持久化.  更新完成与新 TUI 连接之间发生服务端重启的情况,
不属于本次权限保证范围.

普通 stop/start 或消失 pane 的重新创建会更换 pane ID.  此时显式重绑 slot,
递增 generation 并废除旧证明.  冷恢复使用持久化证明, 不查询可能被复用的旧
pane ID.

xats 或共享 app-server 重启后, 旧 Codex TUI 可能无法重连, 实时绑定查询返回
`pending`, `not_found` 或 `stale`.  此时 `r`/`R` 可使用与当前 slot 的 generation,
身份, pane 和线程一致的持久化证明, 前提是当前进程的 launch ID 已被该证明验证,
或进程树中的 Codex 都明确以 `resume <原线程 UUID>` 启动.  此证据读取系统原始
argv 并保留参数边界, 不从 `ps` 扁平文本推断, 避免把提示词识别成子命令.
`awaiting_launch`
只接受新的 launch ID, 不接受已退役的启动.  查询前后仍须为同一次 pane 启动.
缺少证明, 不同线程, fresh 或 fork 启动, 歧义绑定, 接口失败和协议错误均不能
借此绕过校验.  此后仍执行原有 provider 和权限准备, 不新建对话或修改历史.

## 升级与边界

需要配合提供 binding lookup v1 的 xats daemon 使用.  旧 daemon, 未完成注册,
过期启动或不唯一的绑定都不会触发目录和时间猜测.  缺少证明时 `r` 报错,
不会静默改成新会话.

迁移 v011 创建空的绑定证明表, 不把历史猜测记录自动升级为可信记录.
已错配的历史上下文需要单独核对后修复, 本次代码变更不自动交换实时 pane
或修改其历史线程.

旧非托管 host xats Codex 若从未完成启动绑定, 可由用户明确选择原 thread 后执行:

```sh
aoe -p <profile> session recover-codex <完整session-id> --slot 0 --thread <原thread-UUID>
```

该命令只恢复绑定, 不重启 pane, 不创建对话.  必须提供完整 session ID 和准确 slot,
不按 cwd、标题或最近活动猜选 thread.  服务端 `thread/read` 必须确认原 UUID、相同规范化目录
和 notLoaded 状态; 当前 argv 或 xats 指向其他线程时拒绝, 歧义和接口失败也拒绝.
验证前后核对当前启动, 写入事务再次检查 slot 代次和其他 slot 的重复 thread,
并持久保存显式恢复审计事件.  managed session 和 sandbox 不使用此入口.
每个需要恢复的 slot 完成后, 再执行普通 `session restart` 或 TUI Resume,
后续会复用已持久化的证明.  非托管启动仍要求 xats 预注册可用,
本入口不增加 daemon 完全离线时的启动功能.

存活的普通 host xats Codex 在 CLI `session restart` 时, 现在复用 TUI 的逐 pane
Resume 路径, 不再销毁整个 session 后只重建主 pane.  这是该分支的行为变更:
必须预先确认主 pane、所有目标 pane 的 session 归属及 Codex 恢复证明,
缺失或冲突时在重启前拒绝, 不静默新建会话.  逐 pane 执行可能部分成功后报错,
不是整组原子恢复.  sandbox、自定义命令和 managed 路径不适用这个分支.

接口证明本次启动已经完成登记的 thread, 不证明 Codex UI 当前选中的 thread.
同一进程执行 `/new` 或 `/resume` 后尚未重新登记时, launch ID 和注册信息可能
不变.  完整覆盖此场景需要 Codex 提供与 pane 启动关联的线程选择信号.

## 验证范围

纯逻辑和内存数据库回归覆盖逆序完成的双 pane 绑定, 旧 generation 迟到,
重启窗口内写入, 旧 launch 再次返回, 启动失败回滚, stop/start 重绑,
未经验证的历史记录和普通 capture 覆盖保护.

协议回归覆盖字段严格匹配和 pending, not_found, stale, ambiguous 拒绝结果.
真实 tmux 验收必须遵守项目的实时 session 安全规则, 不能用编译通过代替运行证据.
