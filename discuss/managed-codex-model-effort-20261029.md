本次补齐 managed Codex 主 pane 与附加 pane 的模型及推理强度参数.  `managed create` 与 `managed pane create` 均支持 `--model <id> --effort <effort>`, 仅用于带 xats 身份的 Codex pane.  参数绑定持久 pane 身份, 提升为主 pane 时一并保留.

新建及恢复线程时显式传入 model 和 `config.model_reasoning_effort`, 远端 TUI 的独立 profile 也写入相同值.  Codex 响应中的实际 model 或 reasoningEffort 不一致时返回错误, 不把回退当作成功.  运行中重启若被服务拒绝, pane 可能已停止, 需要解决具体拒绝原因后重试.

模型目录元数据不能当作后端能力的绝对边界.  本机目录曾只为 Terra 列出 none, 但 jt 当前会话确认 Terra 支持 xhigh.  因此显式 effort 不受目录硬拒绝, 由实际线程响应和独立真实 turn 验收确认.  不应修改共享目录来掩盖这一差异.

本次曾把验收消息投给已离线的 tester, 实际在线的队友是 aoe-tester.  guard_failed 是唤醒失败, 不是测试正在执行的证据.  已通过 xats 团队状态查明并交给在线验收员.

候选 SHA-256: `a6534448c86782d62c781ac2c5d9c591d3eda33fcc35e3e58a7e8009dfab4f4e`.  来源为 `3dfcdf2634239dd55b2313a4fbff4ab22d3b843f` 加工作区未提交改动, 不是新的 Git 提交.  经 jt 当前会话明确要求, 已原子安装至 `/Users/jtianling/.cargo/bin/aoe`, 安装后哈希与候选一致, 两个创建入口的帮助均确认包含 model 和 effort 参数.  旧二进制保存在 `target/candidates/aoe-before-global-install-6b9d2fc9e6160e4d840e71e637dace14337ff68a45064eb49e16a3362afabe40`.  运行中的旧 TUI 未重启.

编译与格式检查已通过, 模拟测试仅编译, 未执行宿主全量测试.  独立真实验收确认 Sol 和 Terra 均能以 xhigh 完成双 pane 回合.  stop/start 后保留各自 thread ID, fresh 后更换各自 thread ID, 两种恢复后均可继续完成独立真实回合.  fresh 前后的隔离 rollout turn_context 进一步确认两个模型的 effort 均为 xhigh.

独立验收确认旧安装版对含 model 的隔离记录读取失败, 报 unknown field model, 失败的 rename 未改变存储.  旧 TUI 预加载旧格式后读取新增字段会自行退出, 报 unknown field effort, 因此未能实际执行旧 TUI 的后续保存交互, 不能声称并发保存通过.  正式使用前需要由 jt 手动用候选版本重开访问同一 profile 的旧 TUI, 不需要停止其托管的 agent session.

独立报告为 [.e2e-test 本机 HTML](../.e2e-test/runs/managed-model-effort-20261029-125300-6t0lu_oh/report.html).  报告共 7 项, 5 项通过, 旧版兼容项失败, 旧 TUI 保存交互因提前退出而阻塞.  未独立捕获 thread/start 或 resume 的原始 RPC 响应, 真实回合证据来自终端输出、thread ID 和隔离 turn_context.

原始 rollout 和临时认证副本未保留, 证据仅包含脱敏选择字段与测试标记.  自建 managed session 的 remove 和私有 TUI 精确关闭返回成功, 独立 daemon 已退出, 临时目录已删除, 未进一步穷尽核验全部残留.  未操作业务 session.
