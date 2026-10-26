## 1. 身份与权限

- [x] 1.1 建立持久 pane ID、owner 和当前 slot/占用关联, 复用现有存储和必要迁移.
- [x] 1.2 实现精确 session/pane/owner/live binding 验证, 人工或未知 pane 默认受保护.
- [x] 1.3 为父 session 操作加入受影响 pane 检查, 同步人工所有权转移及 fork 行为.

## 2. pane CLI

- [x] 2.1 添加 pane list/create/remove/send/capture 参数与结构化 JSON, 拒绝模糊 ID.
- [x] 2.2 复用分屏和 slot 配置实现创建, 验证四 pane 上限与失败回滚.
- [x] 2.3 实现准确 pane 输入和输出, 不随焦点、位置或 slot 重用误投.
- [x] 2.4 实现任意自有 pane 精确删除和可重试清理, 主 pane 安全接任及最后一个 pane 的 session 回收.

## 3. 独立协作与生命周期

- [x] 3.1 泛化现有 managed runtime 上下文, 每 pane 独立 claim、key、token和配置文件.
- [x] 3.2 接入 Codex、Claude、OpenCode 附加 pane 的自动绑定及旧 helper 拒绝.
- [x] 3.3 覆盖父 session 与人工 TUI 的重启、恢复和删除, 不串用邻 pane 身份.

## 4. 验证与说明

- [x] 4.1 添加权限、slot/ID重用、失败回收、配置隔离、主 pane 接任和最后 pane 回收的回归覆盖.
- [x] 4.2 运行 fmt、check/build、clippy及生成 CLI reference, 不操作宿主真实 tmux.
- [x] 4.3 由独立 tester 在断网、无宿主挂载容器验收实际双 pane 操作、Codex/OpenCode xats 身份隔离及原 session 回归; Claude 新增 pane 冷恢复按用户要求暂缓, 不记为通过.
- [x] 4.4 更新用户文档及 jt-aoe-cli 的 pane 使用说明并交接中央维护方.

独立验收: 本轮 Codex/OpenCode 与通用范围 13 组 PASS, 0 FAIL, 0 BLOCKED; Claude 1 组 user-deferred.  两个断网、无宿主挂载的 Linux arm64 容器已精确清理, 仅保留授权公开工具缓存镜像.  最终运行源码逐文件 hash 与工作区一致, 主测试二进制 SHA256 为 a5a20b480fc5cb5722648587a85f3a200269d06154b7943de847b54897044959.  完整本地证据在 `.e2e-test/runs/managed-pane-independent-20261026/report.md`.

通过项包括真实 shell/TUI 并发、主 pane 接任保持 ID/PID/native/xats、最后 pane 回收、跨 session/替换/旧 helper 拒绝及零副作用、Codex/OpenCode 双 pane 配置与身份隔离、父 stop/start 连续性、真实 release 后本地清理失败恢复、六个定向 SQLite/存储故障测试与原主 session 回归.  独立代码复核 PASS.  未操作宿主真实 tmux、未执行全量测试或生产部署.

中央技能交接: `jt-aoe-cli` 1.1.0 已由 skills-creator 保存到中央源码仓库并更新 `~/.skills-manager/custom/jt-aoe-cli`, 源与安装副本 SHA256 均为 5a9d951abcd415f75cb61bca5c343c312f802edd94ee04e2ea0d4057533ac38f.  不包含向各 agent 目录的分发, Claude 暂缓范围已写入技能.
