## Context

现有 managed list 仅返回 stored 状态, pane list 逐 session 调用且 xats 信息受 owner 限制.  TUI 已有每 profile 的 .instances/PID 登记, 但空文件不能防 PID 复用.  宿主运行实时 session, 全部运行时测试使用一次性隔离容器.  

## Goals / Non-Goals

目标: 无 manager context 的只读盘点, 一次调用返回所选 profile 或全部 profile 的层级和观测证据; 查询多个 TUI 实例及其 profile.  

不包含终端内容、自动认领、owner 转移、控制权限扩张或生产 xats 部署.  不把 live 当作工作进展, 不强制旧人工 pane 补登记.  

## Decisions

- 新入口 `aoe overview [--all-profiles]`, 默认使用现有 profile 解析; `aoe profile instances` 列出全部 profile 的 TUI 登记.  均固定输出 JSON envelope, 无需 manager context.  
- 入口在 migrations/reconcile 之前返回.  提供只读 Storage 读取和 SQLite 只读打开, 不创建目录/锁/schema/备份或清理陈旧记录.  读取不同资源并非原子快照, 返回观测时间与局部错误.  
- 每 profile 显式指定 tmux socket, 清 TMUX/TMUX_PANE, 批量观测所有 pane.  展示完整 session ID、持久 pane ID(无记录则 null)、主从关系、tool、布局位置与 live; 标记替换或关联证据不足不得伪装 live managed pane.  停止项保留, unknown 与 absent 分开.  
- 状态携带 source 和 observed_at.  stored running/idle 不宣称实时; tmux live 不宣称 runtime ready.  xats 查询仅使用本地控制面, 按精确 socket/server 代次/pane 验证, 输出严格白名单.  缺接口、超时、无有效绑定、历史证据不足均显式降级.  
- TUI 登记保存版本、PID、进程启动时间与 profile.  查询通过系统进程信息比较启动时间, 多 TUI 逐个返回.  旧空登记、损坏记录及死进程分别标注 unverified/invalid/stale, 不删除.  不从共享 app-server 环境声称用户当前唯一 TUI.  
- 文档说明总览的只读权限与控制权限相互独立, 以及旧 TUI 重开后才能有完整实例证据.  

## Risks / Trade-offs

- xats 旧版未提供精确查询 -> 保留盘点, 身份返回 unavailable/unknown, 不按 title 推断.  
- 查询期间 pane 退出/迁移或 TUI 重启 -> 精确证据复核, 冲突保守降级; 时间戳表达非原子观测.  
- profile 损坏或 socket 连接失败 -> 返回局部错误, 保留其它 profile 结果并设置 partial.  
- 历史数据无持久 pane ID -> 返回 null, 查询不补写.  
