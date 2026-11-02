# 受限 session 管理 CLI

## Purpose

为本地管理 agent 提供基于稳定身份和所有权校验的 CLI, 允许创建及管理自己拥有的 session, 保护人工和其他管理者的 session.  保留人工接管能力, 支持安全资源回收、并发保存以及 xats 协作身份的准确绑定与恢复, 并通过结构化结果区分进程状态和协作就绪状态.

## Requirements

### Requirement: 稳定管理身份

系统 SHALL 提供持久化管理上下文, 创建 session 时自动写入 owner, 不接受未经验证的 owner 字符串授权.  既有无 owner session SHALL 视为人工所有.  管理身份 SHALL 独立于 xats 身份和对话 thread.

#### Scenario: 重启管理 agent
- **WHEN** 管理 agent 使用相同管理上下文重新运行 CLI
- **THEN** 可以管理原来自己创建的 session, 不获得其他 session 权限

#### Scenario: 缺少身份
- **WHEN** 没有有效管理上下文时请求变更
- **THEN** 拒绝操作且没有 session、tmux 或 xats 副作用

### Requirement: 受限操作边界

系统 SHALL 支持创建、启动、停止、重启、重命名、发送输入、读取输出和删除自己的 session.  所有受限目标操作 SHALL 使用完整 ID 并在副作用前验证 owner.  人工及其他 owner 的 session SHALL 只提供基本信息和状态, 不提供终端内容或配置秘密.

#### Scenario: 越权操作
- **WHEN** 管理者对人工或其他 owner session 执行停止、重启、删除、发送、重命名或读取输出
- **THEN** 返回明确拒绝且目标及其资源不发生变化

#### Scenario: 模糊定位
- **WHEN** 使用标题、ID 前缀或空 ID 执行目标操作
- **THEN** 拒绝而不触发任何 session 操作

### Requirement: 接入已有 worktree 时同步 agent 配置

`managed create` 和 `managed pane create` 接入已有 linked worktree 根目录时,
系统 SHALL 从主仓库复用 TUI 的 agent 配置拷贝规则, 补齐未跟踪的 agent 目录及已知配置文件,
保留 skills 中的符号链接.  系统 SHALL 保留已有目标目录、文件和符号链接,
仅允许单独列出的缺失配置文件在已有目录内补齐, 且不得穿过目标符号链接祖先写到外部.
普通仓库及子目录 SHALL 不触发此同步.  同步 SHALL 不接管外部 worktree 的删除归属,
Git 识别或单项拷贝失败 SHALL 记录具体警告并继续创建.

#### Scenario: 外部创建 worktree 后接入
- **WHEN** 管理者先创建 linked worktree, 再通过 managed CLI 创建 session 或 pane
- **THEN** 缺失的未跟踪 agent 配置和 skills 按现有规则补齐, 外部 worktree 归属保持原状

#### Scenario: 重复接入已有配置的 worktree
- **WHEN** 目标 worktree 已有 agent 目录、配置文件或符号链接
- **THEN** 保留已有内容和链接, 不合并覆盖整个目录, 不写入链接指向的外部目录

#### Scenario: Git 库无法识别目录
- **WHEN** 有效工作目录的 Git 元数据无法由当前 Git 库读取
- **THEN** 记录具体同步警告并跳过可选同步, managed session 或 pane 仍可创建

### Requirement: 指定 Codex 主 pane 模型

带 xats 的 Codex 主 pane MAY 在 `managed create` 时通过 `--model` 指定模型.  系统 SHALL 在新建及恢复线程时使用该模型并验证 Codex 返回值, 不一致时拒绝启动, 未指定时使用默认配置.

指定模型时 MAY 通过 `--effort` 指定推理强度.  系统 SHALL 在启动及恢复时显式传入并核对返回的强度, 被拒绝或回退时启动失败, 不以可能滞后的模型目录阻挡显式设置.

### Requirement: 默认无人值守模式

`managed create` SHALL 为主 pane 默认保存已有的 YOLO 模式,
并在 start、restart 与 stop 后 start 时保留保存的模式.
系统 SHALL 复用对应 runtime 的批准机制, 不修改人工 session 或共享全局默认.
YOLO SHALL 不改变 owner 检查、xats 身份 fence 或用户指令中的授权边界.

