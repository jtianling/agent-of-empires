# Managed Codex 自有服务候选交付

当前状态: 候选已冻结, 已执行范围通过独立验收与最终 review.
统一报告为 130 PASS, 0 FAIL, 1 BLOCKED, 3 SKIPPED.
Linux clippy 因镜像缺少组件 BLOCKED; 更细 profile 原子写、最终 DB save、
respawn 精确失败注入未执行, 不声称这些路径已实测通过.
该状态表示产物可供 jt 审阅与决定启用, 本轮没有执行激活.
之前交付的旧 hash 不再代表当前候选.

本轮仅开发、构建与隔离验收, 没有 commit、全局安装、push、PR、发布,
没有重启工作 AoE、共享 Codex app-server 或操作 Monkeys 业务 session.
原工作区的 provider 等未提交变更保留, 当前构建包含它们,
不能把此构建说成干净 HEAD 的 release.

本机候选是新的独立路径, 不复用上一阶段 NO-SHIP 的 send-managed 目录:

```text
/Users/jtianling/workspace/agent-of-empires/target/review-builds/20261028-owned-codex/aoe
```

命令核对为 `aoe 1.99.0`, Mach-O arm64, dev 构建.
最终本机命令为 `cargo build --locked --config profile.dev.package.agent-of-empires.debug=1`,
只降低本 package 的调试符号级别, 优化与 debug assertions 保持 dev 模式.
该配置强制主 lib 与 bin 使用新 fingerprint 重新编译,
不删除共享 target 或业务二进制.  容器也已仅清理其私有 package 后确认真实 16 项 runner,
不将旧 12 项缓存结果当作新增测试通过.
SHA256 由 shasum 解析, 同目录 `SHA256SUMS` 可重新核对.
Linux 隔离 E2E 的二进制另行记录 hash, 不与 macOS 本机构建混淆.

本机候选 SHA256:

```text
a1a3ef973ea0a034595a8f0069404c40632ebe045f3dd8f8f736c1fe6428d01c
```

最终独立 review 的 253 项清单 SHA256:

```text
3558c1f20733cd44e62e075f692d93d7b153de27d13c101df88bcd788b686551
```

干净 Linux 二进制 SHA256 为
`9dfefa9480b9ab99345ab5a255c4fb104762832d80b0afaaf2e09975abe7b73b`.
它的 200 项 Rust/Cargo 生产源码逐项属于上述 253 项 review 清单,
全部与当前工作区匹配, 没有临时测试模块声明或文件.
最终源启动记录 I/O 负例 7/7、干净单测 16/16、无 shim Sol/env smoke 6/6 通过.
最终被动审计覆盖 14 个 fixture 与 23 条晚生后代事件,
活跃测试进程、观察到的监听端口和 zombie 均为零, 随后移除准确的一次性容器.

