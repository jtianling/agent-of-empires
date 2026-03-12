## Context

`ddba37c` 时, AoE TUI 自己没有 terminal title 生命周期: `src/tui/mod.rs` 只负责 TUI 启停和 nested detach 清理, `src/tui/app.rs` 不会持续写 OSC 0, 配置里也没有 `dynamic_tab_title`.

当前实现把多层逻辑叠在一起:
- `src/tui/mod.rs` 启动时改 tmux `set-titles` / `set-titles-string`, 退出时再恢复
- `src/tui/mod.rs` 和 panic hook 会 push/pop terminal title stack
- `src/tui/app.rs` 每轮事件循环都可能根据 TUI 状态写新的 OSC 0 标题
- `src/tmux/utils.rs` 额外装了一个 `client-session-changed[98]` hook

这让 AoE 自己 session 显示时的标题路径和 agent session 的 pane title 管理混在了一起. 用户这次要的是收敛, 不是继续给 AoE TUI 增加更多标题规则.

## Goals / Non-Goals

**Goals:**
- AoE 自己 session 显示时, 标题相关代码路径回到 `ddba37c` 的简单形态
- 删除只为 AoE TUI 动态 tab title 存在的配置、settings 和 cleanup 逻辑
- 保留与标题无关的现有 TUI 行为, 比如 tmux mouse、launch_dir、同步渲染等改动

**Non-Goals:**
- 不把整个仓库 reset 到 `ddba37c`
- 不回退与标题无关的后续改动
- 不改动 agent session 生命周期本身, 除非某段代码只为了 AoE 内部 title 逻辑而存在

## Decisions

### 1. 让 `src/tui/mod.rs` 回到 ddba 风格的职责边界

`run()` 里只保留 TUI 启停、panic cleanup、tmux mouse 和 nested detach 相关职责. title stack push/pop、`set-titles`/`set-titles-string` 修改、title restore 都从这里移除.

### 2. 删除 AoE TUI 自己的动态 tab title 生命周期

`src/tui/app.rs` 里的 `dynamic_tab_title`、`last_tab_title`、`compute_title()` 调用和 settings toggle refresh 都删掉. `src/tui/home/mod.rs` 里只为 tab title 推导存在的 `pane_titles` / `tab_title_state()` 也一起删除.

### 3. 删除对应配置和 settings UI

既然 AoE TUI 不再管理 terminal tab title, `dynamic_tab_title` 配置项和 settings 开关也一起删除, 避免留下失效行为. 由于这是持久化 config schema 变更, 需要通过 migration 主动清理已有 `config.toml` 中的旧字段, 而不是只依赖 serde 忽略未知字段.

## Risks / Trade-offs

- [已知行为变化] AoE 自己 TUI 不再主动更新终端标题, 这相当于撤销此前的动态 tab title 能力
- [低风险] migration 只清理全局 `config.toml` 下的 `app_state.dynamic_tab_title`, 不影响其他配置项
- [中风险] 如果顺手删掉 agent session 仍在使用的 pane title 管理代码, 可能误伤 Codex 等不自管标题的工具, 所以实现时要把 AoE TUI title 生命周期和 agent pane title 管理区分开