带 xats 的 YOLO Codex pane SHALL 在配置读取进程、托管服务和独立客户端 profile 中,
显式信任其准确工作目录, 避免 Folder access 确认阻塞.
系统 SHALL 不修改共享 Codex 配置, 非 YOLO 启动 SHALL 保留原有目录确认流程.

系统 SHALL 提供显式 `managed restart <id> --yolo`, 在 owner 与实际占用检查通过后,
仅对准确目标主 pane 和全部自有附加 pane 保存 YOLO 模式并停止后恢复整个 session.
既有保存为 false 的记录 SHALL 不因普通 start 或 restart 被自动升级.

#### Scenario: 新建与恢复 managed agent
- **WHEN** 管理者创建受支持 runtime 的 managed agent 并恢复该 session
- **THEN** 主 pane 的 YOLO 配置保持启用, 同 runtime 的人工默认配置保持原值

#### Scenario: 无 xats 的 Claude 对话恢复
- **WHEN** 无 xats 的受管 Claude 主 pane 已完成真实对话, 随后重启或停止后启动
- **THEN** 使用原生 resume 延续准确对话, 即使没有 AoE TUI 也不以旧 UUID 执行 fresh 创建

#### Scenario: 主 pane 尚无对话而附加 pane 已有对话
- **WHEN** 管理者恢复主 Claude pane 尚无可恢复对话的 session, 其附加 pane 已有真实对话
- **THEN** 主 pane 可以选择 fresh, 附加 pane 独立恢复原有对话, 不继承主 pane 的 fresh 决策

#### Scenario: 显式升级旧 session
- **WHEN** 原管理者对旧 session 执行带 `--yolo` 的受限 restart
- **THEN** 主 pane 和自有附加 pane 保存并恢复 YOLO 模式, 逻辑 ID 保留

#### Scenario: 升级目标含外来 pane
- **WHEN** 目标包含人工、其他 owner 或实际占用无法确认的 pane
- **THEN** 在修改持久模式或停止进程之前拒绝操作

### Requirement: 人工管理权

系统 SHALL 保留人工 TUI 和原 CLI 管理所有 session 的能力, 并提供人工显式接管或转移所有权入口.  受限入口 SHALL 不提供任意认领、转移他人 session 的能力.

#### Scenario: 人工接管
- **WHEN** 人工接管管理 agent 创建的 session
- **THEN** 旧 owner 后续变更被拒绝, session 内容保留

### Requirement: 安全资源回收

managed 删除 SHALL 仅处理准确 session 的运行资源和元数据, 保留工作目录、worktree、git 分支及其他 session 共享资源, 不按名称前缀批量清理.

#### Scenario: 删除自己的 session
- **WHEN** 删除带工作目录的自己所有 session
- **THEN** 仅该 session 被移除, 代码目录和其他 session 保留

### Requirement: 并发保存保护

系统 SHALL 防止人工 TUI 和 CLI 的陈旧快照覆盖并发创建、更新或删除, 对同一记录冲突明确拒绝或安全合并.  授权 SHALL 不因并发所有权转移而失效.

#### Scenario: TUI 保存旧快照
- **WHEN** CLI 创建 session 后, 已打开 TUI 保存另一 session 的变更
- **THEN** CLI 创建的 session 及 owner 保留

#### Scenario: 并发删除
- **WHEN** 一个入口删除 session 后另一个入口保存旧快照
- **THEN** 被删除的 session 不会复活

### Requirement: Managed Codex 自有服务

启用 xats 的 managed Codex pane SHALL 使用各自的 loopback app-server,
通过该服务端进程专属配置保持当前代 MCP 凭据、provider 与保存的权限模式.
TUI high reasoning resume、冷恢复及 xats poke 的无配置 resume SHALL 保持这些设置.
任务模型 SHALL 继续按工作目录的有效配置解析, 不改共享用户模型或人工默认.

系统 SHALL 记录每个自有服务的准确 pane ID、launch ID、端点、PID 和进程启动身份.
端点 SHALL 来自该自有进程成功监听后的实际结果, 不猜端口或接管现有监听者.
停止、删除、重启和失败回滚 SHALL 只回收已验证属于该 pane 的进程组,
拒绝 PID、进程组或身份复用, 并保留具体失败原因.
旧代 TUI 退出 SHALL 不回收新代服务, 主 pane 接任 SHALL 保留接任者的服务身份.

