## 1. Tilde expansion on directory submit

- [x] 1.1 In `src/tui/dialogs/group_rename.rs`, import `expand_tilde` from `crate::tui::components::path_ghost`
- [x] 1.2 In `directory_result()`, call `expand_tilde()` on the trimmed directory value before returning `Some(...)`
- [x] 1.3 Add unit test: submitting directory `~/projects` returns expanded absolute path

## 2. Remove unused imports suppression

- [x] 2.1 In `src/tui/components/mod.rs`, remove `#[allow(unused_imports)]` attribute
- [x] 2.2 Remove `longest_common_prefix` from the `pub use text_input::{...}` re-export line

## 3. Remove speculative candidates field

- [x] 3.1 In `src/tui/components/path_ghost.rs`, remove the `candidates: Vec<String>` field and its `#[allow(dead_code)]` from `PathGhostCompletion`
- [x] 3.2 Remove the `candidates` population in `compute()` (stop collecting into a vec for the field)
- [x] 3.3 Verify existing tests still pass after removal

## 4. Verification

- [x] 4.1 Run `cargo fmt`
- [x] 4.2 Run `cargo clippy --all-targets --all-features -- -D warnings` with zero warnings
- [x] 4.3 Run `cargo test --lib` with all tests passing
