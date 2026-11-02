本次现场是 profile auto-coworkspace-bedc 的 `chat&reviewer`, session ID 为
`e2bb2e6c8d8c4a5b`.  两个 Codex UI 都失去共享服务连接, AoE 两个 slot 的
native_session_id 为空, codex_binding 无记录.  原 chat 和 reviewer 线程仍在本机
Codex 历史库中, `thread/read` 确认可读且为 notLoaded, 不能把 notLoaded 解释为历史丢失.

xats-main 只读取证定位到 `/Users/jtianling/.config/xats/local-daemon.log`:
UTC 2026-10-27 17:05:57 为 pane %68 和 %69 预登记, 有效期至 17:15:57.
seeding 提示在 17:06:07 到 17:06:08 投递, 但两条目标 thread 到次日
08:16:16 和 08:16:24 才注册.  nonce 仍被解析到正确 pane,
但 pending 预登记已经过期, auto-bind 返回 rows=0, 未提交 launch 与 thread 绑定.
注册身份仍返回成功, 留下了旧 pane 和旧 PID 的 runtime 信息.

近因是迟到 challenge、预登记有限 TTL 和注册成功状态未表达绑定失败.
没有证据说明模型为何隔 15 小时才回传, 不推测用户操作或私有对话内容.
注册成功、提示已投递和 thread 历史可读都不能替代准确的启动绑定证据.

AoE 增加显式人工恢复入口 `session recover-codex`, 只接受用户选定的完整 session ID、
slot 和原线程 UUID.  先核对当前启动、明确的 resume argv、实时 xats 冲突、
原线程 UUID、目录和 notLoaded 状态, 验证前后复核启动与绑定,
再通过 Immediate 事务执行去重和原有 CAS, 保存审计事件.
该入口不重启 pane, 不猜历史, 不创建新对话, 不关闭共享服务.
恢复后再走普通 Resume, 后续实时绑定暂时缺失时可复用持久证明.

xats 根因修复需要另一项目的授权, 已按全局规则另请 jt 确认.
未在本项目中直接改 xats, 未重启共享服务.  防复发方向是让 challenge 与预登记
共享有效期, 对迟到注册明确返回未绑定原因, 提供经当前 launch 验证的安全重试,
并覆盖投递后 TTL 过期再迟到注册的回归.  单纯延长 TTL 不能解决任意迟到.

独立验收还发现原 CLI restart 与 TUI Resume 不等价: 普通非托管 session 的旧 CLI
直接销毁整个 tmux session 后重建主 pane, 可能丢失附加 pane 布局并启动新对话.
旧候选 `47de881f01e3e015017daad52738ea61e397c77bc0727a006f44ba22d612b8a6`
的显式绑定和 TUI Resume 通过, 但 CLI restart 失败, 未用该 restart 路径操作业务.

现对存活、非 sandbox、无 command override 的普通 host xats Codex 复用逐 pane Resume.
重启前确认真实主 pane、所有 slot 的 session 归属和 Codex 恢复证明,
缺失或冲突时在任何重启前拒绝.  CLI 同时保留恢复与保存失败原因, 不假报成功.
逐 pane 恢复不是整组原子操作, 允许部分成功后明确报告失败.

最终候选 SHA-256 为 `0d606058d8d265a87429a6fce7720063073d554d7f7d07a21a33172df2eb3a58`,
已通过编译、格式检查、独立静态审查及 6 项独立隔离运行验收, 并全局安装.
独立报告为 [本机 HTML](../.e2e-test/runs/ordinary-codex-recovery-final-proof-20261029-160729-3tgwmfd4/report.html).
pending、stale、not_found 分别连续恢复时, 两个前台保留各自准确历史、UUID 和身份键,
代次递增且每个 pane 的 PID 都换代.  歧义、错误协议、控制面失败、错 UUID、错目录、
短 session ID、重复线程、缺失证明及跨 session pane 均拒绝且未改动目标.
daemon 全程在线, 控制状态由本机代理注入, 不声称完全离线启动可用, 未请求真实模型.

jt 明确授权后, 已为业务 16 号的 slot 0 与 slot 1 分别验证并持久化原 chat/reviewer UUID,
再用最终候选执行正常 CLI restart.  两个 pane ID 保持为 %68 和 %69,
两 slot 代次变为 1, 原 UUID 不变.  当前 Codex 服务只读核验两原线程均为 idle,
主 pane 已显示原历史与输入界面, 两个 xats 身份也已绑定到正确的新客户端 PID.
未重启共享 Codex 或 xats 服务.  沙箱内最初数据库备份失败, 未将该次失败当作备份完成;
客户端重启前已通过授权的一致性备份保存到 `target/recovery-backups/`.

xats 项目的根因修复已获 jt 单独授权, 由该项目的负责 agent 完成实现和测试,
当前尚未在共享服务激活.  binding lookup v1 的严格字段兼容边界保持不变.
Codex 注册结果新增 `launch_binding`, 区分身份注册与准确启动绑定;
challenge 与预登记共享截止时间, 迟到、重复消费或启动替换均不能完成绑定.
安全重试使用当前准确 launch 重新预登记并签发新 challenge, 不延长旧授权.

xats 交付结果为 13 个定向文件共 157 测试通过, 类型检查、临时目录构建和独立审查通过.
独立 MCP/HTTP 契约验收为 14 PASS、0 FAIL、2 SKIPPED, 另有身份键脱敏补验 1 PASS.
该验收使用临时数据库、随机 loopback 端口和模拟 carrier/时钟,
真实 TUI 长延迟与真实 recovery 签发链尚未覆盖, 不能据此声称生产端到端通过.
文档为 [challenge 生命周期与激活边界](../../cross-agent-teams-mcp/docs/launchers/codex-pane-challenge-lifecycle.md).
激活仍需新构建和一次共享 xats daemon 重启, 旧内存 challenge 不可迁移,
重启后必须取得新 challenge.  本轮恢复未执行该共享服务操作.
