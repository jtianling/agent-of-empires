## 1. Data Model Changes

- [x] 1.1 Add `reuse_worktree: bool` field to `InstanceParams` in `src/session/builder.rs`
- [x] 1.2 Add `reuse_worktree: bool` field to `NewSessionData` in `src/tui/dialogs/new_session/mod.rs`

## 2. Builder: Support Worktree Reuse

- [x] 2.1 In `build_instance()` `create_new_branch` path (`src/session/builder.rs` lines 128-150): when `reuse_worktree` is true and `worktree_path.exists()`, skip `git worktree add` and set `managed_by_aoe: false`, `cleanup_on_delete: false`
- [x] 2.2 In `build_instance()` `!create_new_branch` path: when worktree not found in git's list but path exists and `reuse_worktree` is true, reuse the path similarly

## 3. TUI: Two-Press Confirmation Flow

- [x] 3.1 Add `confirm_reuse_worktree: bool` state field to `NewSessionDialog`
- [x] 3.2 In the submit path of `handle_key`: before submitting, check if worktree branch is set and computed path exists. If so, set `confirm_reuse_worktree = true` and show warning in `error_message`, return `DialogResult::Continue`
- [x] 3.3 On second Enter press when `confirm_reuse_worktree` is true, set `reuse_worktree = true` in `NewSessionData` and proceed with submit
- [x] 3.4 Clear `confirm_reuse_worktree` when user changes the worktree branch field or other relevant fields

## 4. Creation Poller: Thread Through Flag

- [x] 4.1 Pass `reuse_worktree` from `NewSessionData` through to `InstanceParams` in `CreationPoller::create_instance()`

## 5. CLI: Add `--reuse-worktree` Flag

- [x] 5.1 Add `--reuse-worktree` flag to `AddArgs` in `src/cli/add.rs`
- [x] 5.2 When `--reuse-worktree` is set and worktree path exists, skip the error and create session with `managed_by_aoe: false`, `cleanup_on_delete: false`
- [x] 5.3 Update error message tip to mention `--reuse-worktree` flag

## 6. Testing

- [x] 6.1 Add unit test in builder for worktree reuse path (mocked git state)
- [x] 6.2 Add TUI dialog test for two-press confirmation flow
- [x] 6.3 Run `cargo fmt`, `cargo clippy`, and `cargo test` to verify no regressions
