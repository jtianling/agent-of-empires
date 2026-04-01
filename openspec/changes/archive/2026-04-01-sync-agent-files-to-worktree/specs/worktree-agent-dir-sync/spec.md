## ADDED Requirements

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

## MODIFIED Requirements

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
