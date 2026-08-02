## ADDED Requirements

### Requirement: New Session 按 session 与 pane 分区展示

New Session 对话框 SHALL 先展示 session 元数据, 再展示 primary pane 配置, 然后通过可见分割线展示可选 secondary pane 配置。  字段顺序 MUST 为 Title、Group、primary Tool、primary Path、primary YOLO Mode 与 Cross Agent Team、primary Worktree、Right Pane Agent, 以及选择 right pane Agent 后出现的 secondary Path、secondary YOLO Mode 与 Cross Agent Team、secondary Worktree。

对话框 SHALL NOT 展示 Sandbox 字段或 Sandbox 配置入口。

#### Scenario: 默认布局只显示 primary pane 和 right pane selector
- **WHEN** 用户打开 New Session
- **THEN** Title 与 Group SHALL 位于 pane 配置之前
- **AND** primary pane 字段 SHALL 按规定顺序显示
- **AND** primary pane 与 Right Pane Agent 之间 SHALL 有可见分割线
- **AND** Right Pane Agent 默认为 `none`
- **AND** secondary pane 配置 SHALL 被折叠

#### Scenario: 选择 right pane Agent 展开完整配置
- **WHEN** 用户把 Right Pane Agent 从 `none` 改为一个 Tool
- **THEN** secondary Path、适用的 YOLO Mode 与 Cross Agent Team、secondary Worktree SHALL 显示在 selector 下方
- **AND** primary pane 字段 SHALL 保持原值和原顺序

#### Scenario: New Session 没有 Sandbox 入口
- **WHEN** 用户浏览 New Session 的全部可见字段和配置 overlay
- **THEN** Sandbox checkbox SHALL 不存在
- **AND** Sandbox 配置 overlay SHALL 无法从该对话框打开
