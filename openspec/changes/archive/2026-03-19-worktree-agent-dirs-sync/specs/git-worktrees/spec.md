## MODIFIED Requirements

### Requirement: Worktree creation lifecycle
When AoE creates a worktree, the system SHALL execute the following steps in order:
1. Resolve target path from template
2. `git worktree add <path> <branch>` (create new branch if `-b` flag used)
3. Convert `.git` file to relative path
4. Sync `.gitignore`'d code-agent directories from source repo to worktree
5. Record WorktreeInfo with `managed_by_aoe=true`
6. Set session `project_path` to the worktree path

#### Scenario: Worktree creation includes agent dir sync
- **WHEN** a session is created with a worktree
- **AND** the source repo has `.gitignore`'d agent directories
- **THEN** the system SHALL copy agent directories after `git worktree add` completes
- **AND** before recording WorktreeInfo

### Requirement: Worktree deletion lifecycle
When AoE deletes a managed worktree (with `cleanup_on_delete=true`), the system SHALL execute the following steps in order:
1. Clean up `.gitignore`'d code-agent directories from the worktree
2. `git worktree remove <path>` (without `--force`, unless cleanup failed)
3. If `delete_branch_on_cleanup=true`: `git branch -D <branch>`

#### Scenario: Worktree deletion cleans agent dirs first
- **WHEN** a managed session with worktree is deleted
- **THEN** the system SHALL remove agent directories before calling `git worktree remove`
- **AND** `git worktree remove` SHALL be called without `--force` when agent dir cleanup succeeds
