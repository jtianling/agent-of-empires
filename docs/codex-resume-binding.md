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

Codex 远程 resume 不接受权限覆盖.  `r` 恢复时不追加 `--approve-for-me`
或 `--dangerously-bypass-approvals-and-sandbox`, 保留原会话权限.  新建会话和
`c` 仍按 pane 的 YOLO 设置选择权限, 普通本地 Codex resume 保留原有行为.

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
