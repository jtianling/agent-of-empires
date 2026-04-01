# Capability Spec: Worktree Agent Directory Sync

**Capability**: `worktree-agent-dir-sync`
**Created**: 2026-03-19
**Status**: Draft

## Overview

When AoE creates or deletes git worktrees, code-agent configuration directories (e.g., `.claude`,
`.codex`) that are `.gitignore`'d need special handling. Git worktrees do not include ignored files,
so these directories must be explicitly copied on creation and cleaned up before deletion. This
capability defines the sync and cleanup behavior and the well-known directory list.

## Requirements

### Requirement: Sync agent directories on worktree creation
When AoE creates a new worktree for a session, the system SHALL check the source working directory for well-known code-agent hidden directories (`.claude`, `.codex`, `.gemini`, `.cursor`, `.aider`, `.continue`). For each directory that exists in the source, is `.gitignore`'d, and does not already exist in the new worktree, the system SHALL copy it into the worktree.

#### Scenario: Agent directory exists and is gitignored
- **WHEN** the source repo contains `.claude/` and it is listed in `.gitignore`
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy `.claude/` to the new worktree root
- **AND** the copied directory SHALL contain the same files as the source

#### Scenario: Multiple agent directories present
- **WHEN** the source repo contains `.claude/`, `.codex/`, and `.gemini/`, all `.gitignore`'d
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy all three directories to the new worktree

#### Scenario: Agent directory already exists in worktree
- **WHEN** the source repo contains `.claude/` (`.gitignore`'d)
- **AND** the worktree already has a `.claude/` directory
- **THEN** the system SHALL NOT overwrite the existing directory

#### Scenario: Agent directory is tracked by git
- **WHEN** the source repo contains `.claude/` but it is NOT `.gitignore`'d (it is tracked)
- **AND** a new worktree is created
- **THEN** the system SHALL NOT copy `.claude/` (git already includes it in the worktree)

#### Scenario: Agent directory does not exist in source
- **WHEN** the source repo does not contain `.codex/`
- **AND** a new worktree is created
- **THEN** the system SHALL NOT attempt to copy `.codex/`

#### Scenario: Copy failure is non-fatal
- **WHEN** copying an agent directory fails (e.g., permission error)
- **THEN** the system SHALL log a warning and continue with worktree creation
- **AND** the worktree creation SHALL NOT fail due to the copy error

### Requirement: Clean up agent directories on worktree deletion
Before removing an AoE-managed worktree, the system SHALL check for well-known code-agent hidden directories in the worktree. For each directory that is `.gitignore`'d and untracked, the system SHALL delete it before running `git worktree remove`, allowing the removal to succeed without `--force`.

#### Scenario: Cleanup gitignored agent dirs before removal
- **WHEN** an AoE-managed worktree contains `.claude/` that is `.gitignore`'d
- **AND** the user deletes the session with worktree cleanup enabled
- **THEN** the system SHALL delete `.claude/` from the worktree
- **AND** THEN run `git worktree remove` without `--force`

#### Scenario: Agent directory is tracked - not cleaned up
- **WHEN** a worktree contains `.claude/` that is tracked by git (not `.gitignore`'d)
- **AND** the user deletes the session
- **THEN** the system SHALL NOT delete `.claude/` before worktree removal

#### Scenario: No agent directories in worktree
- **WHEN** a worktree contains no code-agent hidden directories
- **AND** the user deletes the session
- **THEN** the system SHALL proceed with normal `git worktree remove`

#### Scenario: Cleanup failure falls back to force removal
- **WHEN** deleting an agent directory from the worktree fails
- **THEN** the system SHALL log a warning and fall back to `git worktree remove --force`

### Requirement: Sync agent config files on worktree creation
When AoE creates a new worktree for a session, the system SHALL check the source working directory for well-known root-level agent config files (`CLAUDE.md`, `AGENTS.md`). For each file that exists in the source, is `.gitignore`'d, and does not already exist in the new worktree, the system SHALL copy it into the worktree root.

#### Scenario: Agent config file exists and is gitignored
- **WHEN** the source repo contains `CLAUDE.md` and it is listed in `.gitignore`
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy `CLAUDE.md` to the new worktree root

#### Scenario: Multiple agent config files present
- **WHEN** the source repo contains both `CLAUDE.md` and `AGENTS.md`, both `.gitignore`'d
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy both files to the new worktree

#### Scenario: Agent config file already exists in worktree
- **WHEN** the source repo contains `CLAUDE.md` (`.gitignore`'d)
- **AND** the worktree already has a `CLAUDE.md` file
- **THEN** the system SHALL NOT overwrite the existing file

#### Scenario: Agent config file is tracked by git
- **WHEN** the source repo contains `CLAUDE.md` but it is NOT `.gitignore`'d (it is tracked)
- **AND** a new worktree is created
- **THEN** the system SHALL NOT copy `CLAUDE.md` (git already includes it in the worktree)

#### Scenario: Copy failure is non-fatal
- **WHEN** copying an agent config file fails (e.g., permission error)
- **THEN** the system SHALL log a warning and continue with worktree creation
- **AND** the worktree creation SHALL NOT fail due to the copy error

### Requirement: Clean up agent config files on worktree deletion
Before removing an AoE-managed worktree, the system SHALL check for well-known agent config files in the worktree. For each file that is `.gitignore`'d, the system SHALL delete it before running `git worktree remove`.

#### Scenario: Cleanup gitignored agent config files before removal
- **WHEN** an AoE-managed worktree contains `CLAUDE.md` that is `.gitignore`'d
- **AND** the user deletes the session with worktree cleanup enabled
- **THEN** the system SHALL delete `CLAUDE.md` from the worktree
- **AND** THEN run `git worktree remove`

#### Scenario: Agent config file is tracked - not cleaned up
- **WHEN** a worktree contains `CLAUDE.md` that is tracked by git
- **AND** the user deletes the session
- **THEN** the system SHALL NOT delete `CLAUDE.md` before worktree removal

### Requirement: Well-known agent directory list
The system SHALL maintain a hardcoded list of well-known code-agent hidden directories: `.claude`, `.codex`, `.gemini`, `.cursor`, `.aider`, `.continue`, `.agents`. This list SHALL be defined as a constant and used by both the sync and cleanup operations.

#### Scenario: Only well-known directories are synced
- **WHEN** the source repo contains `.claude/` (known) and `.myagent/` (unknown), both `.gitignore`'d
- **AND** a new worktree is created
- **THEN** the system SHALL copy `.claude/` but NOT `.myagent/`

#### Scenario: .agents directory is synced
- **WHEN** the source repo contains `.agents/` and it is `.gitignore`'d
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy `.agents/` to the new worktree root
