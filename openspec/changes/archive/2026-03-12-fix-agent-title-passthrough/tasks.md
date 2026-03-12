## 1. 回退 AoE TUI 自己的标题生命周期

- [x] 1.1 从 `src/tui/mod.rs` 去掉 tmux `set-titles` / title stack / title restore 逻辑, 让 AoE 自己 session 的标题路径回到 `ddba37c` 风格
- [x] 1.2 从 `src/tui/app.rs` 和 `src/tui/home/mod.rs` 去掉动态 tab title 状态更新和推导
- [x] 1.3 删除 `src/tui/tab_title.rs` 和 `src/tmux/utils.rs` 里只为这套 AoE 内部标题逻辑新增的 hook/helper

## 2. 删除对应配置和 settings UI

- [x] 2.1 从 `src/session/config.rs` 移除 `dynamic_tab_title`
- [x] 2.2 从 settings 字段和输入逻辑中移除 `DynamicTabTitle`
- [x] 2.3 添加 migration, 清理旧配置中的 `dynamic_tab_title`

## 3. 测试验证

- [x] 3.1 运行 `cargo fmt`
- [x] 3.2 运行 `cargo clippy`
- [x] 3.3 运行 `cargo test`
- [x] 3.4 runtime probe: AoE 运行期不安装 `client-session-changed[98]`, 设置页中不再显示 `Dynamic Tab Title`
