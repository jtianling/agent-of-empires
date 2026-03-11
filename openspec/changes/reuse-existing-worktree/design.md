## Context

Currently, when creating a session with a worktree branch (`--worktree` or the TUI branch field), the system computes a target path from the branch name and path template. If that path already exists on disk, creation fails with "Worktree already exists at ...". This blocks a common workflow: running multiple agents on the same branch simultaneously.

There are three validation points that reject existing worktrees:
1. `src/session/builder.rs` line 132-134: `create_new_branch` path bails if path exists
2. `src/cli/add.rs` line 111-117: CLI pre-check bails if path exists
3. `src/git/mod.rs` line 130-131: git layer check returns `GitError::WorktreeAlreadyExists`

Note: The `!create_new_branch` path in `builder.rs` (line 95-127) already handles reuse correctly when git already tracks the worktree. The gap is the `create_new_branch` path and the CLI path where the worktree directory exists but may have been created by a previous session.

## Goals / Non-Goals

**Goals:**
- Allow multiple sessions to share the same worktree directory
- Provide a confirmation UX so users don't accidentally reuse worktrees
- Mark reused worktrees as unmanaged (`managed_by_aoe: false`, `cleanup_on_delete: false`) so one session's deletion doesn't break another

**Non-Goals:**
- Concurrent file-level locking between sessions sharing a worktree (agents are responsible for their own coordination)
- Automatically detecting other sessions using the same worktree and warning about conflicts
- Changing behavior for the non-worktree session creation path

## Decisions

### Decision 1: Two-press confirmation in TUI

When the worktree path already exists, instead of showing an error, the builder returns a structured result indicating "worktree exists, can be reused". The TUI dialog shows a warning message on first Enter press. A second Enter press submits with a `reuse_worktree: true` flag.

**Why**: This mirrors the existing `confirm_create_dir` pattern already in the dialog (for non-existent directories). Consistent UX, minimal new state.

**Alternative considered**: A separate confirmation dialog. Rejected because the inline warning pattern is already established and simpler.

### Decision 2: `reuse_worktree` flag threaded through params

Add a `reuse_worktree: bool` field to `InstanceParams` and `NewSessionData`. When true, the builder skips the "worktree already exists" check and directly reuses the path.

**Why**: Clean separation. The builder doesn't need to know about UI confirmation flow. It just checks the flag.

### Decision 3: Reused worktrees marked as unmanaged

When reusing, set `managed_by_aoe: false` and `cleanup_on_delete: false`. This prevents session deletion from removing the worktree that other sessions depend on.

**Why**: Safe default. If the worktree was created by another session, that session should control cleanup. The reusing session is a guest.

### Decision 4: CLI uses `--reuse-worktree` flag

The CLI path (`aoe add --worktree <branch>`) will accept `--reuse-worktree` to skip the existence check. Without the flag, show the existing error but with an updated tip mentioning the flag.

**Why**: CLI users expect explicit flags rather than interactive confirmation. Keeps the CLI non-interactive.

## Risks / Trade-offs

- **[Risk] Multiple sessions edit the same files** -> Mitigation: This is inherent to sharing a worktree. Users opting in accept this. Out of scope for this change.
- **[Risk] Cleanup confusion** -> Mitigation: Reused worktrees are explicitly marked `managed_by_aoe: false`. The original creator session retains cleanup responsibility.
- **[Risk] `create_new_branch` with reuse** -> Mitigation: When `reuse_worktree` is true and the path exists, skip both the existence check AND the `git worktree add` call. Just use the existing path as-is. The branch already exists since the worktree was already created.
