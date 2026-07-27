## ADDED Requirements

### Requirement: Recovery launches each agent exactly once

When rebuilding a session during recovery, the system SHALL create it with a placeholder shell rather than an agent command, so that every recovered pane's agent is launched once, by the per-slot launch that determines what that slot runs.

Rebuilding SHALL still perform the rest of the session setup recovery depends on: worktree and sandbox preparation, on-launch hooks, and tmux options.

A placeholder rebuild SHALL NOT clear a pending fork token, because no agent has been launched to consume it, and SHALL NOT run agent startup auto-confirmation, because the pane holds a shell.

#### Scenario: Rebuilt session survives an agent that refuses to start

- **WHEN** a recoverable instance is recovered
- **AND** its agent exits immediately when launched
- **THEN** the rebuilt tmux session SHALL still exist
- **AND** each durable slot's pane SHALL have been respawned with that slot's command
- **AND** recovery SHALL NOT report that a pane could not be found

#### Scenario: Both restart modes rebuild the same way
- **WHEN** an instance is recovered in resume mode or in fresh mode
- **THEN** the session rebuild SHALL create a placeholder in both cases
- **AND** the per-slot launch SHALL remain the only place the agent command is run

#### Scenario: Pending fork survives a placeholder rebuild
- **WHEN** an instance with a pending fork token is recovered
- **THEN** the rebuild SHALL leave the token in place for the per-slot launch
