## 1. Constants and function renames

- [x] 1.1 Add `.agents` to `AGENT_DIRS` constant in `src/git/mod.rs`
- [x] 1.2 Add `AGENT_FILES` constant with `CLAUDE.md`, `AGENTS.md`
- [x] 1.3 Rename `sync_agent_dirs_to_worktree` to `sync_agent_config_to_worktree`
- [x] 1.4 Rename `cleanup_agent_dirs_from_worktree` to `cleanup_agent_config_from_worktree`
- [x] 1.5 Update all call sites for the renamed functions

## 2. Sync logic for files

- [x] 2.1 Add file sync loop in `sync_agent_config_to_worktree`: for each file in `AGENT_FILES`, check exists + gitignored + target absent, then `fs::copy`
- [x] 2.2 Add file cleanup loop in `cleanup_agent_config_from_worktree`: for each file in `AGENT_FILES`, check gitignored, then `fs::remove_file`

## 3. Tests

- [x] 3.1 Add test: agent config files are copied when gitignored
- [x] 3.2 Add test: agent config files are skipped when tracked
- [x] 3.3 Add test: agent config files are cleaned up on worktree deletion
- [x] 3.4 Add test: `.agents/` directory is synced when gitignored
- [x] 3.5 Verify existing tests still pass with renamed functions
