## 1. Fix same-profile rename path

- [x] 1.1 In `rename_selected()`, capture the old tmux Session (or old tmux name) BEFORE `mutate_instance` changes the title
- [x] 1.2 Use the old session to call `tmux rename-session` with the new name derived from `effective_title`

## 2. Fix cross-profile rename path

- [x] 2.1 In the cross-profile branch of `rename_selected()`, capture the old tmux Session BEFORE mutating `instance.title`
- [x] 2.2 Use the old session to call `tmux rename-session` with the new name

## 3. Testing

- [x] 3.1 Add a unit/integration test: rename a session, verify tmux session name changes and process survives
- [x] 3.2 Run `cargo fmt`, `cargo clippy`, `cargo test` to ensure no regressions
