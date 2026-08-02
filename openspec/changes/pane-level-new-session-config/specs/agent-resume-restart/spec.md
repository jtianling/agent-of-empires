## MODIFIED Requirements

### Requirement: Slot-based multi-pane resume preserves full launch context

`R` restart、fresh restart 和 cold recovery SHALL 通过统一 pane command builder 重建每个 tracked slot。  builder SHALL 显式接收目标 slot 的 pane config 和 `native_session_id`, 并 SHALL 使用该 pane 自己的 Tool、cwd、YOLO Mode、Cross Agent Team、Worktree metadata 和 identity key。

session-level hooks、Sandbox wrapping 和 Cross Agent Team channel 等仍属 session 的上下文 SHALL 与 pane config 合并。  pane-specific Tool override、YOLO treatment、Cross Agent Team decoration 或 Worktree cwd MUST NOT 从 sibling pane 或旧 instance-level flag 推断。

没有可用 `native_session_id` 或 Tool 不支持 resume 的 pane SHALL fresh launch, 但仍 SHALL 保留自己的完整 pane config。  unknown agent 和 invalid resume token SHALL 继续通过现有输入验证拒绝或降级, 不得插入 shell command。

#### Scenario: YOLO CliFlag 只用于开启的 pane
- **WHEN**一个 tracked pane 开启 YOLO Mode 且 Tool 使用 CliFlag
- **AND**该 pane 使用 `R` restart
- **THEN**该 pane command SHALL 包含自己的 YOLO flag
- **AND**未开启 YOLO 的 sibling SHALL 不包含该 flag

#### Scenario: YOLO EnvVar 只用于目标 pane
- **WHEN**一个 tracked pane 开启 YOLO Mode 且 Tool 使用 EnvVar
- **THEN**该 pane SHALL 使用对应 env var 启动
- **AND**该 env var SHALL 不因 sibling 的值而添加或删除

#### Scenario: Hook-config agent 保留 instance id
- **WHEN**一个 tracked pane 的 Tool 需要 hook config
- **THEN**该 pane SHALL 继续获得 `AOE_INSTANCE_ID`

#### Scenario: Existing sandbox session 保留 container wrapping
- **WHEN**一个已有 sandbox session 的 tracked pane restart
- **THEN**该 pane SHALL 继续通过 session container exec wrapper 启动
- **AND** pane-level config SHALL 不关闭已有 Sandbox wrapping

#### Scenario: Non-YOLO pane 不获得 sibling flag
- **WHEN**一个 pane 关闭 YOLO Mode, 另一个 pane 开启
- **AND** session restart
- **THEN**关闭 YOLO 的 pane SHALL 不获得任何 YOLO flag 或 env var

#### Scenario: Heterogeneous panes 使用各自 Tool 语义
- **WHEN**一个 session 的 tracked slots 记录不同 Tool 和不同 YOLO Mode
- **THEN**每个 pane SHALL 按自己的 Tool `YoloMode` variant 和 enabled 值构建

#### Scenario: Degraded fresh pane 保留自己的 launch context
- **WHEN**一个 tracked slot 没有可用 resume token
- **THEN**该 pane SHALL fresh launch
- **AND** SHALL 保留自己的 cwd、YOLO Mode、Cross Agent Team、Worktree 和 identity key

#### Scenario: Cross Agent Team 只对开启的 pane 重放
- **WHEN**一个 session 中只有部分 pane 开启 Cross Agent Team
- **AND** session restart 或 cold recovery
- **THEN**只有开启的 pane SHALL 使用 tool-specific Cross Agent Team launch path
- **AND**其他 pane SHALL 普通启动

#### Scenario: Worktree cwd 按 slot 恢复
- **WHEN** primary 与 secondary slot 记录不同 Worktree cwd
- **AND** session restart 或 cold recovery
- **THEN**每个 pane SHALL 在自己 slot 的 cwd 中启动

#### Scenario: Command injection validation preserved
- **WHEN** slot 记录 unknown unsafe agent 或 invalid native session id
- **THEN**系统 SHALL 使用现有验证拒绝或降级 fresh launch
- **AND**不得把未验证值插入 shell command

#### Scenario: Invalid tracked slot is visible during restart
- **WHEN** restart 或 cold recovery 读取到一个结构性无效 slot 和至少一个有效 sibling slot
- **THEN**系统 SHALL 继续重启或恢复有效 sibling pane
- **AND** session error SHALL 显示 skipped pane count
