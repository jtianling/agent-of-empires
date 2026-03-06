# Capability Spec: Git Worktrees

**Capability**: `git-worktrees`
**Created**: 2026-03-06
**Status**: Stable

## Overview

Git worktree integration allows multiple agent sessions to work on different branches of the
same repository in parallel, each in an isolated working directory. AoE can create, track, and
clean up worktrees automatically. Two repository layouts are supported: standard repos and
bare repos (recommended for power users running many parallel agents).

## Key Entities

### WorktreeConfig (global/profile setting)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable worktree features in the TUI |
| `path_template` | `String` | `../{{repo-name}}-worktrees/{{branch}}` | Path for standard repo worktrees |
| `bare_repo_path_template` | `String` | `./{{branch}}` | Path for bare repo worktrees |
| `auto_cleanup` | `bool` | `true` | Remove worktree directory on session delete |
| `show_branch_in_tui` | `bool` | `true` | Display branch name in session list |
| `delete_branch_on_cleanup` | `bool` | `false` | Also delete the git branch on session delete |

### WorktreeInfo (per-session)

| Field | Type | Description |
|-------|------|-------------|
| `branch` | `String` | Git branch name for this worktree |
| `main_repo_path` | `String` | Path to the main/bare repository |
| `managed_by_aoe` | `bool` | Whether AoE created this worktree (controls cleanup) |
| `created_at` | `DateTime<Utc>` | When the worktree was created |
| `cleanup_on_delete` | `bool` | Whether to remove the worktree on session deletion |

## Path Templates

Templates support these variables:
- `{branch}` -- the branch name (e.g. `feat/auth`)
- `{repo-name}` -- the repository directory name

Examples:
```
Standard repo:  "../myproject-worktrees/feat-auth"   (from ../myproject-worktrees/{branch})
Bare repo:      "./feat-auth"                         (from ./{branch})
```

## Repository Layouts

### Standard Repository

```
~/projects/
  myproject/          ← main working directory (git repo)
  myproject-worktrees/
    feat-auth/        ← worktree for "feat/auth" branch
    fix-bug/          ← worktree for "fix/bug" branch
```

### Bare Repository (Recommended)

```
~/projects/
  myproject.git/      ← bare repository (no working files)
    main/             ← worktree for "main" branch
    feat-auth/        ← worktree for "feat/auth" branch
    fix-bug/          ← worktree for "fix/bug" branch
```

Bare repos keep the working directory clean; there's no "main" working copy competing for space.

## Worktree Lifecycle

```
Session Create with worktree:
  1. Resolve target path from template
  2. git worktree add <path> <branch> (create new branch if -b flag used)
  3. Record WorktreeInfo with managed_by_aoe=true
  4. Set session project_path to the worktree path

Session Delete (when managed_by_aoe=true and cleanup_on_delete=true):
  1. git worktree remove <path> --force
  2. If delete_branch_on_cleanup=true: git branch -D <branch>
```

## TUI Display

When `show_branch_in_tui` is enabled, the branch name is displayed alongside the session title.
The tmux status bar also shows the branch name for each agent session window.

## Functional Requirements

- **FR-001**: Worktree creation MUST use `git worktree add` and record the result in `WorktreeInfo`.
- **FR-002**: Worktree cleanup on session delete MUST only run when `managed_by_aoe=true` and `cleanup_on_delete=true`.
- **FR-003**: Path templates MUST support `{branch}` and `{repo-name}` variables.
- **FR-004**: The bare-repo template MUST default to `"./{branch}"` (sibling within repo dir).
- **FR-005**: Branch deletion on cleanup MUST be opt-in (`delete_branch_on_cleanup=false` default).
- **FR-006**: Worktree-based sessions MUST use the worktree path as `project_path`, not the main repo path.
- **FR-007**: The `main_repo_path` field MUST be stored to support cleanup operations even if the user navigates away.
- **FR-008**: Container sandbox volume mounts MUST use the worktree path, not the bare repo root.

## Success Criteria

- **SC-001**: 3+ agent sessions can work on different branches of the same repo simultaneously without file conflicts.
- **SC-002**: Deleting a worktree session removes the worktree directory and optionally the branch.
- **SC-003**: The TUI correctly displays the branch name for each worktree session.
- **SC-004**: Bare repo setups work with the default template without additional configuration.
