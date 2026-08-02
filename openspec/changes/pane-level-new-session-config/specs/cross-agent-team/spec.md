## MODIFIED Requirements

### Requirement: Cross Agent Team launch option in New Session

New Session SHALL 为 primary pane 和已选择的 secondary pane 分别展示 Cross Agent Team checkbox。  checkbox SHALL 与同一 pane 的 YOLO Mode 位于同一行, 两者 MUST 可以独立切换。

每个 checkbox 的可见性 SHALL 只取决于同一 pane 的 Tool。  Tool 为 `claude` 或 `codex` 时显示, 其他 Tool 时隐藏。  primary 与 secondary 的值 SHALL 相互独立, 初始值分别取自 active profile 解析后的 `cross_agent_team_default`。

#### Scenario: Primary Claude 显示独立开关
- **WHEN** primary Tool 为 `claude`
- **THEN** primary Cross Agent Team checkbox SHALL 显示在 primary YOLO Mode 右侧
- **AND**切换它 SHALL 不改变 secondary pane

#### Scenario: Secondary Codex 显示独立开关
- **WHEN** Right Pane Agent 为 `codex`
- **THEN** secondary Cross Agent Team checkbox SHALL 显示在 secondary YOLO Mode 右侧
- **AND**切换它 SHALL 不改变 primary pane

#### Scenario: 不支持的 Tool 只隐藏自己的开关
- **WHEN**一个 pane 的 Tool 不是 `claude` 或 `codex`
- **THEN**该 pane 的 Cross Agent Team checkbox SHALL 不显示
- **AND**另一个 pane 的可见性 SHALL 不受影响

#### Scenario: 两个 pane 独立应用默认值
- **WHEN** `cross_agent_team_default` 为 true
- **AND** primary 与 secondary 都使用支持的 Tool
- **THEN**两个 pane 的 checkbox SHALL 分别初始化为选中
- **AND**用户 SHALL 可以只关闭其中一个

### Requirement: Cross Agent Team preserved across restart

Cross Agent Team setting SHALL 按 pane 持久化。  `R` restart、fresh restart 和 cold recovery SHALL 从目标 pane 的 durable config 重建 tool-specific launch command。  Claude pane SHALL 根据自己的值决定 development-channel flag 和 auto-confirm, Codex pane SHALL 根据自己的值决定 pane pre-registration 和 remote app-server bootstrap。

#### Scenario: Claude pane restart 重放自己的配置
- **WHEN**一个开启 Cross Agent Team 的 Claude pane 经 `R` restart
- **THEN**该 pane 的新命令 SHALL 包含 development-channel flag
- **AND** AoE SHALL 对该 pane 再次执行 startup auto-confirm

#### Scenario: 未开启的 sibling 不被装饰
- **WHEN**一个 session 中只有一个 pane 开启 Cross Agent Team
- **AND** session 被 restart 或 recovery
- **THEN**只有该 pane SHALL 使用 Cross Agent Team launch path
- **AND** sibling pane SHALL 使用普通 launch path

#### Scenario: Codex pane resume 保留 token 和独立开关
- **WHEN**一个开启 Cross Agent Team 的 Codex pane 使用有效 resume token restart
- **THEN**该 pane SHALL 再次执行 xats bootstrap
- **AND** native Codex resume token SHALL 保留
- **AND**其他 pane 的 Cross Agent Team 值 SHALL 不改变

#### Scenario: Codex pane fresh restart 重放 bootstrap
- **WHEN**一个开启 Cross Agent Team 的 Codex pane fresh restart
- **THEN**该 pane SHALL pre-register 并连接配置的 local app-server

### Requirement: Cross Agent Team configuration

AoE SHALL 在 Settings TUI 中继续提供 global 和 profile 两个 scope 的 Cross Agent Team 配置:

- `cross_agent_team_channel`: Claude development-channel string, 默认 `server:cross-agent-teams-channel`。
- `cross_agent_team_default`: New Session 中每个受支持 pane checkbox 的初始值, 默认 false。

profile override SHALL 继续按现有 merge 规则覆盖 global 值。  channel 可以保持 session 级共享, 但是否启用 SHALL 为 pane 级。

#### Scenario: 自定义 channel 只用于已开启的 Claude pane
- **WHEN** `cross_agent_team_channel` 设置为自定义值
- **AND**一个 Claude pane 开启 Cross Agent Team
- **THEN**该 pane 的命令 SHALL 使用自定义 channel
- **AND**未开启的 Claude pane SHALL 不添加 development-channel flag

#### Scenario: profile default 初始化每个 pane
- **WHEN** profile override 设置 `cross_agent_team_default`
- **THEN** New Session 中每个受支持 pane SHALL 使用该默认值独立初始化
- **AND**清除 override SHALL 回退到 global 值

### Requirement: Identity key storage follows the pane's role

