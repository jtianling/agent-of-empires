# Managed Codex remote resume 配置丢失

后续状态: 本文保留原 shared runtime 的诊断与路线比较.
已在 AoE 内实现每 pane 自有服务候选, 见 `managed-codex-owned-server.md`.
该候选的 Sol 原生双 pane、实际 xats poke/steer、恢复和旧 shared 跨版本迁移
已经独立验证, 本轮没有修改 Codex 项目或 shared app-server.

2026-10-28, managed 无人值守启动的独立隔离验收发现,
AoE 准备 thread 时正确传入的 MCP header 和 YOLO 权限,
可能在 Codex remote TUI resume 重建同一 thread 时丢失.
此问题与已独立通过的发送修复分开, 不能以 xats ready 或模型底栏代替验收.

实际业务现场的 Codex TUI 版本为 0.164.0.  同日工作期间主机磁盘 CLI
已更新为 0.165.0, 本轮 agent 未执行该更新, 运行中 server 的版本不能据此推断.
原参考仓库 HEAD `8e6a44b428e31f91b21edc97904fcdf4f0931ade`
不作为这两个发布版本的精确源码依据.

通过公开 GitHub annotated tag API 解析出的 release commit 为:

| 版本 | 精确 commit |
| --- | --- |
| 0.164.0 | `6aaac694803004a63c6f393d153c96d22521a6b9` |
| 0.165.0 | `360319d8dee62d87e2d59fc0c21805d79519cfed` |

只读下载的对应源码位于 `/private/tmp/aoe-codex-source-20261028/`,
没有修改实际 Codex 维护仓库.  两个 release 的
`codex-rs/tui/src/app_server_session.rs` 均存在如下传输边界:

- `config_request_overrides_from_config` 转发 reasoning effort,
  不转发所选私有 profile 中的 `mcp_servers`.
- `thread_resume_params_from_config` 在 Remote 模式清空 approval policy、
  sandbox 和 permissions 覆盖.
- 因此仅在 AoE prepare 时传入配置, 或在私有 profile 中写入配置,
  不能证明 remote resume 后的实际 thread 仍保持这些配置.

独立 tester 使用断网一次性容器、真实 Codex 0.164.0、真实 tmux,
本地模型与 MCP 夹具取得了两类失败证据, 没有使用业务 session 或生产 xats.

第一类是真正冷加载.  AoE managed prepare 时 MCP initialize、tools/list
的 header 存在且匹配; 仅重启隔离自建 app-server 后,
按 AoE 生成的 remote 命令恢复时 header 缺失.  实际 get_inbox 工具先出现审批框,
记录失败后仅为定位而手工允许无害夹具工具, tools/call 仍缺少 header,
返回 `FIXTURE_UNBOUND_HEADER`.  手工允许不计为无人值守通过.

第二类与业务现场更接近, 不重启 app-server.  设置 reasoning effort 为 high,
AoE `thread/start` 携带 MCP 配置、never 与 danger-full-access,
响应确认 never 与 dangerFullAccess.  随后真实 TUI 的 `thread/resume`
携带 reasoning 配置却不携带 MCP 配置, 权限字段为空,
响应变为 on-request 与 readOnly, 新 MCP initialize 也缺少 header.
透明协议观测只记录布尔值和权限枚举, 不保存原始 token 或 header 内容.

0.165.0 也已完成 high 接入对照: 隔离 server 和 TUI 均使用公开 npm
精确版本的 Linux arm64 程序, 包完整性校验通过.  同样观察到 AoE start 的
never 与 dangerFullAccess, 随后 TUI resume 回到 on-request 与 readOnly,
新 MCP 初始化和实际 get_inbox 调用缺少 header.  0.165.0 的额外冷恢复与
附加 pane 用例仍以独立 tester 最终报告为准.

仅观察 hot attach 复用旧 MCP 连接不能证明恢复正确.  最终验收必须包含
主 pane 和附加 pane 各自实际 get_inbox 读取明确任务,
并在 high 触发重建、cold resume 以及显式旧记录 YOLO 恢复后继续成立.

另一项已复现的错误是将 `--dangerously-bypass-approvals-and-sandbox`
直接加到 managed remote resume.  0.164.0 会退出并返回
`Permission overrides are not supported when resuming a remote task.`
当前 AoE 候选已移除该参数, 改用受控 thread RPC 准备并核验权限,
私有 profile 保存对应权限.  这解决参数冲突, 仍未解决上述 resume 配置丢失,
不能据此把 managed MCP 生命周期标为通过.

AoE 候选同时提供新建 managed 主/附加 pane 默认 YOLO,
以及显式 `managed restart <完整session-id> --yolo` 迁移旧记录.
显式迁移先检查 owner 和实际占用, 保存目标主/子 pane 模式后停止并恢复该 session;
不带参数时保留历史保存值.  该接口尚不能作为业务现场 MCP 恢复已通过的操作指南.

