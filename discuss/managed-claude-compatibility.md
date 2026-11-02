# Managed CLI Claude 兼容记录

2026-10-29 用户要求继续完成上轮暂缓的 Claude 兼容.
工作区保留此前 provider 和 owned Codex 的未提交改动,
本轮不替换已安装的 20261028 冻结候选, 不操作业务 session.

独立 baseline 使用真实 Claude Code 2.1.299、当前生产 xats dist 的隔离副本,
本地 Anthropic SSE 夹具和一次性 network-none、无宿主挂载的 Linux 容器.
Claude 实际执行 Bash 及 HTTP MCP get_inbox, 工具结果中的任务返回模型,
对话文件已有真实 user 与 assistant 条目后才执行冷恢复.

已复现的失败包括:

- Claude 主/附加 pane 及 shell 主下的 Claude 附加 pane 冷恢复后更换原生 UUID.
  冷恢复前 JSON UUID 与 pane_live UUID 相同, 但 agent_slot.native_session_id 为空.
- 无 xats 的 Claude 主 pane 及附加 pane 晋升后, restart 和 stop/start
  实际返回 `Session ID ... is already in use`, 无法恢复原有对话.

第一类根因是 hook 仅写 pane_live, 持久 slot 依赖 AoE TUI 的 reconcile.
纯 CLI 工作流没有该后台过程, 生命周期读取空 slot 后误选 Fresh.
不能为了让测试通过而手动补 DB 或启动 TUI, 产品应自行保存准确目标的 capture.
第二类根因是主 pane 的 Claude resume 限定于 managed_xats,
没有 xats 的受管会话即使已有对话, 仍以旧 UUID 执行 fresh 创建.

首候选在停止、选取恢复模式、附加 pane 换代和主 pane 接任前,
校验准确 session/pane marker、slot、identity key 和 generation,
再按保存的 UUID 匹配 capture, 用条件更新补齐空 native.
只更新当前 session 的准确槽位, 不调用具有全 profile 副作用的 reconcile_all.
读取 capture 的真实 SQL 错误需要传播, 不能当成缺少 capture 而另建对话.
无 xats 的主 pane 恢复复用相同判定规则.
首次尚未形成可恢复对话的预分配 UUID 仍需独立负例,
不能把 SessionStart、进程就绪或 xats ready 视为完整对话存在证明.

第二候选已取得独立核心回归 30/30 PASS:
空对话冷恢复 6 项、真实 Claude 主/附加恢复 15 项、
无 xats 主及接任后恢复 9 项, 均保留各阶段 source/binary hash.
SQLite 回归已执行 4 项.  独立 review 发现的主 Fresh 影响子恢复、
接任部分完成后重试和 Fresh 旧 slot 问题均已作最小修正.

最后一处修正处理合法失败状态 JSON UUID B、slot 和旧 capture 均为 A:
仅当旧 capture 的 agent 与 UUID 精确匹配已保存 slot 时, 保留旧值并允许重试.
真实新 B capture 到达后才作条件更新, 不提前清空旧对话破坏失败恢复.
新增第 5 个 SQLite 用例验证该序列以及错误 agent 仍被拒绝.
各轮 source/binary hash 独立保留,
不把第二候选的 30 项结果套用成最终快照的完整验收结论.

第二候选随后又取得 35 项身份保护回归 PASS,
包含真实 vendor Claude 进程 inode、收件游标、sender read 回执、
旧 helper 在真实 Claude Bash 进程树内的拒绝、旧 token 403、旧 fence 409、
其他 owner 零副作用和接任后的实际收任务/冷恢复.
真实 TUI Fresh 注入 CLI 失败确实复现第二候选的同 ID 重试失败,
最终候选只对该负例、新增第 5 个 SQLite 用例及必要 smoke 作窄复测.
最终源码的独立 review 已 PASS, 之前三项 CRITICAL 全部关闭,
manifest 为 `f77af5aff637c1e380f544aff60c7ba179bfb91b4da94239751f3cf54e23d243`.

