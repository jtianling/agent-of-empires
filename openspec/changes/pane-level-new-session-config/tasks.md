## 1. Pane 数据模型与迁移

- [x] 1.1 定义统一的 `PaneDraft`、`PaneConfig` 和 pane Worktree 请求类型, 并让 primary 与 secondary 使用同一套验证接口
- [x] 1.2 将 primary Tool、Path、YOLO Mode、Cross Agent Team 和 Worktree 的权威状态迁移到 pane 配置, 保留明确的 session 级边界
- [x] 1.3 扩展 `agent_slot` schema、读写 API 和 serde 边界验证, 持久化 pane flags 与 Worktree metadata
- [x] 1.4 添加幂等旧数据迁移, 将旧 session 共享 flags 和 primary identity/Worktree 状态转换为 slot 级配置
- [x] 1.5 为 schema 补列、重复迁移、旧值继承和 invalid persisted metadata 添加 focused tests

## 2. Pane 级启动与恢复

- [x] 2.1 重构 pane command builder, 显式接收目标 `PaneConfig`, 不再从 instance 共享 YOLO 或 Cross Agent Team enabled 值推断
- [x] 2.2 将 Claude development-channel、auto-confirm、Codex xats bootstrap 和 YOLO treatment 改为只读取目标 pane 配置
- [x] 2.3 将 primary 和 secondary identity key 统一持久化到各自 durable slot, 并保持 sibling key 独立
- [x] 2.4 改造首次启动、`R` restart、fresh restart 和 cold recovery, 逐 slot 使用自己的 pane 配置和 working directory
- [x] 2.5 添加 left-only、right-only、双开与双关 YOLO/Cross Agent Team 的 command builder 和 restart tests

## 3. Pane 级 Worktree 生命周期

- [x] 3.1 抽取可复用的 pane Worktree 解析流程, 支持显式 pane Path 和 secondary 空 Path fallback
- [x] 3.2 为 primary 与 secondary 分别保存 branch mode、extra repositories、reuse confirmation 和最终 `WorktreeInfo`
- [x] 3.3 在 secondary split 前创建或复用 Worktree, 并在 split/slot 写入失败时只回滚本次新建的精确路径
- [x] 3.4 改造 session 删除流程, 遍历并清理每个 pane 自己拥有的 managed Worktree 和 branch
- [x] 3.5 添加双 Worktree、混合 managed/reused、同 branch 冲突和 partial failure rollback tests

## 4. New Session 对话框

- [x] 4.1 将对话框状态重构为 session metadata、primary `PaneDraft` 和可选 secondary `PaneDraft`, 删除零散 `right_pane_*` 状态
- [x] 4.2 按 Title、Group、primary Tool、Path、flags、Worktree、分割线、Right Pane Agent、secondary 配置的顺序重写 field layout 和 render
- [x] 4.3 让 YOLO Mode 与 Cross Agent Team 的显示和输入路由只依赖同一 pane 的 Tool
- [x] 4.4 为两个 pane 提供独立 Path picker、ghost completion 和 Worktree `Ctrl+P` overlay, 并保留 collapsed secondary draft
- [x] 4.5 从 New Session 删除 Sandbox 字段、help、overlay 和 hidden state, 让 TUI 提交路径显式创建非 sandbox session
- [x] 4.6 更新提交验证、目录确认和 reuse warning, 一次准确报告两个 pane 的问题且不交叉修改状态
- [x] 4.7 添加动态 field index、collapsed/expanded layout、shell visibility、independent flags 和无 Sandbox 入口的单元测试

## 5. Session 创建集成

- [x] 5.1 用 `Option<PaneDraft>` 替代 `PendingRightPane` 的 tool/path 组合, 并在创建成功后传递 resolved secondary `PaneConfig`
- [x] 5.2 改造 right split、command build 和 launch-time slot record, 原子记录 secondary pane 的完整配置
- [x] 5.3 保持 `@aoe_agent_pane` 指向 primary pane, 并保持 shell right pane、remain-on-exit 和用户 shell wrapper 行为
- [x] 5.4 验证 CLI Sandbox、Settings Sandbox、已有 sandbox session 和 add-pane container wrapping 未受 New Session 入口移除影响

## 6. 验证与文档一致性

- [x] 6.1 更新受影响的主规格说明和相关测试 fixture, 确保代码与 delta specs 一致
- [x] 6.2 运行 `cargo fmt --check`、`cargo clippy` 和 `cargo check`, 不在有实时 session 的环境运行全量测试或 ad-hoc tmux 探针
- [x] 6.3 添加使用私有 tmux socket 且显式清除 `TMUX`/`TMUX_PANE` 的 E2E coverage, 覆盖 primary/secondary 独立 Path、Worktree、YOLO 和 Cross Agent Team
- [x] 6.4 让 E2E cleanup 只定位本测试创建的精确 session、Worktree 和私有 socket, 并输出可审查的验收证据

## 7. Reviewer 修复

- [x] 7.1 将 Worktree cleanup target 固化到 pane metadata, 禁止使用 capture 可变 `cwd` 推断删除路径
- [x] 7.2 让 invalid slot/capture 只隔离自身, 并按实际 Tool 归一化 legacy 与 adopted pane flags
- [x] 7.3 恢复无 `kill-server` 的私有 socket stale E2E reaper, 并补齐未声明的 Worktree 安全行为规格
- [x] 7.4 补回归测试并重新运行 fmt、check、clippy、OpenSpec strict validate 与 diff 检查

## 8. Reviewer 最终修复

- [x] 8.1 让 slot 0 migration 和 legacy primary hydration 按实际 pane Tool 统一归一化 YOLO/Cross Agent Team
- [x] 8.2 让 store read 自愈 capability flag 不一致并持久化修复, 结构性坏 row 继续隔离
- [x] 8.3 向 restart 与 cold recovery 暴露 skipped slot 诊断, 避免 pane 静默消失
- [x] 8.4 补 literal、migration、read-repair 与 diagnostics 回归覆盖并重新运行静态门禁
- [x] 8.5 将 legacy sync 归一化结果回写镜像字段, 避免 CLI 摘要与 authoritative primary pane 不一致

## 9. Tester 修复

- [x] 9.1 让 Codex xats bootstrap 的 `-C` 显式使用目标 pane working directory, 并补 primary/secondary 不同路径的命令回归断言