[最终报告](http://127.0.0.1:55993/report.html) 的 loopback 服务最多保留一小时.
离线报告位于
`.e2e-test/runs/owned-codex-single-20261028-aoe-tester/report.html`.
早期 FAIL 与各阶段 hash 仍保留, 核心矩阵的历史证据没有套用最终二进制 hash.

最后一次修正统一了 owned server 记录、物理位置校验与 runtime 配置的
profile 来源.  普通 human CLI 也使用已经选定的 profile,
避免根据 cwd 重新推导 profile 后出现 stop 返回成功但后台未退出.
macOS 的 fmt、严格 clippy 和无真实 tmux 的 send CLI 回归已通过.
独立 review 的 253 项源码快照为同目录 `review-source-info.json`,
该文件保留审查时的状态, 最终候选可用性以 `build-info.json` 为准.

实现让启用 xats 的 managed Codex 主/附加 pane 各用自己的 app-server,
持有当前代 MCP header 与保存的权限, 不改共享用户模型或人工默认.
新记录默认 YOLO, 旧 false 不自动批改;
`restart --yolo` 在所有权及实际占用检查后显式升级该 session 的自有主/子 pane,
停止并恢复整个准确目标, 保留逻辑 ID.

旧 shared 在线迁移保留 native thread、agent ID、pane 关系和收件游标.
先只读拒绝活跃旧 thread, 再在受检停止后只给准确目标 thread 设置卸载延迟零,
退订并确认目标不在 loaded 列表, 才在新服务恢复同一 native ID.
不停止 shared server, 不改其全局保留策略, 不 fork 新对话或普通 register 重绑.
0.164→0.165 与 0.165→0.165 的独立版本迁移均已取得实际任务证据.

出现创建、配置、保存、绑定或启动失败时, 保留原完整 session/pane ID 和具体错误.
使用同一原 context 对同一 ID 检查与重试, 不盲目重复 create.
若出现未知 PID/进程组、活跃旧 thread、持续订阅或卸载超时,
停止推进并检查具体日志, 不按端口清理、不回滚旧 fence/token、不手改存储.
跨服务检查与换代不是原子事务, 失败可能已推进合法的新代,
重试应基于保存结果, 不能凭先前 fence 再造旧运行时.

旧 AoE TUI 与新生命周期不能混用.  client_config 是 opaque JSON,
普通并发保存可保留内部 owned 字段, 但旧 ClientConfig 拒绝未知字段,
旧 stop/remove/restart 也不会准确回收新服务.
启用候选前, 由 jt 在对应 AoE 前端正常退出后使用本候选重开同 profile.
不按 PID 强杀, 不执行 tmux server 清理, 不要求停止其他 agent session.

UTC 2026-10-28 06:04:49 的只读登记中,
auto-coworkspace-bedc 的 2706 是 stale, 29459 为 legacy unverified.
另只读 OS 核对确认 29459 是 aoe, cwd 为 coworkspace,
启动时间本地 2026-10-26 09:07:34, 无显式 profile 参数.
这仅供 jt 识别界面, 不是自动重启依据.
源码正常退出只清 AoE 快捷键、通知 UI 并恢复终端,
没有停止 agent session 的操作; macOS 现场退出影响未实测.

由 jt 重开目标前端时可使用:

```sh
/Users/jtianling/workspace/agent-of-empires/target/review-builds/20261028-owned-codex/aoe -p auto-coworkspace-bedc
```

在候选、profile 和原 context 经 steward 核对且 jt 决定启用后,
由 steward 使用自己的原 context 执行受限操作, 不是本轮已执行的动作:

```sh
/Users/jtianling/workspace/agent-of-empires/target/review-builds/20261028-owned-codex/aoe \
  -p auto-coworkspace-bedc managed --context "<steward原context绝对路径>" \
  restart 9afb9f0e2e554117 --yolo
```

随后用同一 context 的 `managed pane list` 核对原 tester pane
`f9aa8bc89b1a4e2e`, 并以两工作线程自己读取任务、工具结果及 ack read 验收.
不要仅凭 ready、模型底栏或 CLI exit 0 判定现场恢复完成.
这是明确目标的命令示例, 没有将 session ID 写进产品逻辑.

完整运行证据来自 Linux arm64 隔离环境, macOS 本机仅编译与只读核对.
macOS 严格 clippy 通过; Linux fresh build 有 HEAD 已存在的
`src/process/linux.rs` 未使用 Command import 警告, 本轮未混修或声称 Linux
`-D warnings` 全绿.
一对 Sol/high agent 含 TUI/wrapper 和独立 daemon 的 PSS 实测约 1.36 GiB,
空闲 5 秒窗口约占单核 CPU 0.8%.  资源分解与多任务线性估算见
`managed-codex-owned-server.md`, 不作为 macOS、忙时、复杂 MCP 或容量承诺.
当前无需全局安装才能审阅候选; 是否正式安装应由 jt 在核对效果和成本后决定.
