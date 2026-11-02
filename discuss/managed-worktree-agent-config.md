2026-10-30, 用户反馈 managed CLI 接入的 worktree 缺少 skills.  现场流程经 steward 的原始调用记录确认: 先由调用方执行 `git worktree add`, 再执行 `managed create` 和 `managed pane create`.  外部 Git 创建不会经过 AoE `GitWorktree::create_worktree()` 中的 agent 配置同步, 而 managed CLI 原先只验证并使用已有目录, 因此漏掉该步骤.

本次在两个 managed 创建入口启动 runtime 前, 识别已有 linked worktree 根目录并复用原有同步函数.  来源及拷贝范围沿用 TUI 规则, 已有目标目录整体保留, 单独列出的缺失配置文件可补齐.  普通目录、主仓库及 worktree 子目录跳过.  不写入 worktree 托管或清理元数据, 删除 managed session 仍保留外部目录及分支.

复用同步到已有目录时, 必须将失效的目标符号链接也视为已有内容.  本轮将目标存在判断改为 `symlink_metadata`, 避免跟随失效链接创建其外部目标.  保留原有嵌套配置文件的目标符号链接祖先检查.

项目经验: 接入外部创建的资源时, 不能把资源已经存在等同于 AoE 初始化已经完成.  应明确复用必要的配置准备步骤, 同时独立保留资源的删除归属.  不应为了补 skills 复制而给外部 worktree 写入托管元数据.

回归脚本为 `tests/managed_worktree_cli_smoke.py`, 使用临时 HOME、真实临时 Git 仓库与拒绝全部调用的 tmux sentinel.  旧全局版本可复现创建成功但 skills 缺失, 修订候选通过 10 类检查, 原 managed CLI smoke 也通过.  `cargo build --profile dev-release`、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 和差异格式检查通过.  宿主有实时 session, 未运行全量 Rust 测试或 tmux 探针, 未重启 xats 或业务 session.

独立 tester 使用临时 Git、隔离 HOME 和私有 tmux socket 完成 8 项验收, 包括运行中 shell session 的真实 pane 创建与移除, 全部通过.  报告位于 `.e2e-test/runs/managed-worktree-independent-20261029-170134-6f3qr_6m/report.html`.  首次夹具 socket 路径过长, 缩短私有路径后完整重跑通过, 原始阻塞记录保留在报告目录.  验收未调用真实云模型或共享 xats, 结论针对配置同步及相关生命周期边界.

独立审查发现首个候选把可选同步的 Git 识别错误传播成创建失败.  用真实 SHA256 Git 仓库复现该回归后, 已改为记录目录和具体原因并跳过可选同步, 保持 managed 创建原有的目录兼容性.  修订版静态复审 PASS, 无新增问题.  项目经验: 给已有流程添加可选初始化时, 不能无意中新增第三方库的硬依赖, 验收要包含该依赖无法识别但原流程支持的输入.

修订候选完成独立隔离复验, 11 项全部通过, 包含 SHA256 仓库和损坏 Git 配置的主 session 与运行中 pane 创建, 以及具体警告记录.  最终报告为 `.e2e-test/runs/managed-worktree-regression-20261029-170607-xp414a80/report.html`.  最终二进制 SHA256 为 `344eb4a469a99a4a56a7db39a7fba2bdb6dc033c0c0e338a1c45b97f5a20c247`, 已核对 210 项生产输入后原子安装到 `/Users/jtianling/.cargo/bin/aoe`.  旧版备份保留在 `target/candidates/aoe-before-worktree-sync-0d606058d8d265a87429a6fce7720063073d554d7f7d07a21a33172df2eb3a58`.  没有追补其他项目中的既有业务目录, 无需重启 AoE TUI 或 xats.
