## MODIFIED Requirements

### Requirement: Agent-specific fields hidden for shell

系统 SHALL 隐藏 shell pane 不适用的 agent 专属字段: YOLO Mode 和 Cross Agent Team。  Path 和 Worktree SHALL 保持可用, 因为它们配置 pane working directory, 而不是 agent 权限。

#### Scenario: YOLO mode hidden for shell
- **WHEN** 用户在 New Session 中为一个 pane 选择 `shell`
- **THEN** 该 pane 的 YOLO Mode SHALL 不显示

#### Scenario: Cross Agent Team hidden for shell
- **WHEN** 用户在 New Session 中为一个 pane 选择 `shell`
- **THEN** 该 pane 的 Cross Agent Team SHALL 不显示

#### Scenario: Path and Worktree remain available for shell
- **WHEN** 用户为 primary 或 secondary pane 选择 `shell`
- **THEN** 该 pane 的 Path SHALL 保持显示
- **AND** 该 pane 的 Worktree SHALL 保持显示