后续最小跨项目修复建议是由 Codex 维护者处理 remote resume 的显式 profile
配置转发或服务端恢复语义.  至少保留当前所选实例专属 MCP 配置与明确授权的权限,
保留服务端约束, 并覆盖主/子 pane 的实际工具调用和冷恢复.
仅删除 high、重复 register、复用旧 token 或手改生成 profile 都不能作为修复.
每 pane 独立 app-server 会扩大资源生命周期改动, 本轮未采用.
修改实际 Codex 仓库或重启工作 AoE 必须按已明确的跨项目与人工重启边界另行协调.

当前只找到 `/Users/jtianling/workspace/ref-repos/codex` 这个参考 checkout,
其 origin 是公开上游, 不能默认视为个人维护仓库.  跨项目选项拟在获得确认后,
新建 `/Users/jtianling/workspace/codex-managed-remote` 作为维护工作区,
从上述精确 release commit 开始, 不修改参考 checkout, 不自动建远程 fork 或 push.

拟限制修改的 Codex 模块为 `codex-rs/tui/src/app_server_session.rs` 的
remote resume 参数构造及其测试.  必要时增加相邻小模块处理显式 profile 的
覆盖来源, 不改变通用 config loader 或人工默认.  具体行为是只把用户通过
`-p` 明确选中的私有 profile 所拥有的 MCP server 配置与权限选择附到该 thread
请求上, 合并所需的有效 URL 等字段, 遵守服务端约束.  不把其他 profile 的
token 或全局 MCP 表无差别复制到请求, 不改变模型和 reasoning 的选择规则.
该设计尚未实现或验证, 不能保证只改 TUI 就已覆盖全部服务端恢复路径.

验证顺序是参数来源与主/子隔离的窄单测, 再用真实 0.164/0.165 server 与
候选 TUI 验证 high、cold、MCP get_inbox、权限、模型、身份 fence,
并验证人工默认和共享 server 上的旁路线程不受影响.
候选先构建到该维护工作区的私有路径, 不全局替换 Codex.
AoE 当前 managed 入口禁止自定义 Codex 启动参数, 因此私有候选 binary 如何
精确接入目标 managed pane 也须验证, 不能仅因构建成功就要求重启或全局安装.
若客户端修复能兼容现有 server, 可先尝试只更新目标 TUI;
若必须更换共享 app-server 或工作 AoE, 需给出独立验收结果后由 jt 本人重启.

AoE 范围内替代选项的比较如下, 都尚未实现:

| 选项 | 主要改动 | 资源与生命周期 | 验收和维护边界 |
| --- | --- | --- | --- |
| Codex 显式 profile 转发 | 上述 TUI 参数构造与来源限制 | 保留共享 app-server, 需维护候选 Codex binary | 跨项目确认, 两版本真实 MCP 与旁路隔离验证 |
| AoE 每 pane 自有 app-server | `managed_codex` 下新增 server 生命周期, 复用现有 owned runtime 模式, 以该 pane 私有 profile 启动 | 每个 pane 增加一个完整 app-server, 需端点、进程身份、失败回滚和清理 | 不改 Codex, 但扩大 AoE 资源与恢复设计, 需完整生命周期验收 |
| AoE 每 pane WS 中介 | 新增严格绑定 thread/fence 的转发层, 在受控 resume 时补当前配置 | 仍使用共享 engine, 但新增敏感配置代理和自身进程生命周期 | 不是现有受支持入口, 需完整协议透明性、凭据保密及失败测试, 本轮不自行采用 |

没有已验证的现有 AoE 单侧参数可保证完整冷恢复.  在选择上述路线前,
保持业务现场和 claim/fence, 不将现有候选的 `restart --yolo` 当作通信恢复已通过.

本轮证据目录:
`.e2e-test/runs/managed-yolo-mcp-20261028-aoe-tester/`.
其中 `pre-yolo-cold/` 保存冷加载对照,
`v2-high/` 保存同一 server 内真实 TUI 重建的协议与工具观测.

独立第二阶段最终结论为 FAIL/NO-SHIP.  报告合计 75 项,
50 PASS、21 FAIL、2 BLOCKED、2 SKIPPED, 包含首版与第二版对照,
失败断言重复覆盖同一恢复问题, 不代表 21 个独立缺陷.
0.165.0 的附加 pane 和真正 cold 恢复也重现配置丢失与审批阻塞.
非 remote 主/子生命周期与人工默认 12 项通过,
显式旧记录迁移、持久恢复及越权零副作用 17 项通过.
最新 RPC 错误分支只在隔离容器运行 `managed_codex::provider_tests`, 5/5 通过.
这些通过项不能抵消 managed remote 生命周期失败.

离线报告: `.e2e-test/runs/managed-yolo-mcp-20261028-aoe-tester/report.html`.
本轮自建测试容器、应用进程及私有安装已清理, 仅保留证据和限时 loopback 报告服务.
发送修复的独立 57 PASS 结论保持不变.