#### Scenario: 双 pane 独立任务
- **WHEN** 主 pane 和附加 pane 分别通过真实 xats poke 收到不同任务
- **THEN** 两者使用不同端点与 thread, 各自读取准确任务并能继续调用工具,
  不出现 MCP 人工审批, 不读取邻 pane 的任务

#### Scenario: 原 shared runtime 迁移
- **WHEN** 原管理者通过受检生命周期恢复旧 shared runtime 记录
- **THEN** 使用准确 previous fence 准备新代, 检查旧 thread 状态并切到自有端点,
  保留逻辑身份和收件进度, 不终止 shared app-server 或旁路线程

#### Scenario: 活跃旧 thread
- **WHEN** 旧 shared thread 仍有活跃任务或其状态无法确认
- **THEN** 明确拒绝不安全迁移, 不猜测空闲或按其他 thread 替换目标

#### Scenario: 自有服务启动失败
- **WHEN** 监听、配置、绑定、保存或 TUI 启动失败
- **THEN** 保留可重试的准确元数据, 回收本次已验证的服务,
  不遗留未知进程或端口, 不改其他 pane 的服务

### Requirement: xats 预设身份与恢复

创建 SHALL 支持 xats name/team, 通过实际本地启动集成绑定准确实例, 保持 owner 与 xats 身份独立.  重启 SHALL 复用持久身份, fork SHALL 不继承身份凭据.  冲突 SHALL 明确失败且不覆盖已有绑定.

首批自动 xats 注册 SHALL 支持 Codex、Claude、OpenCode.  当前 Kimi 共享引擎不具备经过验证的每 session token 注入通路时, 系统 SHALL 在创建及预留之前拒绝 Kimi+xats, 保留普通 managed Kimi 支持.

#### Scenario: 指定协作身份
- **WHEN** 管理 agent 创建指定 name/team 的工作 session
- **THEN** 启动集成使用指定身份, owner 仍为创建者

#### Scenario: 身份冲突
- **WHEN** 指定身份已属于另一个实例
- **THEN** 报告冲突, 不改名、不接管该实例

#### Scenario: 不支持的 Kimi 身份注入
- **WHEN** 请求创建带 xats name/team 的 Kimi session
- **THEN** 返回明确不支持原因, 不创建 session 或预留 xats 身份

### Requirement: 结构化结果与真实就绪

managed CLI SHALL 返回稳定 JSON 结果和明确错误码, 将 session 进程状态与 xats 注册状态分开.  第三方错误 SHALL 保留具体失败原因且不泄露凭据.  部分创建失败 SHALL 报告已创建的 session ID 以便后续恢复.

#### Scenario: 进程运行但未注册
- **WHEN** session 已启动而 xats 尚未确认准确实例就绪
- **THEN** 返回未就绪或失败及原因, 不报告协作就绪

#### Scenario: 空列表
- **WHEN** 当前没有 session
- **THEN** 返回有效 JSON 空列表

### Requirement: 父 session 操作保护混合所有权 pane

受限 session 操作 SHALL 检查实际受影响的 pane.  stop/remove/restart 若会终止或重建人工、其他 owner 或占用无法确认的 pane, SHALL 在副作用前拒绝.  session 级 send/capture SHALL 核对实际主 pane 的所有权和占用.  人工入口保留显式管理权.

#### Scenario: 不通过删除父级绕过权限
- **WHEN** 自有 session 中存在人工附加 pane, 管理 agent 请求删除或停止整个 session
- **THEN** 拒绝并指出受保护 pane, 全部 pane 及 xats 绑定保持原状

### Requirement: pane 管理权随人工转移一致更新

人工 set-owner SHALL 同步转移原 session owner 的受管 pane, 保留本来人工所有的 pane.  旧 owner SHALL 立即失去 pane 操作权限.  fork SHALL 不复制受管 pane 凭据和 xats claim.

#### Scenario: 人工接管 session
- **WHEN** 人工接管包含受管 pane 的 session
- **THEN** 原管理 agent 不能再操作这些 pane, 其进程与内容保持不变
