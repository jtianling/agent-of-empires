## 1. Shorten TUI Home Title

- [ ] 1.1 In `src/tui/home/render.rs:127`, change `" Agent of Empires [{}] "` to `" AoE [{}] "`

## 2. Pre-fill Rename Dialog Title

- [ ] 2.1 In `src/tui/dialogs/rename.rs:59`, change `new_title: Input::default()` to `new_title: Input::new(current_title.to_string())`
- [ ] 2.2 Update rename dialog unit tests to account for pre-filled title value (tests that type text will now append to existing value; tests that check empty initial state need updating)

## 3. Verify

- [ ] 3.1 Run `cargo fmt`, `cargo clippy`, and `cargo test` to ensure everything passes
