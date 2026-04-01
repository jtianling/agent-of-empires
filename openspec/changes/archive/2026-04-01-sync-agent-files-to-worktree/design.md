## Context

AoE's worktree system syncs `.gitignore`'d agent directories (`.claude`, `.codex`, etc.) when creating worktrees. The sync and cleanup logic lives in `src/git/mod.rs` with two constants/functions:
- `AGENT_DIRS` -- list of directory names to sync
- `sync_agent_dirs_to_worktree()` -- copies dirs on creation
- `cleanup_agent_dirs_from_worktree()` -- removes dirs before deletion

Root-level agent config files (`CLAUDE.md`, `AGENTS.md`) and the `.agents/` directory are not handled, leaving worktrees without critical AI agent instructions.

## Goals / Non-Goals

**Goals:**
- Sync `CLAUDE.md` and `AGENTS.md` to worktrees when they are gitignored
- Add `.agents/` to the agent directory list
- Clean up synced files before worktree deletion
- Same semantics as existing dir sync: exists + gitignored + target absent

**Non-Goals:**
- Configurable sync path lists (hardcoded list is sufficient)
- Symlink preservation (independent copies are fine for worktrees)
- Bidirectional sync or file watching

## Decisions

### Add `AGENT_FILES` constant parallel to `AGENT_DIRS`

Files use `fs::copy` + `fs::remove_file` while directories use `copy_dir_recursive` + `fs::remove_dir_all`. Keeping them as separate constants with parallel loops in the same functions is cleaner than trying to unify file/dir handling.

```rust
const AGENT_FILES: &[&str] = &["CLAUDE.md", "AGENTS.md"];
```

### Add `.agents/` to existing `AGENT_DIRS`

Simply append to the existing constant. No structural change needed.

### Rename functions to reflect broader scope

- `sync_agent_dirs_to_worktree` -> `sync_agent_config_to_worktree`
- `cleanup_agent_dirs_from_worktree` -> `cleanup_agent_config_from_worktree`

These are private/pub(crate) so the rename has no external impact.

### Reuse `is_gitignored()` for files

`git check-ignore -q` works identically for files and directories. No change needed.

## Risks / Trade-offs

- [Symlink `CLAUDE.md` -> `AGENTS.md`] -> Both get copied as independent files. The symlink relationship is lost in the worktree. This is acceptable -- worktrees are independent workspaces. -> No mitigation needed.
- [File size] -> `CLAUDE.md`/`AGENTS.md` are small text files. `fs::copy` is fine. -> No concern.
