## MODIFIED Requirements

### Requirement: Sync agent config files on worktree creation
When AoE creates a new worktree for a session, the system SHALL check the source working directory for well-known agent config files (`CLAUDE.md`, `AGENTS.md`, `.mcp.json`, `opencode.json`, `opencode.jsonc`, `.codex/config.toml`). For each file that exists in the source, is NOT tracked by git (either `.gitignore`'d or simply untracked), and does not already exist in the new worktree, the system SHALL copy it into the worktree at the same relative path. When the file's relative path is nested (e.g. `.codex/config.toml`), the system SHALL create the parent directory in the worktree before copying. If the source entry is a symlink, the system SHALL preserve it as a symlink rather than copying the dereferenced contents.

#### Scenario: Agent config file exists and is gitignored
- **WHEN** the source repo contains `CLAUDE.md` and it is listed in `.gitignore`
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy `CLAUDE.md` to the new worktree root

#### Scenario: Agent config file is untracked but not gitignored
- **WHEN** the source repo contains `AGENTS.md` that is neither tracked by git nor listed in `.gitignore`
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy `AGENTS.md` to the new worktree root

#### Scenario: MCP config file is synced
- **WHEN** the source repo contains `.mcp.json` (Claude Code MCP servers) that is `.gitignore`'d
- **OR** contains `opencode.json` or `opencode.jsonc` (opencode config) that is untracked
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy each such file to the new worktree root

#### Scenario: Nested agent config file is synced with parent directory created
- **WHEN** the source repo contains `.codex/config.toml` (codex config) that is untracked
- **AND** the new worktree does not already contain `.codex/config.toml`
- **THEN** the system SHALL create `.codex/` in the worktree if needed
- **AND** copy `.codex/config.toml` into it at the same relative path

#### Scenario: Agent config file is a symlink
- **WHEN** the source repo contains `CLAUDE.md` as a symlink to `AGENTS.md` (and neither is tracked)
- **AND** a new worktree is created via AoE
- **THEN** the copied `CLAUDE.md` in the worktree SHALL remain a symlink with the same link target
- **AND** the system SHALL NOT materialize the symlink into a regular file containing the dereferenced contents

#### Scenario: Multiple agent config files present
- **WHEN** the source repo contains both `CLAUDE.md` and `AGENTS.md`, both `.gitignore`'d
- **AND** a new worktree is created via AoE
- **THEN** the system SHALL copy both files to the new worktree

#### Scenario: Agent config file already exists in worktree
- **WHEN** the source repo contains `.mcp.json` (`.gitignore`'d)
- **AND** the worktree already has a `.mcp.json` file (for example, already brought in by the agent directory copy or by git)
- **THEN** the system SHALL NOT overwrite the existing file

#### Scenario: Agent config file is tracked by git
- **WHEN** the source repo contains `CLAUDE.md` but it is tracked by git
- **AND** a new worktree is created
- **THEN** the system SHALL NOT copy `CLAUDE.md` (git already includes it in the worktree)

#### Scenario: Nested config file with a symlinked ancestor is not written through the symlink
- **WHEN** the config file is nested (e.g. `.codex/config.toml`)
- **AND** an intermediate path component in the worktree (e.g. `.codex`) exists as a symlink
- **THEN** the system SHALL NOT create directories or copy the file through the symlink
- **AND** the system SHALL skip that entry and log a warning, so no file is written outside the worktree

#### Scenario: Copy failure is non-fatal
- **WHEN** copying an agent config file fails (e.g., permission error)
- **THEN** the system SHALL log a warning and continue with worktree creation
- **AND** the worktree creation SHALL NOT fail due to the copy error

### Requirement: Clean up agent config files on worktree deletion
Before removing an AoE-managed worktree, the system SHALL check for well-known agent config files (`CLAUDE.md`, `AGENTS.md`, `.mcp.json`, `opencode.json`, `opencode.jsonc`, `.codex/config.toml`) in the worktree. For each file (or symlink) that is NOT tracked by git (either `.gitignore`'d or simply untracked), the system SHALL delete it before running `git worktree remove`. This mirrors the sync gate so any config file AoE may have copied in is also cleaned up. A nested file (e.g. `.codex/config.toml`) SHALL be removed as an individual file; any resulting empty parent agent directory is handled by the agent-directory cleanup.

#### Scenario: Cleanup gitignored agent config files before removal
- **WHEN** an AoE-managed worktree contains `CLAUDE.md` that is `.gitignore`'d
- **AND** the user deletes the session with worktree cleanup enabled
- **THEN** the system SHALL delete `CLAUDE.md` from the worktree
- **AND** THEN run `git worktree remove`

#### Scenario: Cleanup gitignored MCP config files before removal
- **WHEN** an AoE-managed worktree contains `.mcp.json` (or `opencode.json`) that is `.gitignore`'d or untracked
- **AND** the user deletes the session with worktree cleanup enabled
- **THEN** the system SHALL delete each such file from the worktree before `git worktree remove`

#### Scenario: Cleanup untracked-but-not-ignored agent config files before removal
- **WHEN** an AoE-managed worktree contains `AGENTS.md` that is neither tracked by git nor listed in `.gitignore`
- **AND** the user deletes the session with worktree cleanup enabled
- **THEN** the system SHALL delete `AGENTS.md` from the worktree before `git worktree remove`

#### Scenario: Agent config file is tracked - not cleaned up
- **WHEN** a worktree contains `CLAUDE.md` that is tracked by git
- **AND** the user deletes the session
- **THEN** the system SHALL NOT delete `CLAUDE.md` before worktree removal

#### Scenario: Nested config file with a symlinked ancestor is not deleted through the symlink
- **WHEN** the worktree contains a nested config path (e.g. `.codex/config.toml`) whose intermediate component (e.g. `.codex`) is a symlink pointing outside the worktree
- **THEN** the system SHALL NOT remove the file through the symlink (which would delete data outside the worktree)
- **AND** the system SHALL leave the entry and fall back to forced worktree removal, which unlinks the symlink itself rather than its target

## ADDED Requirements

### Requirement: Well-known agent config file list
The system SHALL maintain a hardcoded list of well-known agent config files: `CLAUDE.md`, `AGENTS.md`, `.mcp.json`, `opencode.json`, `opencode.jsonc`, `.codex/config.toml`. This list SHALL be defined as a constant and used by both the sync and cleanup operations. Entries MAY be nested relative paths (a file inside an agent directory), not only repo-root files.

#### Scenario: Only well-known config files are synced
- **WHEN** the source repo contains `.mcp.json` (known) and `.myagent.json` (unknown), both `.gitignore`'d
- **AND** a new worktree is created
- **THEN** the system SHALL copy `.mcp.json` but NOT `.myagent.json`

#### Scenario: Nested codex config is synced independently of the agent directory copy
- **WHEN** the source repo `.codex/` directory is skipped by the agent-directory sync (for example because it is partially tracked by git)
- **AND** `.codex/config.toml` itself is untracked and absent from the worktree
- **THEN** the system SHALL still sync `.codex/config.toml` as a well-known agent config file