最终候选的 5 项 SQLite 回归与 Fresh 故障重试 9/9 均实际通过.
后者使用真实 TUI Fresh 并注入 CLI 退出 73, 再经公开的
`managed restart --yolo` 全 pane 路径恢复, 两原生 UUID 都更换,
两 agent ID 保持且 cursor 不回退, 两者实际 get_inbox 读到新任务.
普通 `managed restart` 仅重主 pane 的既有语义单独记录,
未把等待全 pair 就绪的过宽夹具断言当作产品新失败.
五条核心 cold/promotion 路径还核对了原始 transcript,
前后唯一任务和回合都在相同原生对话文件中保留.

当前产品没有安装 SessionStart hook.  自然首次 prompt、无 user/model turn 时,
未观察到该强前提下的 pane_live capture, 不手工调用 hook 伪造前提.
实际负例使用正常无 capture 的首次冷恢复和 CLI 启动失败后同 ID 重试.

Channels 验收必须与 HTTP MCP 分开.  离线夹具中的真实 Claude 提示
`Channels are not currently available`, 本轮 channel wake 因 host 可用性门控阻塞.
这不代表 API key 认证方式本身不支持 Channels.
[官方说明](https://code.claude.com/docs/en/channels) 支持 claude.ai 或 Console API key,
同时受 rollout、协议和组织策略控制.  未发现正式的离线强制启用开关.
本轮不借真实凭据、不伪造认证或远端配置来转绿.
xats daemon ready 只证明其所见的进程、proxy/订阅和业务关联,
不能证明 Claude host 已接收 channel 消息并实际读信.

本机候选保存于独立路径:

```text
/Users/jtianling/workspace/agent-of-empires/target/review-builds/20261029-managed-claude/aoe
```

macOS arm64 候选 SHA256:

```text
004135d2c3cafda636e418f217149a67a945a12e22792beceab80f1f79d74fda
```

构建命令为 `cargo build --locked --config profile.dev.package.agent-of-empires.debug=1`,
版本 `aoe 1.99.0`, dev 构建, 包含原工作区已有的未提交改动, 不是干净 HEAD release.
同目录 `build-info.json` 和 `SHA256SUMS` 记录状态及校验.
`review-source-info.json` 保留审查时快照, 最终可交付状态以 `build-info.json` 为准.
macOS 本机仅编译、严格 clippy、fmt 及无真实 tmux 的 CLI 回归,
真实 Claude 生命周期证据来自隔离 Linux, 不扩述为 macOS 全生命周期已实测.
验收阶段没有 commit、全局安装、push 或操作任何业务 session.
2026-10-29 验收后, 用户明确授权全局安装, 已将该候选原子安装到
`/Users/jtianling/.cargo/bin/aoe`, 版本和完整 SHA256 均核对一致.
旧二进制备份及安装记录保存在候选目录的 `global-install-backup/`
和 `global-install-info.json`, 没有重启运行中的 AoE 或业务 session.

统一报告为 85 PASS、0 FAIL、2 BLOCKED.
两个阻塞分别是离线 Channels 可用性门控和当前产品未自然出现的
SessionStart 已写 capture 但无 turn 的强前提, 没有通过伪造前提转绿.
最终 Linux 产物 SHA256 为
`d4ea1b92bbf219893bbaf12bd338a85751ba725c5dcd7da53c2ceef0f0f484f2`,
202 项 Rust/Cargo 生产源码与最终 255 项 review 清单及工作区全部匹配.
历史失败候选曾在其私有 namespace 留下进程, 不把它们描述为产品自行正常清理.
最后精确移除本轮一次性容器, 并确认精确名字查询为空,
清除了全部被测进程和状态, 未触及宿主 tmux、其他容器或生产服务.

[查看 E2E 证据报告](http://100.77.111.113:52182/report.html),
服务实际到期时间为 2026-10-31 03:30:56 UTC.
报告已完成真实浏览器的 87 张用例卡、状态筛选、文本展开及桌面/390 窄屏展示核验,
本轮浏览器 tab 已关闭, 固定报告服务继续保留至到期时间.
离线备份位于
`.e2e-test/runs/managed-claude-independent-20261029-023911-r6jro7uo/report.html`.
