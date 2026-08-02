## MODIFIED Requirements

### Requirement: New session dialog includes right pane tool selector

New Session SHALL 在全部 primary pane 字段和可见分割线之后展示 `Right Pane Agent`。  该字段 SHALL 不受 primary Tool 影响, SHALL 提供与 primary Tool 相同的可用 Tool 列表并在开头加入 `none`, 默认 SHALL 为 `none`。

选择 `none` 时, secondary pane 配置 SHALL 折叠且提交结果 SHALL 不包含 secondary pane。  选择 Tool 后, 对话框 SHALL 展开 pane-scoped Path、适用的 YOLO Mode 与 Cross Agent Team、Worktree。

#### Scenario: Right pane selector 位于 primary 配置之后
- **WHEN** 用户打开 New Session
- **THEN** primary Worktree 后 SHALL 显示可见分割线
- **AND** `Right Pane Agent` SHALL 位于分割线之后
- **AND** `none` SHALL 默认选中

#### Scenario: 用户循环选择 right pane Tool
- **WHEN** 用户 focus `Right Pane Agent`
- **AND** 按 Left 或 Right
- **THEN** 选项 SHALL 在 `none` 和全部可用 Tool 之间循环

#### Scenario: none 不创建 split
- **WHEN** 用户以 `Right Pane Agent = none` 提交
- **THEN** 提交数据 SHALL 不包含 secondary pane
- **AND** session SHALL 只创建 primary pane

#### Scenario: 折叠的 draft 可以在提交前恢复
- **WHEN** 用户配置 secondary pane, 切换为 `none`, 然后在同一次对话框中重新选择 Agent
- **THEN** secondary draft SHALL 恢复之前输入的值
- **AND** 以 `none` 提交 SHALL 丢弃该 draft

### Requirement: New session dialog includes a right pane path field

New Session SHALL 在展开的 secondary pane section 中展示 pane-scoped Path。  该字段 SHALL 只在 right pane Tool 不是 `none` 时出现。  当 secondary Path 和 Worktree 都为空时, 空 Path SHALL 表示 primary pane 的最终 working directory。

secondary Path SHALL 支持与 primary Path 相同的 ghost completion、Right 或 End 接受补全、按 path segment 移动 cursor、`Ctrl+P` directory picker 和 invalid-path indication。

primary Path SHALL 继续作为 session identity 和 group default directory 的来源。  secondary Path SHALL 只影响 secondary pane 和它的 Worktree base。

#### Scenario: 选择 Agent 后出现 secondary Path
- **WHEN** 用户把 `Right Pane Agent` 从 `none` 改为 Tool
- **THEN** secondary Path SHALL 出现在 selector 下方
- **AND** 改回 `none` SHALL 隐藏该字段

#### Scenario: 空 Path 继承 primary 最终目录
- **WHEN** secondary Path 和 Worktree 都为空
- **THEN** right pane SHALL 在 primary pane 的最终 working directory 启动

#### Scenario: Directory picker 只写 secondary Path
- **WHEN** 用户在 secondary Path focus 时通过 `Ctrl+P` 选择目录
- **THEN** 选择结果 SHALL 写入 secondary Path
- **AND** primary Path SHALL 不变

#### Scenario: Ghost completion 相互独立
- **WHEN** secondary Path 存在唯一目录补全
- **THEN** secondary Path SHALL 显示并接受 ghost text
- **AND** primary Path SHALL 不变

### Requirement: Session creation splits tmux window when right pane tool is selected

配置 secondary pane 后, session creation SHALL 先创建 primary pane, 再解析 secondary pane 自己的 Path 和可选 Worktree, 然后水平 split tmux window 并使用 secondary pane 自己的 launch config 启动选定 Tool。

没有 secondary Path 或 Worktree 时, secondary pane SHALL 在 split 时继承 primary pane 的最终 `project_path`。  只有 Path 时 SHALL 原样使用。  请求 secondary Worktree 时, AoE SHALL 从显式 secondary Path, 或空值时从 primary 最终路径解析 repository, 并 SHALL 把 Worktree 路径作为 secondary working directory。

secondary pane 的 resolved directory 和 launch config SHALL 写入自己的 durable slot, 使 restart 和 cold recovery 可以保持它们。  AoE 通过 New Session 启动的每个 right pane SHALL 立即获得 durable slot, 包括 shell。

shell right pane SHALL 通过现有安全 POSIX wrapper 使用用户配置的 shell, non-POSIX user shell SHALL 仍作为最终 interactive shell。

#### Scenario: Right pane Tool 创建 horizontal split
- **WHEN** 用户提交已配置的 secondary pane
- **THEN** AoE SHALL 对新 session 执行 horizontal tmux split
- **AND** right pane SHALL 运行所选 Tool
- **AND** command SHALL 使用 secondary pane 自己的 YOLO Mode 与 Cross Agent Team

#### Scenario: Shell right pane 默认使用 primary 最终路径
- **WHEN** primary pane 最终路径为 `/some/project`
- **AND** secondary Tool 为 `shell`
- **AND** secondary Path 与 Worktree 为空
- **THEN** right shell SHALL 在 `/some/project` 启动

#### Scenario: Shell right pane 使用用户 shell
- **WHEN** 用户 shell 为 `/bin/zsh`
- **AND** secondary Tool 为 `shell`
- **THEN** right pane SHALL 通过 user-shell launch path 进入 interactive zsh
- **AND** 不得先加载 Bash login configuration

