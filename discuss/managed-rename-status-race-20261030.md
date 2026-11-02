2026-10-30, steward 报告在 profile `auto-coworkspace-bedc` 中,
通过正式 `managed rename` 去掉 5 个任务的槽号后, TUI 全部显示红叉.
只读检查确认 5 个 tmux session 已正确使用新名称, 原 10 个 pane `%93` 至 `%102`
均存活, 工作目录保持不变.  managed capture 也成功, 因而不能把缺失 tmux 改名当作根因.
当时没有记录该次改名的调试日志, 不以存储的 `idle` 代替实时存活证据.

独立 tester 在临时 HOME 和私有 socket 中确定性复现旧版问题:
暂停已经持有旧标题的后台 pane 采样, 执行真实 CLI rename,
等待 TUI 磁盘刷新显示新标题, 再释放旧采样, 新标题被旧结果误标为红叉.
前后 logical pane ID、tmux pane ID 和 PID 不变且仍 live.
复现报告为 `.e2e-test/runs/external-rename-status-repro-20261030-040348-s_146xrh/report.html`.

根因是状态结果仅按 session ID 和启动时间匹配, 没有携带采样时的标题.
外部标题刷新仍继承旧错误和退避状态, 后台活动缓存也只按 ID 保存.
TUI 内部改名曾通过丢弃下一次 Error 缓解, 但外部改名没有该保护,
而忽略一次 Error 还可能吞掉当前名称下的真实故障.

修复为状态结果携带原始标题, 在应用状态及恢复 token 前校验标题和启动代次.
外部刷新发现标题变化时清除旧错误、退避及 spinner 状态;
后台发现标题变化时清除旧活动与状态缓存, 对未冻结的 session 重新采样.
移除忽略一次 Error 的旧补丁.  tmux rename 统一刷新本进程缓存,
managed CLI 使用准确 has-session 检查, 不用可能过期的 exists 缓存决定是否改名.

首个修复候选通过独立静态审查及连续三轮受控竞态验收,
每轮等待新标题出现后释放旧采样并观察 6 秒, 没有红叉,
logical pane ID、tmux pane ID 和 PID 保持不变.
隔离环境中真实 session 失效仍显示红叉与恢复标记.
报告为 `.e2e-test/runs/external-rename-status-fixed-20261030-040926-vdcx11tb/report.html`.
审查提出的重复缓存刷新已移除, 最终候选另行冻结复核.

旧版无需重启的现场刷新办法也已独立验证:
大写 `P` 选择另一个已有 profile 并确认, 再按 `P` 返回原 profile,
会重新建立界面状态, 保留 pane 和进程.  在当前 profile 上直接确认只是取消.
`r`、`R` 是重启动作, 不用于本次刷新.
报告为 `.e2e-test/runs/external-rename-profile-recovery-20261030-040943-rjpz71hu/report.html`.
已将此办法交给 steward 转告用户, 尚无业务现场刷新后的回执.

项目经验: 异步状态采样必须携带决定查找目标的身份信息,
不能仅凭稳定逻辑 ID 接受依赖旧名称的结果.
丢弃过期结果时也要处理源头缓存和错误退避, 同时验证当前目标的真实故障仍会报告.
隔离复现和安全刷新通过不代表业务现场已经恢复, 两者应分别记录.

最终候选通过独立静态复审, 无 CRITICAL、WARNING 或 SUGGESTION.
去重缓存刷新后的二进制再次完成三轮外部竞态、真实 `e` 内部改名、
logical pane ID、tmux pane ID、PID 保留及真实故障负向验收.
最终报告为 `.e2e-test/runs/external-rename-status-final-20261030-041247-8y5oen_r/report.html`.
宿主仅运行构建、格式、Clippy 全目标编译与已有 Python CLI 隔离回归,
没有执行宿主全量 Rust 测试或业务 tmux 探针/清理.

最终 SHA256 为 `e5ac69b7086b570c7b88dbea6b3b9515c344b15439120e2797a4634d8cd9b98b`,
核对 210 项生产输入后已原子安装到 `/Users/jtianling/.cargo/bin/aoe`.
旧版备份为 `target/candidates/aoe-before-rename-status-344eb4a469a99a4a56a7db39a7fba2bdb6dc033c0c0e338a1c45b97f5a20c247`.
没有重启现有 TUI、session、Codex、tmux 或 xats, 也没有回滚业务标题.
当前旧 TUI 进程不会热加载新代码, 防竞态逻辑需由用户之后正常重新打开 TUI 时载入.
现场仍红叉时可先使用已验证的 profile 往返刷新, 业务现场恢复状态待用户回执.
