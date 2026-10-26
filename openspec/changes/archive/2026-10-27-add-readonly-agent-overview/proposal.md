## Why

管家 agent 当前必须逐 session 查询 pane, 且无法确认人工 pane 的 xats 身份或正在运行的 TUI 所属 profile.  需要一次只读盘点, 避免为了了解项目分布而转移 owner 或读取终端内容.  

## What Changes

- 增加 JSON 只读总览命令, 汇总 profile、group、session、pane 及有来源和观测时间的状态.  
- 精确绑定验证通过才展示 xats name/team 与 readiness, 无法验证时明确 unknown/unavailable.  
- 增加运行中 TUI 实例与 profile 查询, 对陈旧记录和旧版证据不足记录保守标记.  
- 查询不变更 session、owner、pane marker、数据库 schema 或 xats 注册状态.  

## Capabilities

### New Capabilities

- `readonly-agent-overview`: agent 可用的只读层级总览与 TUI 实例发现.  

### Modified Capabilities

无.  

## Impact

CLI、只读存储访问、tmux 批量观测、TUI 实例登记、xats 本机只读查询、文档和隔离 E2E.  现有控制权限保持不变.  
