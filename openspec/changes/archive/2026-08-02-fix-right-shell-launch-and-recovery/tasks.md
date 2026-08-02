## 1. Shell 启动语义

- [x] 1.1 为 extra shell command 增加回归测试, 证明用户 shell 为 zsh 时不会固定经过 Bash login profile, 并覆盖 cwd quoting 和 `stty susp undef` wrapper
- [x] 1.2 复用 primary pane 的用户 POSIX shell 选择语义构造 extra shell command, 删除固定 `bash -lc` 启动路径

## 2. Managed shell durable lifecycle

- [x] 2.1 更新 launch-time slot 测试, 证明同目录和不同目录的 managed shell pane 都立即写入正确 cwd 的 durable slot
- [x] 2.2 删除同目录 shell slotless 例外, 让 New Session、Fork、`%` 和 CLI managed-pane action 通过统一路径记录 shell slot
- [x] 2.3 增加 restart/recovery focused coverage, 证明 Codex + Shell 两个 slot 都会重建, shell 恢复 cwd, Codex xats identity key 保持不变

## 3. 验证

- [x] 3.1 在私有 tmux socket、隔离 HOME 且移除 `TMUX`/`TMUX_PANE` 的环境运行相关执行级或 E2E 测试, 精确清理本次创建的资源
- [x] 3.2 运行 `cargo fmt --check`、`cargo check`、`cargo clippy` 和与本变更相关且不接触 live tmux server 的 targeted tests
- [x] 3.3 运行 `openspec validate fix-right-shell-launch-and-recovery --strict`, 确认实现与 delta specs 一致并勾选完成 tasks
