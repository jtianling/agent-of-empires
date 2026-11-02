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
对已验证且开启 YOLO 的 slot, AoE 在重启 pane 前通过同一 app-server 的
`thread/settings/update` 设置 `never` 和 `dangerFullAccess`, 再读取目标
线程确认权限.  请求失败或校验不通过时拒绝重启, 不静默降级.
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

## 升级与边界

需要配合提供 binding lookup v1 的 xats daemon 使用.  旧 daemon, 未完成注册,
过期启动或不唯一的绑定都不会触发目录和时间猜测.  缺少证明时 `r` 报错,
不会静默改成新会话.

迁移 v011 创建空的绑定证明表, 不把历史猜测记录自动升级为可信记录.
已错配的历史上下文需要单独核对后修复, 本次代码变更不自动交换实时 pane
或修改其历史线程.

接口证明本次启动已经完成登记的 thread, 不证明 Codex UI 当前选中的 thread.
同一进程执行 `/new` 或 `/resume` 后尚未重新登记时, launch ID 和注册信息可能
不变.  完整覆盖此场景需要 Codex 提供与 pane 启动关联的线程选择信号.

## 验证范围

纯逻辑和内存数据库回归覆盖逆序完成的双 pane 绑定, 旧 generation 迟到,
重启窗口内写入, 旧 launch 再次返回, 启动失败回滚, stop/start 重绑,
未经验证的历史记录和普通 capture 覆盖保护.

协议回归覆盖字段严格匹配和 pending, not_found, stale, ambiguous 拒绝结果.
真实 tmux 验收必须遵守项目的实时 session 安全规则, 不能用编译通过代替运行证据.
