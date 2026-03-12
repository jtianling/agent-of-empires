## Why

最近几轮 terminal title 改动把 AoE 自己的 TUI session 也卷进来了: `src/tui/mod.rs` 会改 tmux `set-titles` 配置, `src/tui/app.rs` 会持续写 OSC 0, 退出时还会做 title stack push/pop 恢复. 这套逻辑已经超出当前需求, 也让 AoE 自己 session 显示时的标题路径变得混乱. 用户希望 AoE 自己 session 的标题相关逻辑回到 `ddba37cd9f020bcd2ea983fa4bf6c7ca024fc4cb` 的思路, 而不是继续堆叠更多修补.

## What Changes

- 将 AoE TUI 自己的 terminal title 生命周期收敛回 `ddba37c` 风格: 不再在 TUI 启动/运行/退出时写 terminal title escape sequence, 也不再在 `run()` 中改 tmux `set-titles`
- 删除为这套 AoE 内部标题逻辑新增的 `tab_title` wiring、`dynamic_tab_title` 配置和 settings 开关, 避免保留无效控制项
- 保留与 agent session 自身 pane title 管理直接相关的逻辑, 但撤销把 AoE 自己 session 也纳入同一套 title 生命周期的改动

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-tab-title`: AoE 自己 session 显示时不再主动管理外层终端标题, 行为回到 `ddba37c` 风格
- `configuration`: 移除 `dynamic_tab_title` 配置项和对应 settings UI

## Impact

- `src/tui/mod.rs`: 去掉 AoE TUI 的 tmux title setup/restore 和 title stack cleanup
- `src/tui/app.rs`: 去掉动态 tab title 状态更新
- `src/tui/home/mod.rs`: 去掉仅用于 tab title 推导的内部状态
- `src/tui/tab_title.rs`: 删除不再使用的模块
- `src/session/config.rs`: 移除 `dynamic_tab_title`
- `src/tui/settings/{fields,input}.rs`: 移除对应 settings 字段
- `src/tmux/utils.rs`: 删除本轮尝试新增的 title refresh hook
- `src/migrations/`: 添加 migration, 清理旧配置中的 `dynamic_tab_title`
