## ADDED Requirements

### Requirement: 总览重复错误聚合

系统 SHALL 在每个 profile.errors 内按相同 source 和完整 message 聚合重复错误, 每条返回 affected_count, 表示受影响的观测条数, observed_at 保留首次观测时间.  不同 profile、来源或原因 MUST 保持分离.  pane 自身原因、partial 与 unknown 语义 MUST 保持不变.  

#### Scenario: 多个 pane 遇到相同错误
- **WHEN** 同一 profile 中 46 个 pane 的 lookup 返回同一错误
- **THEN** profile.errors 只有该原因一条, affected_count 为 46, 每个 pane 仍有自身 unavailable 原因和 unknown runtime

#### Scenario: 混合错误与跨 profile
- **WHEN** 多个 profile 存在不同来源或不同原因的错误
- **THEN** 各 profile 分别聚合, 不同来源或原因保持独立计数

### Requirement: 旧 lookup 端点诊断

系统 SHALL 将 lookup HTTP 404 明确报告为端点不受支持, 不依赖旧服务错误响应体格式, 不误报 invalid error/invalid JSON.  其它状态 MUST 保留具体错误诊断, 查询仍为只读.  

#### Scenario: 旧服务的不同响应体
- **WHEN** lookup 返回 HTTP 404 和 JSON、纯文本或 HTML 响应体
- **THEN** 错误明确说明 lookup 端点不受支持, partial 为 true 且身份 unavailable, 不触发升级或控制操作
