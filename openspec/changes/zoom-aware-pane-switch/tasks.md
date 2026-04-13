## 1. Zoom-aware C-; keybinding

- [x] 1.1 In `src/tmux/utils.rs` `setup_session_cycle_bindings()`, change the `C-;` binding from `select-pane -t :.+` to use `if-shell -F "#{window_zoomed_flag}"` that re-zooms after switching when zoomed, and keeps existing behavior when not zoomed

## 2. Auto-zoom on narrow attach

- [x] 2.1 In `src/tui/app.rs` attach flow (before `tmux_session.attach()`), detect narrow terminal using `is_narrow_layout()`, query pane count for the session, and if narrow and multi-pane, run `tmux resize-pane -Z -t <session>:.0` to auto-zoom the agent pane

## 3. Verification

- [ ] 3.1 Run `cargo fmt`, `cargo clippy`, and `cargo test` to ensure no regressions
