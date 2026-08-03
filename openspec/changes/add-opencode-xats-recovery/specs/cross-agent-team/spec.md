## MODIFIED Requirements

### Requirement: Cross Agent Team launch option in New Session

New Session SHALL 为 primary pane 和已选择的 secondary pane 分别展示 Cross Agent Team checkbox。  checkbox SHALL 与同一 pane 的 YOLO Mode 位于同一行, 两者 MUST 可以独立切换。

每个 checkbox 的可见性 SHALL 只取决于同一 pane 的 Tool。  Tool 为 `claude`、`codex` 或 `opencode` 时显示, 其他 Tool 时隐藏。  primary 与 secondary 的值 SHALL 相互独立, 初始值分别取自 active profile 解析后的 `cross_agent_team_default`。

#### Scenario: Primary Claude 显示独立开关
- **WHEN** primary Tool 为 `claude`
- **THEN** primary Cross Agent Team checkbox SHALL 显示在 primary YOLO Mode 右侧
- **AND**切换它 SHALL 不改变 secondary pane

#### Scenario: Secondary Codex 显示独立开关
- **WHEN** Right Pane Agent 为 `codex`
- **THEN** secondary Cross Agent Team checkbox SHALL 显示在 secondary YOLO Mode 右侧
- **AND**切换它 SHALL 不改变 primary pane

#### Scenario: OpenCode 显示独立开关
- **WHEN**任一 pane 的 Tool 为 `opencode`
- **THEN**该 pane SHALL 显示自己的 Cross Agent Team checkbox
- **AND**切换它 SHALL 不改变 sibling pane

#### Scenario: 不支持的 Tool 只隐藏自己的开关
- **WHEN**一个 pane 的 Tool 不是 `claude`、`codex` 或 `opencode`
- **THEN**该 pane 的 Cross Agent Team checkbox SHALL 不显示
- **AND**另一个 pane 的可见性 SHALL 不受影响

#### Scenario: 两个 pane 独立应用默认值
- **WHEN** `cross_agent_team_default` 为 true
- **AND** primary 与 secondary 都使用支持的 Tool
- **THEN**两个 pane 的 checkbox SHALL 分别初始化为选中
- **AND**用户 SHALL 可以只关闭其中一个

## ADDED Requirements

### Requirement: OpenCode xats launch is pane scoped

Each non-sandboxed OpenCode pane with Cross Agent Team enabled SHALL launch with its own durable identity key, runtime generation, loopback endpoint and exact native session.  A sibling pane's identity or runtime values MUST NOT be reused.

#### Scenario: Two OpenCode panes have independent xats state
- **WHEN** primary and secondary OpenCode panes both enable Cross Agent Team
- **THEN** their identity keys SHALL differ
- **AND** their runtime generations SHALL be advanced independently
- **AND** their base URLs and native session ids SHALL identify different runtimes

#### Scenario: Disabled sibling remains ordinary OpenCode
- **WHEN** only one OpenCode pane enables Cross Agent Team
- **THEN** only that pane SHALL execute xats reserve and commit
- **AND** the disabled sibling SHALL still use accurate native session capture without xats control-plane calls