每个 managed pane 的 identity key SHALL 跟随该 pane 的 durable slot 保存, 包括 primary pane。  instance record MAY 在 slot 0 建立前保留 primary identity key 作为首次启动 bootstrap 或 migration mirror, 但 SHALL NOT 在 slot 0 已有非空 key 时覆盖它。  slot SHALL 同时保存决定该 key 是否应被注入的 pane-level Cross Agent Team config。

#### Scenario: Primary key 存在 slot 0
- **WHEN** primary pane 开启 Cross Agent Team 并获得 identity key
- **THEN**该 key SHALL 保存到 slot 0
- **AND**关闭并重开 AoE 后 SHALL 仍可读取同一个 key

#### Scenario: Secondary key 存在自己的 slot
- **WHEN** secondary pane 开启 Cross Agent Team 并获得 identity key
- **THEN**该 key SHALL 保存到 secondary pane 的 durable slot
- **AND**它 SHALL 与 primary key 相互独立

#### Scenario: 旧 primary key 迁移到 slot 0
- **WHEN** migration 读取到旧 instance record 上的 primary identity key
- **THEN** migration SHALL 把它写入 slot 0
- **AND**重复运行 migration SHALL 不覆盖 slot 0 已有的非空 key

### Requirement: Extra agent panes AoE launches carry an identity key from their first launch

当 AoE 启动一个 pane-level Cross Agent Team 已开启的额外 agent pane 时, SHALL 在首次启动时创建独立 identity key, 保存到该 pane 的 durable slot, 并以 `XATS_IDENTITY_KEY` 注入进程环境。  该规则覆盖 New Session secondary pane 和 `aoe session add-agent-pane` 的明确 pane 配置。

额外 pane SHALL 不复用任何 sibling pane 的 key。  只有该 pane 自己开启 Cross Agent Team 时才创建 key, session 中其他 pane 的开关 SHALL 不作为判断依据。  shell pane SHALL 永远不创建 key。

#### Scenario: 只有 secondary 开启时仍获得 key
- **WHEN** primary pane 关闭 Cross Agent Team
- **AND** secondary agent pane 开启 Cross Agent Team
- **THEN** secondary pane SHALL 在首次启动时获得并持久化自己的 key
- **AND** primary pane SHALL 不获得 key

#### Scenario: 只有 primary 开启时 secondary 不获得 key
- **WHEN** primary pane 开启 Cross Agent Team
- **AND** secondary agent pane 关闭 Cross Agent Team
- **THEN** secondary pane SHALL 不创建或注入 identity key

#### Scenario: 两个 pane 都开启时 key 不同
- **WHEN** primary 与 secondary agent pane 都开启 Cross Agent Team
- **THEN**两个 pane SHALL 分别获得不同 key

#### Scenario: Restart 复用 extra pane key
- **WHEN**一个额外 pane 已持久化 identity key
- **AND**该 pane restart 或 recovery
- **THEN**该 pane SHALL 复用原 key 而不是重新创建

#### Scenario: Shell extra pane 没有 key
- **WHEN**额外 pane Tool 为 `shell`
- **THEN**该 pane SHALL 不创建 identity key

#### Scenario: key 持久化失败需要显式报告
- **WHEN**额外 pane 已启动但其 identity key 无法持久化
- **THEN**失败 SHALL 显示给用户而不是只写日志
- **AND**系统 SHALL 精确关闭本次启动的 pane
- **AND**只回滚本次新建且尚未转交 durable state 的 Worktree
- **AND**任何复用 Worktree SHALL 保持不变

## ADDED Requirements

### Requirement: Cross Agent Team launch decoration is pane scoped

所有 tool-specific Cross Agent Team 行为 SHALL 使用目标 pane 的配置判断, 不得读取 sibling pane 或 session 级 enabled 值替代。  该行为包括 Claude development-channel 与 auto-confirm、Codex pre-registration 与 remote bootstrap、identity key minting 和 injection。

#### Scenario: Right-only Claude 使用 development channel
- **WHEN** primary pane 关闭 Cross Agent Team
- **AND** secondary Claude pane 开启 Cross Agent Team
- **THEN** secondary Claude command SHALL 包含 development-channel flag
- **AND** primary command SHALL 不包含 Cross Agent Team decoration

#### Scenario: Right-only Codex 使用 xats bootstrap
- **WHEN** primary pane 关闭 Cross Agent Team
- **AND** secondary Codex pane 开启 Cross Agent Team
- **THEN** secondary Codex SHALL 使用 pane-local xats bootstrap
- **AND** primary pane SHALL 保持普通启动

#### Scenario: 新 adopt 的 pane 不继承 primary 开关
- **WHEN** reconciler 首次 adopt 一个没有 durable pane config 的非 primary pane
- **THEN**该 pane 的 Cross Agent Team SHALL 初始化为 false
- **AND** primary pane 的 enabled 值 SHALL 不复制到该 pane
