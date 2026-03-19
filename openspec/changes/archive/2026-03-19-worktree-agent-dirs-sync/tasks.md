## 1. Constants and Utilities

- [x] 1.1 Add `AGENT_DIRS` constant to `src/git/mod.rs` with the well-known agent directory list: `.claude`, `.codex`, `.gemini`, `.cursor`, `.aider`, `.continue`
- [x] 1.2 Add helper function `is_gitignored(repo_path: &Path, dir_name: &str) -> bool` that uses `git check-ignore -q` to verify a path is ignored

## 2. Sync on Worktree Creation

- [x] 2.1 Implement `GitWorktree::sync_agent_dirs_to_worktree(source_dir: &Path, worktree_dir: &Path)` that iterates `AGENT_DIRS`, checks each exists in source, is gitignored, and not present in worktree, then recursively copies
- [x] 2.2 Add recursive directory copy helper function (`copy_dir_recursive`) in `src/git/mod.rs`
- [x] 2.3 Call `sync_agent_dirs_to_worktree` at the end of `create_worktree()`, after `convert_git_file_to_relative()` and before returning `Ok(())`

## 3. Cleanup on Worktree Deletion

- [x] 3.1 Implement `GitWorktree::cleanup_agent_dirs_from_worktree(worktree_dir: &Path) -> bool` that iterates `AGENT_DIRS`, checks each is gitignored and untracked, deletes them, returns whether all cleanups succeeded
- [x] 3.2 Modify worktree removal flow: call `cleanup_agent_dirs_from_worktree` before `git worktree remove`. If cleanup succeeded, use non-force remove; if cleanup failed, fall back to force remove

## 4. Tests

- [x] 4.1 Unit test: `sync_agent_dirs_to_worktree` copies gitignored agent dirs and skips tracked/missing/already-existing ones
- [x] 4.2 Unit test: `cleanup_agent_dirs_from_worktree` removes gitignored agent dirs and skips tracked ones
- [x] 4.3 Unit test: `is_gitignored` correctly identifies ignored vs tracked paths
- [x] 4.4 Integration test: full worktree create -> verify agent dirs copied -> delete -> verify agent dirs cleaned up
