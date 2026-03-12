## Why

When creating multiple sessions that target the same git branch, the worktree path computed from the template already exists (created by the first session). This causes the second session creation to fail with "Worktree already exists at ..." error. Users expect to be able to run multiple agents on the same branch/worktree simultaneously, which is a core use case for parallel agent workflows.

## What Changes

- When a new session specifies a worktree branch and the computed worktree path already exists, reuse the existing worktree instead of erroring out.
- In the TUI: first submit attempt shows a warning message ("Worktree already exists, press Enter again to reuse it"); second submit proceeds with reuse.
- In the CLI: add a `--reuse-worktree` flag (or auto-detect and prompt for confirmation).
- Reused worktrees are marked with `managed_by_aoe: false` and `cleanup_on_delete: false` to prevent one session's deletion from removing another session's worktree.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `git-worktrees`: Add worktree reuse behavior when the target worktree path already exists during session creation. Changes the error path to a confirmation-and-reuse flow.

## Impact

- `src/session/builder.rs`: Change the "create new branch" path to support reuse instead of bailing on existing worktree.
- `src/cli/add.rs`: Change the pre-check to allow reuse with confirmation or flag.
- `src/tui/dialogs/new_session/mod.rs`: Add confirmation state for worktree reuse warning.
- `src/tui/creation_poller.rs`: May need to pass through a "reuse worktree" flag.
- `src/git/mod.rs`: The `create_worktree` function's existing-path check may need to be relaxed when reuse is intended.
