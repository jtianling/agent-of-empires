## MODIFIED Requirements

### Requirement: Worktree reuse on session creation

系统 SHALL 允许 primary 或 secondary pane 独立复用已存在的 Worktree。  每个复用的 Worktree MUST 在所属 pane 的配置中记录 `managed_by_aoe: false` 和 `cleanup_on_delete: false`。

当一个或多个 pane 的计算路径已存在时, New Session 第一次提交 SHALL 显示一次警告, 明确列出 pane 与路径, 并且不创建 session。  再次提交 SHALL 只复用用户确认的路径。

#### Scenario: Primary Worktree 已存在
- **WHEN** primary Worktree 的计算路径已存在
- **AND**用户第一次提交 New Session
- **THEN**警告 SHALL 标明 primary pane 和已存在路径
- **AND** session SHALL 不被创建

#### Scenario: Secondary Worktree 已存在
- **WHEN** secondary Worktree 的计算路径已存在
- **AND**用户第一次提交 New Session
- **THEN**警告 SHALL 标明 secondary pane 和已存在路径
- **AND** primary Worktree 状态 SHALL 不被改写

#### Scenario: 两个 pane 的冲突合并确认
- **WHEN** primary 与 secondary Worktree 的计算路径都已存在
- **THEN**一次警告 SHALL 同时列出两个 pane 的路径
- **AND**再次提交 SHALL 复用两个已确认路径

#### Scenario: 复用 Worktree 不被清理
- **WHEN** session 删除时某个 pane 使用 `managed_by_aoe: false` 的 Worktree
- **THEN**该 Worktree 目录和 branch SHALL 不被 AoE 删除

#### Scenario: CLI reuse 保持原行为
- **WHEN**用户通过现有 CLI Worktree flags 创建 session
- **THEN**现有 `--reuse-worktree` 行为 SHALL 保持不变

#### Scenario: CLI 未确认复用时显式报错
- **WHEN** CLI 计算出的 Worktree 已存在
- **AND**用户没有传递现有 `--reuse-worktree` flag
- **THEN**创建 SHALL 失败并提示用户显式确认复用

### Requirement: Worktree creation lifecycle

AoE SHALL 为每个请求 Worktree 的 pane 独立执行以下生命周期:

1. 从该 pane 的有效 Path 解析 main repository。
2. 从 template 计算该 pane 的目标路径。
3. 执行 `git worktree add`, 并按该 pane 的配置决定是否创建新 branch。
4. 转换 `.git` file 为相对路径。
5. 同步该 pane 请求的 ignored agent directories 和 extra repositories。
6. 在该 pane 的配置与 durable slot 中记录 `WorktreeInfo`。
7. 在 pane metadata 中单独记录不可变 `worktree_path`, 作为唯一 cleanup target。
8. 把该 pane 的 working directory 设置为 Worktree 路径。

一个 pane 的创建 SHALL NOT 改写另一个 pane 的 Path、branch、repository list 或 `WorktreeInfo`。

#### Scenario: 两个 pane 创建不同 Worktree
- **WHEN** primary 与 secondary pane 配置不同 branch
- **THEN** AoE SHALL 分别创建和记录两个 Worktree
- **AND**每个 pane SHALL 启动在自己的 Worktree 路径

#### Scenario: Secondary 使用自己的 Path 作为 repository base
- **WHEN** secondary pane 明确设置 Path 并请求 Worktree
- **THEN** Worktree SHALL 从 secondary Path 对应的 main repository 创建
- **AND** primary Path SHALL 不参与该解析

#### Scenario: Secondary 空 Path 使用 primary 最终路径
- **WHEN** secondary Path 为空但 secondary Worktree 非空
- **THEN** AoE SHALL 从 primary pane 的最终路径寻找 main repository
- **AND** secondary 的最终 working directory SHALL 是新 Worktree 路径

#### Scenario: 创建包含 agent dir sync
- **WHEN**一个 pane 的源 repository 包含 ignored agent directories
- **THEN**该 pane 的 Worktree SHALL 在 `git worktree add` 后执行同步
- **AND**同步 SHALL 在记录 `WorktreeInfo` 前完成

#### Scenario: Workspace 目标目录已存在时拒绝创建
- **WHEN** multi-repo workspace 的计算目标目录已存在
- **THEN**创建 SHALL 显式失败
- **AND**该既有目录及其内容 SHALL 保持不变

### Requirement: Worktree deletion lifecycle

删除 session 时, AoE SHALL 遍历该 session 的全部 managed pane Worktree。  对每个 `managed_by_aoe=true` 且 `cleanup_on_delete=true` 的不可变 `worktree_path`, 系统 SHALL 按以下顺序执行:

1. 清理该 Worktree 的 ignored agent directories。
2. 执行 `git worktree remove <exact-path>`。
3. 当 `delete_branch_on_cleanup=true` 时, 删除该 Worktree 记录的精确 branch。

清理 SHALL 不使用 session name、branch prefix 或目录 pattern 扫描其他 Worktree。
清理 SHALL NOT 使用 runtime capture 更新后的 pane `cwd` 推断 Worktree 路径。

#### Scenario: 删除 session 清理两个 managed Worktree
- **WHEN** primary 与 secondary pane 各自拥有可清理的 managed Worktree
- **THEN** AoE SHALL 分别清理两个精确 Worktree 路径
- **AND**每个 branch SHALL 只按自己的 metadata 决定是否删除

#### Scenario: 删除 Worktree 前先清理 agent directories
- **WHEN**一个可清理的 pane Worktree 包含 ignored agent directories
- **THEN** AoE SHALL 在删除该精确 Worktree 前执行现有 agent directory cleanup

#### Scenario: Reused secondary Worktree 保留
- **WHEN** primary Worktree 由 AoE 管理但 secondary Worktree 为复用
- **AND** session 被删除
- **THEN** primary Worktree SHALL 按配置清理
- **AND** secondary Worktree SHALL 保留

#### Scenario: Partial creation failure 只回滚本次所有权
- **WHEN** secondary Worktree 创建成功但 pane split 或 slot 持久化失败
- **THEN** AoE SHALL 清理本次新建且尚未转交 durable state 的 secondary Worktree
- **AND**任何复用 Worktree SHALL 不被删除

#### Scenario: Runtime cwd 漂移不改变 cleanup target
- **WHEN** capture 把 pane `cwd` 更新为 Worktree 之外的目录
- **AND**该 pane 仍持有 managed Worktree metadata
- **THEN**删除 SHALL 只使用 metadata 中记录的不可变 `worktree_path`
- **AND** capture 目录 SHALL 不被删除

#### Scenario: Fork 保留共享 branch
- **WHEN** fork 继承 `managed_by_aoe=true` 但 `cleanup_on_delete=false` 的 Worktree
- **AND**用户删除 fork 并请求删除 branch
- **THEN**共享 Worktree 和 branch SHALL 保留
