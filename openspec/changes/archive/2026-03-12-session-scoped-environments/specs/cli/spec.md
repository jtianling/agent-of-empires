## MODIFIED Requirements

### Requirement: Profile resolution for CLI commands
All session lifecycle operations (create, start, stop, restart, delete) MUST be available via CLI. Profile and global config flags MUST apply to CLI commands (e.g., `--profile <name>`). When `--profile` is not specified and `AGENT_OF_EMPIRES_PROFILE` env var is not set, the CLI SHALL use environment-scoped profile resolution to determine the active profile. This applies to all subcommands that operate on a profile: `add`, `list`, `remove`, `status`, `session`, `group`, `worktree`, and the TUI (no subcommand).

#### Scenario: CLI add respects environment scoping
- **WHEN** user runs `aoe add .` inside tmux session "project-a" without `--profile`
- **THEN** the session is added to the `auto-project-a` profile

#### Scenario: CLI list respects environment scoping
- **WHEN** user runs `aoe list` from `/home/user/project-b` outside tmux without `--profile`
- **THEN** sessions from the directory-scoped profile are listed

#### Scenario: CLI with explicit profile overrides scoping
- **WHEN** user runs `aoe -p default list` inside tmux session "project-a"
- **THEN** sessions from the `default` profile are listed, not `auto-project-a`
