## Why

人工管理大量 session 的成本过高, 需要本地管理 agent 协助创建和回收工作 agent.  现有 CLI 没有所有权边界, 无法防止管理 agent 误操作人工或其他管理者的 session.

## What Changes

- 增加受限的 `aoe managed` CLI, 使用稳定管理身份和结构化 JSON 返回.
- 创建时自动记录 owner, 所有变更及终端内容读取仅允许操作自己创建的 session, 完整 ID 精确定位.
- 人工 TUI 和现有人工 CLI 保留管理及接管能力, 旧 session 默认人工所有.
- 支持创建时指定 xats name/team, 精确绑定及恢复身份, 区分进程运行与注册就绪.
- 增加并发保存保护, 防止人工 TUI 和 CLI 相互覆盖 session.
- 删除 session 默认保留工作目录、分支等代码资源, 不实现远程 API 或 MCP.

## Capabilities

### New Capabilities

- `managed-session-cli`: 受限 CLI、稳定所有权、并发安全与 xats 创建闭环.

### Modified Capabilities

无.  现有人工入口权限保持不变, 新增能力的约束由独立规格描述.

## Impact

涉及 CLI 分派、session 持久化与生命周期、现有 xats 本地控制面及用户文档.  同一系统用户可通过普通 shell 绕过受限入口, 本次提供接口防误操作而非操作系统强隔离.  生产机器不执行全量测试或临时 tmux 探针, 使用编译检查及不接触真实 session 的验证.
