## 1. Core title save/restore functions

- [x] 1.1 Add `save_terminal_title` function to `src/tui/tab_title.rs` that writes CSI 22;2 t (push title to stack)
- [x] 1.2 Replace `clear_terminal_title` with `restore_terminal_title` that writes CSI 23;2 t (pop title from stack)

## 2. Integration

- [x] 2.1 Call `save_terminal_title` in `src/tui/mod.rs` before the first title set, guarded by `dynamic_tab_title` config
- [x] 2.2 Replace `clear_terminal_title` calls with `restore_terminal_title` in normal exit path (`src/tui/mod.rs`)
- [x] 2.3 Replace `clear_terminal_title` call with `restore_terminal_title` in panic hook (`src/tui/mod.rs`)
- [x] 2.4 Replace `clear_terminal_title` call in settings toggle-off path (`src/tui/app.rs`) with `restore_terminal_title`

## 3. Verification

- [x] 3.1 Build and manually test: launch aoe, exit, verify Alacritty tab title is restored
- [x] 3.2 Run `cargo fmt` and `cargo clippy`
- [x] 3.3 Run `cargo test`
