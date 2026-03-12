## 1. Add push/pop title functions to tab_title.rs

- [x] 1.1 Add `push_terminal_title` function that writes CSI 22;2 t (`\x1b[22;2t`) to push the current title onto the xterm title stack
- [x] 1.2 Add `pop_terminal_title` function that writes CSI 23;2 t (`\x1b[23;2t`) to restore the previously pushed title
- [x] 1.3 Add unit tests for both functions verifying correct escape sequences are written

## 2. Push title on TUI startup

- [x] 2.1 In `src/tui/mod.rs::run`, read `dynamic_tab_title` from config before App construction
- [x] 2.2 If `dynamic_tab_title` is enabled, call `push_terminal_title` before any title-setting code runs (before entering alternate screen or constructing App)

## 3. Restore title on all exit paths

- [x] 3.1 Replace `clear_terminal_title` with `pop_terminal_title` in the normal exit cleanup path in `src/tui/mod.rs::run` (line 149)
- [x] 3.2 Replace `clear_terminal_title` with `pop_terminal_title` in the panic hook in `src/tui/mod.rs::run` (line 111)
- [x] 3.3 Replace `clear_terminal_title` with `pop_terminal_title` in `src/tui/app.rs` where `dynamic_tab_title` is toggled off in settings (line 453)

## 4. Cleanup and verification

- [x] 4.1 Remove `clear_terminal_title` function from `tab_title.rs` if no longer referenced, or keep if still needed for other purposes
- [x] 4.2 Update existing tests for `clear_terminal_title` to test `pop_terminal_title` instead
- [x] 4.3 Run `cargo fmt`, `cargo clippy`, and `cargo test` to verify everything compiles and passes
