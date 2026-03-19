## Context

AoE creates git worktrees for agent sessions via `GitWorktree::create_worktree()` in `src/git/mod.rs`. Git worktrees only contain tracked files -- `.gitignore`'d directories like `.claude`, `.codex`, `.gemini` are absent. These directories contain agent-specific configuration (skills, project context, instructions) that agents need to function properly.

Currently, worktree creation calls `git worktree add` and worktree removal calls `git worktree remove [--force]`. Neither handles untracked agent directories.

## Goals / Non-Goals

**Goals:**
- Automatically copy `.gitignore`'d code-agent directories from the source repo to new worktrees
- Clean up copied agent directories before worktree removal so `--force` is not needed for this reason
- Keep the solution safe: only operate on directories that are confirmed `.gitignore`'d and untracked

**Non-Goals:**
- Syncing changes back from worktree agent dirs to the source repo (one-time copy, not live sync)
- Making the list of agent directories user-configurable (hardcode a well-known list for now)
- Handling agent directories that ARE tracked by git (those are already in the worktree)
- Copying agent directories for non-AoE-managed worktrees (reused worktrees)

## Decisions

### Decision 1: Hardcoded list of well-known agent directories

Copy a fixed list of directories: `.claude`, `.codex`, `.gemini`, `.cursor`, `.aider`, `.continue`.

**Rationale**: These are the most common code-agent config directories today. A hardcoded list is simple and predictable. Adding a config option can be done later if needed (YAGNI for now).

**Alternative considered**: Scan for all hidden directories and filter by `.gitignore` status. Rejected because it could accidentally copy unrelated hidden dirs (`.vscode`, `.idea`, etc.) that users may not want duplicated.

### Decision 2: Copy only when source dir exists, is `.gitignore`'d, AND target doesn't exist

Three conditions must ALL be true before copying:
1. The agent directory exists in the source working directory
2. The directory is `.gitignore`'d (verified via `git check-ignore`)
3. The directory does NOT already exist in the worktree

**Rationale**: This is the safest approach. If a user has tracked their `.claude` dir, it's already in the worktree via git. If the worktree already has the dir (e.g., from a previous copy), we don't overwrite.

### Decision 3: Use `git check-ignore` for `.gitignore` verification

Use `git check-ignore -q <path>` to verify a path is ignored. This respects all levels of `.gitignore` (repo root, nested, global).

**Alternative considered**: Parsing `.gitignore` files manually. Rejected because gitignore rules are complex (negation, nested files, global config) and `git check-ignore` handles all edge cases.

### Decision 4: Clean up agent dirs before `git worktree remove` instead of using `--force`

Before removing a worktree, iterate through the agent directory list. For each one that exists in the worktree, verify it's `.gitignore`'d and untracked, then delete it. After cleanup, `git worktree remove` (without `--force`) should succeed since the remaining files are all tracked.

**Rationale**: Using `--force` risks deleting uncommitted tracked changes. Cleaning up only known-untracked directories is safer and more intentional.

### Decision 5: Place sync logic in `src/git/mod.rs` alongside existing worktree operations

Add two new methods to `GitWorktree`: `sync_agent_dirs_to_worktree()` and `cleanup_agent_dirs_from_worktree()`. Call them from `create_worktree()` and before `remove_worktree()` respectively.

**Alternative considered**: Placing the logic in `src/session/builder.rs`. Rejected because the operation is purely git-related and benefits from access to `GitWorktree`'s repo context. The builder can call these methods through the existing `GitWorktree` instance.

### Decision 6: Use `fs::copy` recursively (not symlinks)

Copy directories with their full contents rather than symlinking.

**Rationale**: Symlinks would break inside Docker containers where mount paths differ. A full copy ensures the worktree is self-contained. The directories are typically small (config files, not large data).

## Risks / Trade-offs

- **[Risk] Large agent directories slow down worktree creation** -> Mitigation: Agent config dirs are typically small (< 10MB). If this becomes an issue, we can add a size check or make copying opt-out. Log the copy operation so users can see what's happening.
- **[Risk] Agent directory contains absolute paths that break in the worktree** -> Mitigation: This is an agent-specific concern, not AoE's problem. Most agent configs use relative paths. Document this as a known limitation.
- **[Risk] Race condition if agent writes to dir during copy** -> Mitigation: Acceptable risk. Worktree creation happens before the agent session starts, so the source dir should be stable. Use recursive copy that tolerates partial failures.
- **[Trade-off] One-time copy vs. symlink**: Copy is safer (Docker-compatible, no cross-filesystem issues) but means changes in the source agent dir won't propagate to existing worktrees. This is acceptable because each worktree session is meant to be independent.
