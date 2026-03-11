# Capability Spec: CLI Interface

**Capability**: `cli`
**Created**: 2026-03-06
**Status**: Stable

## Overview

The `aoe` binary provides a full-featured CLI for managing sessions, profiles, groups,
worktrees, and configuration. All session lifecycle operations are available without
launching the TUI. The CLI is useful for scripting and headless environments.

## Command Tree

```
aoe                         -- launch TUI (no subcommand)
aoe add <path>              -- create and start a new session
aoe remove <id|title|path>  -- remove a session
aoe list                    -- list sessions (tabular output)
aoe status [id]             -- show session status
aoe session <subcommand>    -- session subcommands
  aoe session start <id>
  aoe session stop <id>
  aoe session restart <id>
  aoe session attach <id>
aoe profile <subcommand>    -- profile management
  aoe profile list
  aoe profile create <name>
  aoe profile delete <name>
  aoe profile rename <old> <new>
aoe group <subcommand>      -- group management
  aoe group create <name>
  aoe group delete <name>
  aoe group rename <old> <new>
aoe worktree <subcommand>   -- worktree management
  aoe worktree list
  aoe worktree cleanup
aoe tmux <subcommand>       -- tmux integration
  aoe tmux status           -- output tmux status bar content
aoe sounds <subcommand>     -- sound management
  aoe sounds list
  aoe sounds preview <name>
aoe init                    -- initialize repo config (.aoe/config.toml)
aoe uninstall               -- remove all AoE data and config
```

## `aoe add` Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--worktree <branch>` | `-w` | Create session on a git worktree |
| `--branch` | `-b` | Create new branch (used with --worktree) |
| `--sandbox` | | Enable Docker sandboxing |
| `--tool <name>` | `-t` | Agent to use (claude, opencode, vibe, codex, gemini, cursor) |
| `--yolo` | | Enable YOLO/auto-approve mode |
| `--title <name>` | | Session display title |
| `--group <path>` | | Assign to group (e.g. `work/clients`) |

## Session Resolution

CLI commands that accept a session identifier try resolution in order:
1. Exact ID match
2. ID prefix match
3. Exact title match
4. Project path match

An error is returned if no match is found.

## Output Format

`aoe list` outputs a table with columns:
- ID (truncated to 8 chars)
- Title
- Status
- Tool
- Path
- Created

`aoe status` outputs a summary of one or all sessions.

## `aoe tmux status`

Outputs a tmux status string for use in tmux `status-right` or `status-left`.
This enables the tmux status bar integration showing active session counts and states.

## `aoe init`

Creates `.aoe/config.toml` in the current directory with a commented template showing
all available repo-level config options.

## Functional Requirements

- **FR-001**: All session lifecycle operations (create, start, stop, restart, delete) MUST be available via CLI.
- **FR-002**: `aoe add` MUST create the session and start it immediately.
- **FR-003**: Session identifiers MUST support exact ID, ID prefix, exact title, and path matching.
- **FR-004**: `aoe list` output MUST be machine-parseable (consistent column format).
- **FR-005**: `aoe tmux status` MUST output text suitable for embedding in a tmux status bar.
- **FR-006**: Profile and global config flags MUST apply to CLI commands (e.g. `--profile <name>`).
- **FR-007**: `aoe uninstall` MUST prompt for confirmation before removing data.
- **FR-008**: `aoe init` MUST not overwrite an existing `.aoe/config.toml` without confirmation.

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

## Success Criteria

- **SC-001**: All TUI session operations can be scripted via the CLI.
- **SC-002**: `aoe list` output can be parsed by standard Unix tools (grep, awk).
- **SC-003**: tmux status bar integration works via `aoe tmux status`.
- **SC-004**: New repos can be initialized with a config template via `aoe init`.
