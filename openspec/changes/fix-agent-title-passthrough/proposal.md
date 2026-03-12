## Why

进入 AoE 管理的 agent session 后, 外层终端标题不会在第一次 `switch-client` 时刷新到该 session 当前 pane title. 这导致 Claude Code 和 Gemini CLI 在 AoE session 内无法像直接运行时那样透传自己的标题. Codex CLI 依赖 AoE 维护 pane title, 这里不需要改变它的现有机制. 根本原因是 tmux 在 `switch-client` 切换 session 时不会自动重新计算 `set-titles-string` 并推送到外层终端.

## What Changes

- 将 `TITLE_REFRESH_HOOK` (`client-session-changed[98]`) 从 `setup_nested_detach_binding()` 移到 `enable_tmux_titles()`, 确保在 TUI 启动时就安装 hook, 不错过第一次 session switch
- hook 机制: 每次 session 切换时, 通过 toggle pane title 强制 tmux 重新推送 `set-titles-string` 到外层终端
- 保持现有 agent 分工不变: Claude Code 和 Gemini CLI 继续自行设置 pane title 并透传, Codex CLI 继续使用 AoE 当前的 pane title 管理逻辑

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-tab-title`: 修复 session 切换时标题不更新的问题, 使 Claude Code / Gemini CLI 当前 session 的 pane title 正确透传到外层终端, 同时保持 Codex CLI 现有标题机制不变

## Impact

- `src/tui/mod.rs`: TITLE_REFRESH_HOOK 安装和清理
- `src/tmux/utils.rs`: 移除原来放错位置的 hook 逻辑
- `src/tui/status_poller.rs`: 保持非 `sets_own_title` agent 的 pane title 管理逻辑不变, 作为 Codex 回归保护点
