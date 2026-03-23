# Capability Spec: Agent Pane Restart

**Capability**: `agent-pane-restart`
**Created**: 2026-03-20
**Status**: Draft

## Purpose

Supports restarting only the AoE-managed agent pane within a tmux session, preserving the session layout and all user-created panes. Uses a reusable command builder and scoped process cleanup to ensure restarts are safe and consistent.

## Requirements

### Requirement: Respawn agent pane without destroying session
The system SHALL support restarting only the AoE-managed agent pane within a tmux session, preserving the session layout and all user-created panes. The respawn SHALL use `tmux respawn-pane -k -t <pane_id>` targeting the stored `@aoe_agent_pane`.

#### Scenario: Respawn dead agent pane in multi-pane session
- **WHEN** the agent pane is dead (process exited)
- **AND** the session has user-created split panes
- **AND** the user triggers agent pane restart
- **THEN** the system SHALL respawn only the agent pane with the original agent command
- **AND** all user-created panes and the session layout SHALL be preserved
- **AND** the session status SHALL transition to `Starting`

#### Scenario: Force-restart running agent pane
- **WHEN** the agent pane is alive (process running)
- **AND** the user triggers agent pane restart via `R` keybinding
- **AND** the agent has a `ResumeConfig`
- **AND** the instance does not use a custom command
- **THEN** the system SHALL initiate the graceful resume restart flow
- **AND** set the instance status to `Restarting`

#### Scenario: Force-restart without resume support
- **WHEN** the agent pane is alive (process running)
- **AND** the user triggers agent pane restart via `R` keybinding
- **AND** the agent has no `ResumeConfig` OR the instance uses a custom command
- **THEN** the system SHALL kill the agent pane's process tree
- **AND** respawn the pane with the original agent command
- **AND** all user-created panes SHALL be preserved

#### Scenario: Respawn single-pane session falls back to full restart
- **WHEN** the agent pane is dead
- **AND** the session has only 1 pane (no user-created splits)
- **AND** the restart is triggered via attach-time recovery (not `R` keybinding)
- **THEN** the system SHALL use the existing kill-session + recreate flow

#### Scenario: Respawn re-runs on-launch hooks
- **WHEN** an agent pane is respawned
- **THEN** the system SHALL execute on-launch hooks before starting the agent
- **AND** the hooks SHALL run in the same manner as during initial session creation

#### Scenario: Respawn re-applies tmux session options
- **WHEN** an agent pane is respawned
- **THEN** the system SHALL call `apply_tmux_options()` after respawn
- **AND** tmux options (status bar, mouse, etc.) SHALL remain correct

### Requirement: Agent launch command is reusable
The agent launch command construction (binary, extra_args, yolo flags, env vars, custom instruction) SHALL be extracted into a reusable method so both initial session creation and pane respawn can share the same command-building logic.

#### Scenario: Respawn uses same command as initial start
- **WHEN** an agent pane is respawned
- **THEN** the respawn command SHALL be identical to what `start_with_size_opts()` would produce for the same instance configuration
- **AND** env vars, yolo flags, and custom instructions SHALL all be applied

### Requirement: Scoped process cleanup for respawn
When respawning the agent pane, the system SHALL only kill the process tree of the agent pane, not processes in user-created panes.

#### Scenario: Process cleanup targets only agent pane
- **WHEN** the system respawns the agent pane
- **AND** user-created panes have running processes
- **THEN** only the agent pane's process tree SHALL be terminated
- **AND** processes in user-created panes SHALL not be affected
