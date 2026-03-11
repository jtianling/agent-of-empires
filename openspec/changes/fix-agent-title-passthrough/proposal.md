## Why

进入 AoE 管理的 agent session 后, Alacritty tab 标题不更新 -- 无论是 Claude Code/Gemini CLI 自己设置的标题, 还是 AoE 为 Codex 设置的 pane 标题, 都不会反映到外层终端. 根本原因是 tmux 在 `switch-client` 切换 session 时不会自动重新计算 `set-titles-string` 并推送到外层终端.

## What Changes

- 将 `TITLE_REFRESH_HOOK` (`client-session-changed[98]`) 从 `setup_nested_detach_binding()` 移到 `enable_tmux_titles()`, 确保在 TUI 启动时就安装 hook, 不错过第一次 session switch
- hook 机制: 每次 session 切换时, 通过 toggle pane title 强制 tmux 重新推送 `set-titles-string` 到外层终端

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-tab-title`: 修复 session 切换时标题不更新的问题, 使 agent 标题正确透传到外层终端

## Impact

- `src/tui/mod.rs`: TITLE_REFRESH_HOOK 安装和清理
- `src/tmux/utils.rs`: 移除原来放错位置的 hook 逻辑
