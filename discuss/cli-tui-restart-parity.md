# CLI session 的 TUI 恢复一致性

2026-10-29, 用户反馈 CLI 创建的第 18 号 session 与人工创建的 session 在 `r/R` 恢复行为上不一致, 并要求同时检查 `c/C`.  现场目标为 `monkeys-copy`, ID `105a0688d54b4ed8`, 含两个独立 xats Codex pane.  当前运行 TUI 的可执行文件 inode 与已安装版本一致, 排除了旧进程构建的影响.

只读检查发现 JSON 保存了两项有效 thread ID, SQLite slot 的 native ID 为空.  独立容器已证明空 slot 本身不导致热重启失败.  当前 rollout 只有会话元数据和名称事件; 用户补充空白截图可能是在按 `c` 后取得, 因此不将截图作为 `r` 丢失历史的证据.  生产 daemon 的正常业务拒绝不落日志, 没有错误日志也不能证明请求成功.

独立 tester 在真实 Codex 和 xats daemon 副本中复现冷恢复失败: CLI 创建双 pane 后停止, 再在 TUI 按 `r`, 返回 `xats bind failed (HTTP 409 Conflict, runtime_conflict): Instance already has other coordinates`.  原因是 `recover_from_slots` 创建占位 pane 时, `start_with_size_inner` 提前调用 managed runtime 准备逻辑, 在正式 xats 换代之前将新服务坐标绑定到旧 fence.

第一版修复仅延迟了 runtime 准备, 消除了 409, 但独立四冷键验收发现第二个问题: tmux server 重建后, 新主 pane 可能复用旧附加 pane 的编号, 原位置保护将它判作被替换, 导致附加 pane 恢复失败.  第一版保留为失败候选, 没有安装.

第二版在创建新 pane 前校验附加 pane 的槽位 key、代数及旧位置, 对仍位于其他 session 的旧 pane 拒绝恢复.  校验后使用既有 CAS 操作解除旧物理绑定并持久化, 保留 native ID 和身份.  SQLite 已清而 JSON 未清的部分失败可在重试时归一化, 创建新对象前捕获 Claude 会话; 清理前的 slot 快照保留给布局映射.  主 pane 正式恢复仍走原有 `managed_lifecycle` 的换代和绑定, 附加 pane 仍独立恢复.  不改变 owner、fence、进程范围或 worktree 清理规则.

第三版补齐独立 review 的边界: 附加 pane 的全部只读预检先于 pending 槽位换代, 拒绝外来位置时不写入恢复状态.  提前捕获仅覆盖已校验的 Claude 附加 pane, 不验证主 pane 的旧失效编号.  新增默认省略的 `managed_recovery_panes` 槽位到物理编号映射, 解绑前持久化, 跨 SQLite、JSON 或占位创建失败重试保留, 完整恢复成功后先同步新的物理布局快照, 再清除映射; fork 不继承该中间状态.  现有 session JSON 无需迁移.  tester 已用可丢弃夹具分别复现 pending 拒绝副作用及占位创建失败后布局退化, 第三版已通过对应红绿验收.  最终增量同步恢复后的新布局快照, 覆盖不等待后台轮询就连续停止、恢复的窗口, 对照仍保留各候选实际二进制来源.

项目经验: 恢复测试必须先完成实际对话和工具调用, 再比较恢复后的 ID、历史及新工具调用.  只比较空 thread ID 或进程存活, 不能证明用户任务可恢复.  占位启动也必须验证没有提前创建和绑定外部 runtime.  冷恢复要包含 tmux server 重建后的物理编号复用, 并在新对象分配前清除经过校验的失效绑定.  位置校验必须先于 pending 换代, 且兼容 SQL 已应用、JSON 未提交的状态.  布局重试既要保存旧映射, 也要在成功后同步新快照; 仅保留 cache 不能证明下一轮仍可恢复.  外来位置拒绝须同时比较进程、JSON 和 SQL, 不能只检查没有杀进程.  Linux 用例导出到 macOS 时, 大小写按键必须使用不同目录前缀, 防止大小写不敏感的文件系统折叠证据.

候选与无凭据现场摘要位于 `target/review-builds/20261029-cli-tui-restart/`.  分阶段独立验收合计 97 项, 96 PASS、0 FAIL、1 BLOCKED.  最终 candidate5 的冷四键及连续恢复为 26 项, candidate2/3 的热路径和普通创建等 67 项保留实际二进制来源, 另有 3 项证据与隔离核对; 不声称全部在最终二进制重跑.  Claude 真实补验仅有 2.1.62, 不支持当前 ElicitationResult hook, 未达到对话前提而 BLOCKED.  最终 reviewer Avicenna 为 PASS, CRITICAL 0、WARNING 0.

固定报告为 `http://127.0.0.1:60109/report.html`, 到期 UTC `2026-10-31T11:03:36.318672Z`.  离线文件位于 `.e2e-test/runs/cli-tui-restart-parity-20261029-102023-oh06bcqa/report.html`.  最终 Linux 二进制 SHA256 为 `fb7aa8cdfdf5f86e8d71aad2a26f647720ea1f4abfabf93e5a06c19e43a1422b`, 208 项生产源码无漂移.  自建容器已精确删除, 宿主 tmux 未操作.

已沿用用户此前的全局安装授权更新 `/Users/jtianling/.cargo/bin/aoe`, SHA256 为 `6b9d2fc9e6160e4d840e71e637dace14337ff68a45064eb49e16a3362afabe40`.  上一全局版本已备份, 完整记录位于候选目录的 `global-install-info.json`.  没有重启正在运行的 TUI 或 agent, 没有操作第 18 个业务 session.  用户正常退出并重开 TUI 后生效.
