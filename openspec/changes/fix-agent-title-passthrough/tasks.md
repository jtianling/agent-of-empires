## 1. 修复 TITLE_REFRESH_HOOK 时机

- [x] 1.1 将 `TITLE_REFRESH_HOOK` 从 `setup_nested_detach_binding()` (首次 attach 后) 移到 `enable_tmux_titles()` (TUI 启动时), 确保第一次 session switch 就能触发标题更新
- [x] 1.2 在 `restore_tmux_titles()` 中清理 hook
- [x] 1.3 从 `cleanup_nested_detach_binding()` 中移除 hook 清理 (已移到 restore_tmux_titles)

## 2. 测试验证

- [x] 2.1 运行 `cargo test` 和 `cargo clippy` 确保通过
