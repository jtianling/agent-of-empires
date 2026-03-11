# Capability Spec: Environment Scoping

**Capability**: `environment-scoping`
**Created**: 2026-03-12
**Status**: Draft

## Overview

Environment scoping automatically isolates AoE profiles based on execution context. Instead
of always defaulting to the `"default"` profile, the system derives an environment key from
the current working directory. This means separate project directories each get their own
isolated set of sessions and groups without manual profile switching.

## Functional Requirements

### Requirement: Environment key derivation
The system SHALL derive an environment key from the current execution context to determine which profile to use. The derivation follows this priority:
1. Explicit `--profile` flag or `AGENT_OF_EMPIRES_PROFILE` env var (highest priority)
2. Canonical working directory (always used when no explicit profile is given)

tmux session name SHALL NOT be used for profile resolution. The working directory is the sole automatic identifier, ensuring that the same directory always resolves to the same profile regardless of which tmux session it is accessed from.

#### Scenario: Explicit profile takes precedence
- **WHEN** user runs `aoe -p work`
- **THEN** profile `work` is used regardless of directory

#### Scenario: Directory-based isolation
- **WHEN** user runs `aoe` without explicit profile from `/home/user/project-a`
- **THEN** the environment key resolves to profile `auto-project-a-<hash>` where `<hash>` is the first 4 hex chars of SHA-256 of the canonical path

#### Scenario: Same directory from different tmux sessions shares profile
- **WHEN** user runs `aoe` from `/home/user/project-a` in tmux session "dev"
- **AND** user runs `aoe` from `/home/user/project-a` in tmux session "main"
- **THEN** both instances resolve to the same profile `auto-project-a-<hash>`

#### Scenario: Different directories get different profiles
- **WHEN** user runs `aoe` from `/home/user/project-a` and then from `/home/user/project-b`
- **THEN** each resolves to a different profile

### Requirement: Auto-profile creation
The system SHALL automatically create a profile when the resolved environment key maps to a non-existent profile. The auto-created profile MUST have an empty sessions list and groups list.

#### Scenario: First run in new context
- **WHEN** user runs `aoe` from `/home/user/new-project` for the first time
- **THEN** profile `auto-new-project-<hash>` is created automatically with empty sessions and groups
- **AND** the TUI launches showing an empty session list

#### Scenario: Subsequent runs reuse existing profile
- **WHEN** user runs `aoe` from `/home/user/new-project` after having previously added sessions
- **THEN** the existing `auto-new-project-<hash>` profile is loaded with all previously saved sessions

### Requirement: Auto-profile naming convention
Auto-created profiles MUST use the prefix `auto-` to distinguish them from user-created profiles. The format is `auto-<sanitized_dir_name>-<4char_hash>`, where `<4char_hash>` is the first 4 hex characters of the SHA-256 hash of the canonical directory path.

#### Scenario: Directory name with hash
- **WHEN** working directory is `/home/user/repos/my-project`
- **THEN** the auto-profile name includes both the dir name and a short hash: `auto-my-project-<hash>`

### Requirement: Migration hint for default profile
When the resolved environment key differs from `default` and the `default` profile contains sessions while the resolved profile is empty, the system SHALL display a one-time informational message directing the user to access their existing sessions.

#### Scenario: Existing user first run with new scoping
- **WHEN** user has sessions in `default` profile and runs `aoe` which resolves to `auto-myproject-a1b2`
- **AND** `auto-myproject-a1b2` has no sessions
- **THEN** a hint is displayed: "Your existing sessions are in the 'default' profile. Use `aoe -p default` to access them."

## Success Criteria

- **SC-001**: Running `aoe` from two different directories simultaneously shows different session lists.
- **SC-002**: Explicit `--profile` flag always overrides automatic scoping.
- **SC-003**: Auto-created profiles persist across restarts and are visible in `aoe profile list`.
- **SC-004**: Existing users with sessions in `default` profile receive a clear migration hint.
