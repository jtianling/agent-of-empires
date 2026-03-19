## Why

AoE's primary users are code agents (Claude, Codex, Gemini, etc.) that store configuration, skills, and context in hidden directories (`.claude`, `.codex`, `.gemini`). These directories are typically `.gitignore`'d because they contain user-specific or proprietary content not suitable for version control. When AoE creates a git worktree for a new session, `git worktree add` only checks out tracked files -- the ignored agent directories are missing from the new worktree, making the agent session less functional (no skills, no project context, no custom instructions).

## What Changes

- On worktree creation: detect `.claude`, `.codex`, `.gemini` (and similar code-agent hidden directories) in the source repo that are `.gitignore`'d, and copy them into the newly created worktree if they don't already exist there.
- On worktree deletion: before running `git worktree remove`, clean up any copied agent directories that are `.gitignore`'d and untracked, so that a non-force `git worktree remove` succeeds cleanly without risk of accidentally deleting user work.

## Capabilities

### New Capabilities

- `worktree-agent-dir-sync`: Automatic detection and copying of `.gitignore`'d code-agent hidden directories (`.claude`, `.codex`, `.gemini`, etc.) during worktree creation, and cleanup of those copied directories before worktree deletion.

### Modified Capabilities

- `git-worktrees`: The worktree lifecycle gains two new steps -- agent-dir copy on create and agent-dir cleanup on delete.

## Impact

- `src/git/mod.rs`: Worktree create and remove functions gain pre/post hooks for agent directory sync.
- `.gitignore` checking: New utility to verify a path is ignored by git before copying/deleting.
- No config schema changes needed -- the list of agent directories to sync can be hardcoded as a well-known constant with an option to extend via config if needed later.
- No breaking changes to existing behavior -- this is purely additive.
