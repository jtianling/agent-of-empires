## 1. Narrow-screen detection helper

- [x] 1.1 Add `is_narrow_layout(available_width: u16) -> bool` method to `HomeView` in `src/tui/home/mod.rs` that returns `true` when `available_width < self.list_width + 20`

## 2. Single-panel rendering

- [x] 2.1 In `src/tui/home/render.rs` `render()`, use `is_narrow_layout()` to branch layout: narrow mode renders list at full width with no preview panel; normal mode keeps existing two-panel layout
- [x] 2.2 In `src/tui/app.rs` render loop, use the same narrow check to skip `update_caches()` when preview is not visible

## 3. Verification

- [x] 3.1 Run `cargo fmt`, `cargo clippy`, and `cargo test` to ensure no regressions. Clean on 2026-08-02: `cargo fmt --check` and `cargo clippy --all-targets` report nothing, and all 1350 unit tests pass. The E2E suites were not run here because this machine hosts live AoE tmux sessions; the suites covering the areas this change touches were run separately under an isolated socket and were green.
