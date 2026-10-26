# 只读 agent 总览

## Purpose

为管理 agent 提供不读取终端内容、不变更所有权的项目与会话盘点, 并以明确的来源和观测时间展示 pane、xats 身份及 TUI 所属 profile.

## Requirements

### Requirement: 单次只读层级总览

系统 SHALL 提供 `aoe overview [--all-profiles]`, 默认盘点解析到的 profile, 显式 all-profiles 盘点全部已存在 profile.  返回 JSON envelope 与 profile、group、session 完整 ID/title/path、pane 持久 ID、主从关系、tool 和 live 观测.  无持久 ID 的人工 pane MUST 返回 null, 不通过查询补写.

#### Scenario: 盘点人工和不同 owner 的资源
- **WHEN** 无 manager context 调用总览, profile 含人工和多个 owner 的 session/pane
- **THEN** 返回全部资源的只读 metadata, 不要求转移 owner, 不读取终端内容, 不修改已有控制权限

#### Scenario: 多 profile 与空 profile
- **WHEN** 调用 all-profiles, 其中某些 profile 无 session 或包含空 group
- **THEN** 一次返回各 profile 和 group, 保留空层级与停止 session

### Requirement: 状态来源与观测时效

系统 MUST 分离 stored session 状态、tmux pane 存在性、xats 身份验证和 runtime readiness, 返回来源与观测时间.  无法观测与已确认不存在 MUST 可区分.

#### Scenario: stored running 但 pane 已不存在
- **WHEN** stored 状态为 running 且实时 tmux 观测证明 pane 不存在
- **THEN** 保留 stored 状态来源, live 为 false, 不声称正在推进或 runtime ready

#### Scenario: 依赖局部失败
- **WHEN** 某 profile 数据损坏、tmux 失败或 xats 不可达
- **THEN** 成功取得的数据仍返回, partial 为 true 并有具体错误及对应来源, 未知数据不得伪装为空或 false

#### Scenario: 已存在的数据文件被截断
- **WHEN** sessions.json 或 groups.json 已存在但为空或纯空白
- **THEN** 对应字段为 null 且 partial 为 true, 附具体解析错误, 不将截断数据宣称为空列表

### Requirement: xats 精确绑定与输出脱敏

系统 SHALL 仅在 xats 只读查询验证当前精确 pane 绑定后展示 verified name/team.  验证 MUST 使用 socket/server 代次/pane 证据, 不依赖 title、cwd、裸 pane 编号或单纯 PID 存活.  输出 MUST 不含 token、identity key、fence、完整 argv、环境和聊天内容.

#### Scenario: 验证通过
- **WHEN** xats 对当前 pane 的精确绑定返回 verified
- **THEN** 展示 agent name/team 及独立 runtime readiness 和观测时间

#### Scenario: 身份未知或陈旧
- **WHEN** 无绑定、历史证据不足、重复候选、pane 被替换或服务不支持查询
- **THEN** 明确返回 unknown/unverifiable/stale/ambiguous/unavailable 等状态, 无已验证身份, 不触发注册或恢复

### Requirement: TUI 实例 profile 查询

系统 SHALL 提供 `aoe profile instances` 返回全部 profile 的 TUI 实例登记及验证状态.  新 TUI 登记 MUST 包含可校验的进程启动时间, 避免 PID 复用误认.  查询 MUST 不删除旧记录.

#### Scenario: 同时运行多个 TUI
- **WHEN** 多个 TUI 在不同或相同 profile 中运行
- **THEN** 列出每个 PID/profile 和启动证据, 不声称唯一当前 TUI

#### Scenario: 旧登记与 PID 复用
- **WHEN** 登记为空、损坏、进程已退出或启动时间不匹配
- **THEN** 分别保守标注 unverified/invalid/stale, 不把无关活进程认作已验证 TUI

#### Scenario: TUI 内切换 profile 和退出
- **WHEN** TUI 成功从一个 profile 切换到另一个 profile, 随后正常退出
- **THEN** 查询仅将该 PID 验证为新 profile, 旧 profile 登记移除; 退出移除当前登记, 不删除其它 PID 的登记

#### Scenario: profile 切换登记失败
- **WHEN** 新 profile 登记无法写入
- **THEN** 不完成 HomeView profile 切换, 保留或恢复原 profile 登记并报告具体错误, 不将新 profile 伪报为当前实例

### Requirement: 无控制副作用

总览和实例查询 MUST 不创建或修改 session 数据、owner、pane marker、数据库 schema 或 xats 绑定; 不调用 capture/send/prepare/bind/release 或清理.

#### Scenario: 查询前后状态一致
- **WHEN** 连续执行查询, 包含缺失 profile、损坏登记与依赖失败
- **THEN** 数据文件、owner 和运行中进程保持不变, 仅进行明确的只读观测

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
