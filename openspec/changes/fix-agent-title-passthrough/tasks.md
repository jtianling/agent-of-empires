## 1. 修复 TITLE_REFRESH_HOOK 时机

- [x] 1.1 将 `TITLE_REFRESH_HOOK` 从 `setup_nested_detach_binding()` (首次 attach 后) 移到 `enable_tmux_titles()` (TUI 启动时), 确保第一次 session switch 就能触发标题更新
- [x] 1.2 在 `restore_tmux_titles()` 中清理 hook
- [x] 1.3 从 `cleanup_nested_detach_binding()` 中移除 hook 清理, 并保持 Codex CLI 当前 pane title 管理逻辑不变

## 2. 测试验证

- [x] 2.1 运行 `cargo fmt`
- [x] 2.2 运行 `cargo clippy`
- [x] 2.3 运行 `cargo test`
  - 2026-03-12 实际执行了完整 `cargo test`, 当前环境中唯一失败的是 `containers::docker::tests::test_docker_image_exists_locally_with_common_image`, 原因是 Docker Hub TLS 证书校验异常, 与本次标题透传改动无关