#### Scenario: Non-POSIX shell 保持最终 shell
- **WHEN** 用户 shell 为 fish、nu 或 PowerShell
- **AND** secondary Tool 为 `shell`
- **THEN** POSIX wrapper MAY 使用 Bash fallback
- **AND** 最终 interactive shell SHALL 仍是用户配置的 shell

#### Scenario: Managed shell right pane 可持久恢复
- **WHEN** AoE 启动 managed shell right pane
- **THEN** 该 pane SHALL 获得包含 resolved directory 和 pane config 的 durable slot
- **AND** restart 和 cold recovery SHALL 包含该 pane

#### Scenario: 显式 right Path 只影响 right pane
- **WHEN** primary Path 最终为 `/some/project`
- **AND** secondary Path 为 `/some/other`
- **AND** secondary Worktree 为空
- **THEN** right pane SHALL 在 `/some/other` 启动
- **AND** primary pane SHALL 保持 `/some/project`

#### Scenario: Secondary Worktree 独立解析
- **WHEN** primary 与 secondary 配置不同 Worktree branch
- **THEN** 两个 pane SHALL 分别进入自己的 Worktree 路径
- **AND** 任一 pane 的 branch 或 Path SHALL 不覆盖 sibling config

#### Scenario: Durable slot 保存 pane 自己的配置
- **WHEN** secondary pane 启动成功
- **THEN** 其 slot SHALL 保存自己的 directory、Tool、YOLO Mode、Cross Agent Team 和 Worktree metadata
- **AND** primary slot SHALL 保存 primary 对应值

#### Scenario: Restart 恢复各自目录
- **WHEN** primary 与 secondary 使用不同 working directory
- **AND** session restart
- **THEN** 每个 pane SHALL 在自己 slot 记录的目录启动

#### Scenario: Right pane command 禁用 Ctrl-Z
- **WHEN** right pane Tool 启动
- **THEN** command SHALL 使用现有 `stty susp undef` wrapper

#### Scenario: Right pane 单独启用 remain-on-exit
- **WHEN** right pane 创建
- **THEN** `remain-on-exit` SHALL 对该 pane 启用
- **AND** primary pane setting SHALL 不改变

### Requirement: Right pane works with sandboxed sessions

已有 sandbox session SHALL 继续通过现有 add-pane flow 支持 managed pane。  这些 pane SHALL 在已记录的 session container 中运行并使用 container working directory。  New Session SHALL 不提供 Sandbox, 因此它的 right pane selector SHALL 不与隐藏的新 Sandbox state 组合。

#### Scenario: Existing sandbox session 可以添加 managed pane
- **WHEN** 用户向已有 sandbox session 添加 managed pane
- **THEN** command SHALL 使用 session container exec wrapper
- **AND** host-side Path 或 Worktree SHALL 不覆盖 container working directory

#### Scenario: New Session 没有 Sandbox 交互状态
- **WHEN** 用户在 New Session 配置 right pane
- **THEN** Sandbox checkbox SHALL 不存在
- **AND** secondary Path SHALL 不因 invisible Sandbox value 被隐藏

## ADDED Requirements

### Requirement: Pane controls are independent

primary 与 secondary pane settings SHALL 独立编辑和提交。  改变一个 pane 的 Tool、YOLO Mode、Cross Agent Team、Path 或 Worktree SHALL 不改变 sibling 对应字段。

#### Scenario: YOLO 选择独立
- **WHEN** 两个 pane 都使用支持 opt-in YOLO 的 Tool
- **AND** 用户只对 secondary 开启 YOLO Mode
- **THEN** 只有 secondary launch command SHALL 包含对应 YOLO treatment

#### Scenario: Cross Agent Team 选择独立
- **WHEN** 两个 pane 都使用受支持 Tool
- **AND** 用户只对 primary 开启 Cross Agent Team
- **THEN** 只有 primary pane SHALL 使用 Cross Agent Team launch path

#### Scenario: 字段可见性只依赖同一 pane
- **WHEN** primary Tool 不支持 Cross Agent Team 而 secondary Tool 支持
- **THEN** primary Cross Agent Team SHALL 隐藏
- **AND** secondary Cross Agent Team SHALL 显示

### Requirement: Worktree configuration is pane scoped

每个 pane SHALL 有自己的 Worktree field 和 `Ctrl+P` Worktree overlay。  Branch mode、extra repositories 和 reuse confirmation SHALL 只作用于打开该 overlay 的 pane。

#### Scenario: Primary overlay 不改变 secondary
- **WHEN** 用户编辑 primary Worktree sub-options
- **THEN** secondary Worktree 与 sub-options SHALL 不变

#### Scenario: Secondary overlay 不改变 primary
- **WHEN** 用户编辑 secondary Worktree sub-options
- **THEN** primary Worktree 与 sub-options SHALL 不变

## REMOVED Requirements

### Requirement: YOLO field visibility considers both pane tools

**Reason**: 一个共享 YOLO field 无法表达 pane-level independent config, 并会继续耦合 primary 与 secondary launch behavior。

**Migration**: New Session 为每个支持 opt-in YOLO 的 pane 显示独立 YOLO Mode。  旧 session 的共享值在 migration 时复制到已有 managed pane, 保持旧 session 的启动语义。
