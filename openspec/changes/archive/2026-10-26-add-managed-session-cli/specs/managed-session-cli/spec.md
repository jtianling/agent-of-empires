## ADDED Requirements

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
