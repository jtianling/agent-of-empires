# Capability Spec: Pane CWD Inherit

**Capability**: `pane-cwd-inherit`
**Created**: 2026-03-30
**Status**: Draft

## Purpose

When splitting panes in AoE-managed tmux sessions, new panes should inherit the project's working directory rather than the home directory or wherever the tmux server was started. This is achieved by storing the project path as a session-scoped tmux option (`@aoe_project_path`) and using guarded split-window bindings that read this option for AoE sessions while preserving default behavior for non-AoE sessions.

## Requirements

### Requirement: Session stores project path as tmux option
When AoE creates a tmux session for an agent, the system SHALL store the session's project working directory as the `@aoe_project_path` session-scoped tmux option on the created session.

#### Scenario: Project path stored during session creation
- **WHEN** AoE creates a new tmux session for an agent with project path `/home/user/my-project`
- **THEN** the session SHALL have `@aoe_project_path` set to `/home/user/my-project`

#### Scenario: Project path stored atomically with session creation
- **WHEN** AoE creates a new tmux session
- **THEN** the `@aoe_project_path` option SHALL be set in the same tmux command chain as `new-session` (not as a separate command)

### Requirement: Horizontal split inherits project path in AoE sessions
When a user presses `Ctrl+b %` (horizontal split) while attached to an AoE-managed session, the new pane's working directory SHALL be the session's `@aoe_project_path`.

#### Scenario: Horizontal split in AoE session uses project path
- **WHEN** the user is attached to an AoE-managed session with `@aoe_project_path` set to `/home/user/my-project`
- **AND** the user presses `Ctrl+b %`
- **THEN** the new pane SHALL open with working directory `/home/user/my-project`

#### Scenario: Horizontal split in non-AoE session uses default behavior
- **WHEN** the user is attached to a non-AoE tmux session (name does not start with `aoe_`)
- **AND** the user presses `Ctrl+b %`
- **THEN** the new pane SHALL open with tmux's default split-window behavior (no `-c` override)

### Requirement: Vertical split inherits project path in AoE sessions
When a user presses `Ctrl+b "` (vertical split) while attached to an AoE-managed session, the new pane's working directory SHALL be the session's `@aoe_project_path`.

#### Scenario: Vertical split in AoE session uses project path
- **WHEN** the user is attached to an AoE-managed session with `@aoe_project_path` set to `/home/user/my-project`
- **AND** the user presses `Ctrl+b "`
- **THEN** the new pane SHALL open with working directory `/home/user/my-project`

#### Scenario: Vertical split in non-AoE session uses default behavior
- **WHEN** the user is attached to a non-AoE tmux session (name does not start with `aoe_`)
- **AND** the user presses `Ctrl+b "`
- **THEN** the new pane SHALL open with tmux's default split-window behavior (no `-c` override)

### Requirement: Split bindings restored on cleanup
When AoE cleans up keybindings on exit, the `%` and `"` bindings SHALL be restored to tmux default behavior (plain `split-window -h` and `split-window -v` respectively), not simply unbound.

#### Scenario: Bindings restored after cleanup
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** `%` SHALL be bound to `split-window -h` in the prefix table
- **AND** `"` SHALL be bound to `split-window -v` in the prefix table

### Requirement: Existing sessions backfilled with project path
When AoE sets up keybindings, sessions created before this feature SHALL have `@aoe_project_path` set from their stored instance data, so they gain the split-pane working directory behavior without recreation.

#### Scenario: Older session receives project path on next launch
- **WHEN** AoE launches and an existing session `aoe_myagent_abc123` exists without `@aoe_project_path`
- **AND** the stored instance for that session has project path `/home/user/old-project`
- **THEN** `@aoe_project_path` SHALL be set to `/home/user/old-project` on that session
