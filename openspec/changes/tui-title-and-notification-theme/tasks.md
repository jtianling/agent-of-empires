## 1. TUI Panel Title

- [x] 1.1 Change title format in `src/tui/home/render.rs:131` from `" Agent of Empires [{}] "` to `" AoE [{}] "`

## 2. Notification Bar Theme Colors

- [x] 2.1 Update `STATUS_LEFT_FORMAT` in `src/tmux/status_bar.rs` to use hex colors: index `#22c55e`, title `#cbd5e1`, hint `#94a3b8`, notification `#fbbf24`, from-title `#64748b`
- [x] 2.2 Update any other hardcoded colour references in `status_bar.rs` that use the old 256-color values (colour46, colour252, colour245, colour220)

## 3. Spec Updates

- [x] 3.1 Update `openspec/specs/notification-bar/spec.md` to replace colour220/colour245 references with theme-aligned hex color values

## 4. Verification

- [x] 4.1 Run `cargo build` to verify compilation
- [x] 4.2 Run `cargo clippy` and `cargo fmt` to ensure code quality
- [x] 4.3 Run `cargo test` to verify no regressions (1061/1061 lib tests pass; worktree_integration test failure is pre-existing)
