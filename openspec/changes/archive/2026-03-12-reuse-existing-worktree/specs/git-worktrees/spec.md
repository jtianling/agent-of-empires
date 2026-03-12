## ADDED Requirements

### Requirement: Worktree reuse on session creation
When creating a session with a worktree branch and the computed worktree path already exists on disk, the system SHALL allow reusing the existing worktree instead of failing. Reused worktrees MUST be marked with `managed_by_aoe: false` and `cleanup_on_delete: false`.

#### Scenario: TUI first submit with existing worktree shows warning
- **WHEN** user fills in a worktree branch in the new session dialog and presses Enter, and the computed worktree path already exists
- **THEN** the dialog SHALL display a warning message indicating the worktree already exists and can be reused by pressing Enter again, and SHALL NOT create the session

#### Scenario: TUI second submit reuses existing worktree
- **WHEN** user presses Enter again after seeing the worktree reuse warning
- **THEN** the system SHALL create the session using the existing worktree path, with `managed_by_aoe: false` and `cleanup_on_delete: false`

#### Scenario: CLI reuse with flag
- **WHEN** user runs `aoe add --worktree <branch> --reuse-worktree` and the worktree path exists
- **THEN** the system SHALL create the session using the existing worktree path without error, with `managed_by_aoe: false` and `cleanup_on_delete: false`

#### Scenario: CLI without reuse flag shows updated error
- **WHEN** user runs `aoe add --worktree <branch>` without `--reuse-worktree` and the worktree path exists
- **THEN** the system SHALL display an error with a tip suggesting `--reuse-worktree`

#### Scenario: Reused worktree not cleaned up on session delete
- **WHEN** a session with a reused worktree (`managed_by_aoe: false`) is deleted
- **THEN** the system SHALL NOT remove the worktree directory or its git branch

## MODIFIED Requirements

### Requirement: Worktree creation MUST use `git worktree add` and record the result in `WorktreeInfo`.
Worktree creation MUST use `git worktree add` and record the result in `WorktreeInfo`. When the target worktree path already exists and the user has confirmed reuse (via TUI second-press or CLI `--reuse-worktree` flag), the system SHALL skip `git worktree add` and record the existing path in `WorktreeInfo` with `managed_by_aoe: false` and `cleanup_on_delete: false`.

#### Scenario: New worktree creation
- **WHEN** session is created with a worktree branch and the path does not exist
- **THEN** system SHALL run `git worktree add` and record WorktreeInfo with `managed_by_aoe: true`

#### Scenario: Existing worktree reuse
- **WHEN** session is created with a worktree branch, the path exists, and reuse is confirmed
- **THEN** system SHALL skip `git worktree add` and record WorktreeInfo with `managed_by_aoe: false` and `cleanup_on_delete: false`
