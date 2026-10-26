## ADDED Requirements

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
