## Context

已有人工 CLI 覆盖生命周期操作, Instance 尚无 owner.  Storage 使用 JSON 整表保存, TUI 可持有旧快照.  xats 已有 identity key、运行时代际及本地控制面, 应复用而非另建注册系统.

## Goals / Non-Goals

目标是本地 agent 通过受限 CLI 管理自己创建的 session, 与人工 TUI 并行使用, 创建时支持明确 xats 身份.  非目标为远程访问、MCP、操作系统强隔离、自动删除代码资源.

## Decisions

- 使用独立 managed 命令组, 提供身份初始化、创建、列表、详情、启动、停止、重启、重命名、发送、读取输出、删除.  管理上下文为本地持久化的随机身份凭据, 每次调用通过环境或上下文文件读取, 不接受任意 owner 字符串作为授权.  人工入口提供显式接管或转移所有权.
- owner 写在持久化 session 数据中, 缺失表示人工所有.  xats name/team 只表示新工作 agent 的协作身份, 不参与 owner 判定.  fork 不复制所有权凭据或 xats 身份凭据.
- 受限入口统一校验完整 ID、上下文、owner 和输入 schema, 校验通过后才能触发 tmux、文件及 xats 副作用.  人工 session 只公开基本信息和状态, 终端内容及启动命令环境不公开.
- 生命周期复用已有实现, 但删除采用保留代码资源的明确路径, 不调用会按配置删除 worktree、分支或共享资源的通用清理.
- Storage 增加锁和原子写入, 并基于加载时快照合并无冲突变更或拒绝冲突, 而不是仅锁住整表覆盖.  managed 授权与操作需避免所有权变化窗口, 人工转移与受限操作协调.
- xats 使用 `/api/managed/{prepare,bind,status,release}` v1 控制面.  稳定 identity key 与每次启动的 instance UUID 分开, daemon 返回 claim/generation fence 及本代 MCP token.  创建时预留身份, 启动时绑定准确运行坐标, 状态查询核对 fence.  重启先持久化待请求 UUID, 再 CAS 新代, 最后停止旧进程.  旧 daemon 或不支持的协议明确失败, 不用 prompt 模拟成功.
- Codex 通过准确 thread/start 响应获取 ID, thread/name/set 持久化后使用私有 profile resume.  单线程 unsubscribe 后重新 resume 才能轮换 MCP header, 禁止全局 MCP reload.  Claude 通过带独立 token 的 channel proxy 自动绑定准确 pid/csid, 业务连接另携带该代 token.  OpenCode 使用每 slot 独立 server 的精确 session 和动态 MCP 配置.  token 不进 argv、日志或共享 server 全局配置.
- 首批自动 xats 注册支持 Codex、Claude 和 OpenCode.  当前 Kimi 共享引擎缺少可验证的每 session managed token 注入通路, 创建前明确拒绝该组合; 普通 managed Kimi 不受此限制, 本次不修改 kimi 项目.
- stdout 统一 JSON, stderr 记录诊断, 非零退出表示失败.  部分创建失败返回已有 session ID 及具体阶段, 避免遗失可管理的资源.

## Risks / Trade-offs

- 同用户 shell 可绕过限制 → 文档明确接口防误操作的边界.
- 老 TUI 进程仍运行旧代码 → 文档要求更新并重启 TUI 后使用并发管理.
- xats 新契约未部署 → 明确报告不可用, 不虚报注册成功.
- 活跃生产 tmux → 宿主只编译及运行无真实 tmux 的隔离检查.  真实生命周期与 TUI 验收放在断网、无宿主挂载的一次性容器中, 不运行全量测试套件.

## Migration Plan

新增可选字段保持现有 session 为人工所有.  若实现需要更改已有存储形态, 使用项目 migration 系统.  不修改或接管当前真实 session.

## 集成状态

xats-main 已确认 v1 请求与响应契约, 并在 xats 工作区实施服务端.  AoE 不负责发布 xats, 真实 daemon 可用性与本地接口实现完成分别报告.  Codex 启动、线程隔离和 token 轮换使用临时 HOME、独立 app-server、本机假 MCP 实测, 不调用模型或触碰生产 session.

独立容器验收补充的实现约束: tmux pane/window 上下文使用 `=session:` 精确选择器, 归属输出使用不会被 tmux 转换的 `|` 分隔符.  重启等待原 pane 进程退出, 不把 pane-died hook 新建的备用 shell 当作旧 agent.  Codex `loaded/list` 不是订阅状态证据, 准确 unsubscribe 后的凭据轮换以真实 MCP 连接验证.  URL 按服务端规则规范化后比较, thread ID 和 fence 始终精确匹配.  managed Claude 主 pane 禁止旧式 reconnect 输入, 避免自动触发模型 turn.
