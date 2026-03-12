## Context

AoE 在 tmux 中运行, 每个 agent session 是独立的 tmux session. 用户通过 `switch-client` 在 AoE TUI 和 agent session 之间切换. tmux 的 `set-titles on` + `set-titles-string "#T"` 将当前 active pane 的标题 (`#T`) 推送到外层终端 (Alacritty).

问题: tmux 的 `set-titles` 只在 pane title **发生变化**时推送, `switch-client` 切换 session 不触发推送. 因此切换 session 后外层终端标题不会立即变成新 session 的 pane title.

## Key Insight

AoE 写 OSC 0 到**自己的 pane**, Claude Code / Gemini CLI 写 OSC 0 到**各自的 pane**, Codex CLI 则继续由 AoE 通过 `select-pane -T` 维护 pane title. 它们在不同的 tmux session, 互不干扰. `set-titles-string "#T"` 根据当前 active pane 决定显示什么. AoE 不需要新增 agent 特判或全局停写标题来让 Claude / Gemini 标题透传, 只需要确保 tmux 在 session 切换时正确推送当前 pane title.

## Goals / Non-Goals

**Goals:**
- 每次 `switch-client` 后, 外层终端标题立即反映新 session 的 pane 标题
- 第一次进入 Claude Code / Gemini CLI session 就生效, 不需要 "第二次才正常"
- Codex CLI session 继续显示 AoE 当前维护的 pane title

**Non-Goals:**
- 不修改 AoE 的标题计算逻辑 (`tab_title_state`, `compute_title`)
- 不修改 agent 的 `sets_own_title` 配置
- 不改变 Codex CLI 的 pane title 来源

## Decisions

### 1. 在 TUI 启动时安装 client-session-changed hook

将 `TITLE_REFRESH_HOOK` 的安装从 `setup_nested_detach_binding()` (首次 attach 后才调用) 移到 `enable_tmux_titles()` (TUI 启动时立即调用). 这确保 hook 在第一次 session switch 之前就已就绪.

Hook 命令: `select-pane -T '#{pane_title}.' ; select-pane -T '#{pane_title}'`

原理: 先追加 `.` 再恢复, 强制 tmux 检测到 title "变化", 触发 `set-titles` 推送到外层终端. 对 Claude Code / Gemini CLI, 这会把 agent 已经设置好的 pane title 原样透传出去. 对 Codex CLI, 这会重新推送 AoE 维护的 pane title, 因此现有行为保持不变.

### 2. 在 restore_tmux_titles 中清理 hook

hook 的安装和清理都放在 `mod.rs` 的 tmux title 管理代码中, 保持职责内聚.

## Risks / Trade-offs

- [已验证] `select-pane -T '#{pane_title}'` 中的 `#{pane_title}` 在 hook fire 时由 tmux 展开, 不受单引号影响
- [低风险] hook 每次 session switch 都执行两次 `select-pane -T`, 开销可忽略
- [中风险] 如果误把问题建模成 "TUI 对 `sets_own_title` agent 全局停写 OSC 0", 会把行为范围写错成所有活跃 session, 与实际所需的 "当前切换到的 session 标题透传" 不一致
