# Capability Spec: Profiles

**Capability**: `profiles`
**Created**: 2026-03-06
**Status**: Stable

## Overview

Profiles are isolated workspaces within AoE. Each profile has its own session list, group
structure, and configuration overrides. Profiles allow users to separate work by context
(e.g., "work", "personal", "client-a") without interference between session sets.

## Storage

Each profile is a directory under `<app-dir>/profiles/<name>/`:

```
~/.agent-of-empires/
  profiles/
    auto-myproject-a1b2/
      sessions.json    ← session list for this profile
      groups.json      ← group tree for this profile
      config.toml      ← profile-level config overrides
    work/
      sessions.json
      groups.json
      config.toml
    personal/
      sessions.json
      groups.json
```

Profile directories are created lazily when first accessed (e.g., on session save or instance registration).

## Profile Lifecycle

| Operation | Behavior |
|-----------|----------|
| Create | Create directory, validate name (no path separators, non-empty) |
| Delete | Remove entire profile directory. Any profile can be deleted. |
| Rename | Move directory; if renamed profile was the default, update `default_profile` in global config |
| Switch | Reload sessions and groups for the selected profile in the TUI |

## Name Constraints

- Non-empty string
- No `/` or `\` characters (no path traversal)

## Configuration Overrides

Profile configs use `*Override` structs where every field is `Option<T>`. `None` means
"inherit from global". This allows fine-grained overrides:

```toml
# profiles/work/config.toml

[sandbox]
enabled_by_default = true
default_image = "ghcr.io/mycompany/dev-sandbox:latest"

[session]
yolo_mode_default = false

[hooks]
on_launch = ["source ~/.work-env"]
```

Fields not set in the profile config fall back to the global config.

## CLI Operations

```
aoe profile list              -- list all profiles
aoe profile create <name>     -- create a new profile
aoe profile delete <name>     -- delete a profile
aoe profile rename <old> <new>
```

## TUI Operations

Profiles can be created, renamed, deleted, and switched from the TUI home screen.

## Functional Requirements

- **FR-001**: Any profile MAY be deleted, including `"default"`. There are no undeletable profiles.
- **FR-002**: Profile names MUST NOT contain `/` or `\`.
- **FR-003**: Creating a profile with an existing name MUST return an error.
- **FR-004**: Renaming a profile that is currently the default MUST update `default_profile` in global config.
- **FR-005**: Deleting a profile MUST remove all its sessions, groups, and config (the directory and all contents).
- **FR-006**: Profile config overrides MUST be `Option<T>` fields that merge with global config on load.
- **FR-007**: Switching profiles MUST immediately reload the session list and groups in the TUI.
- **FR-008**: Profile listing MUST be sorted alphabetically.

### Requirement: Profile selection default behavior
Profiles are isolated workspaces within AoE. Each profile has its own session list, group structure, and configuration overrides. When no explicit profile is specified, the system SHALL use the environment-scoped profile resolution instead of always defaulting to `"default"`.

#### Scenario: No explicit profile
- **WHEN** user runs `aoe` without `--profile`
- **THEN** the profile is resolved from the working directory via environment scoping

#### Scenario: Explicit profile still works
- **WHEN** user runs `aoe -p myprofile`
- **THEN** profile `myprofile` is used directly, bypassing environment scoping

#### Scenario: AGENT_OF_EMPIRES_PROFILE env var still works
- **WHEN** `AGENT_OF_EMPIRES_PROFILE=work` is set and no `--profile` flag is given
- **THEN** profile `work` is used directly, bypassing environment scoping

### Requirement: Auto-profile visibility in profile list
Auto-created profiles (prefixed with `auto-`) SHALL appear in `aoe profile list` output. They are regular profiles that can be renamed, deleted, or managed like any other profile. The listing SHALL NOT mark any profile as "default".

#### Scenario: Listing profiles shows auto-profiles
- **WHEN** user runs `aoe profile list` after using `aoe` in multiple contexts
- **THEN** both user-created and auto-created profiles appear in the list

### Requirement: Empty profile auto-cleanup
When the TUI exits, if the active profile contains no sessions, the system SHALL automatically delete that profile directory ONLY IF no other aoe instances are currently using the same profile.

#### Scenario: Empty profile cleaned up on exit (single instance)
- **WHEN** user exits the TUI and the active profile has zero sessions
- **AND** no other aoe instance is running in the same profile
- **THEN** the profile directory is deleted

#### Scenario: Empty profile preserved when other instances active
- **WHEN** user exits the TUI and the active profile has zero sessions
- **AND** another aoe instance is running in the same profile
- **THEN** the profile directory is preserved

#### Scenario: Non-empty profile preserved on exit
- **WHEN** user exits the TUI and the active profile has one or more sessions
- **THEN** the profile directory is preserved

### Requirement: Multi-instance tracking for profile cleanup
The system SHALL track active aoe instances per profile using PID files in `<profile_dir>/.instances/`. On startup, the system SHALL write a PID file. On exit, the system SHALL remove the PID file. Stale PID files (from crashed processes) SHALL be cleaned up on startup.

#### Scenario: Single instance exits with no sessions
- **WHEN** a single aoe instance is the only one in a profile and exits with zero sessions
- **THEN** the profile directory is deleted

#### Scenario: Multiple instances, one exits with no sessions
- **WHEN** two aoe instances are running in the same profile and one exits with zero sessions
- **THEN** the profile directory is NOT deleted because another instance is still active

#### Scenario: Last instance exits with no sessions
- **WHEN** the last remaining aoe instance in a profile exits with zero sessions
- **THEN** the profile directory is deleted

#### Scenario: Instance exits with sessions remaining
- **WHEN** an aoe instance exits but the profile still has sessions
- **THEN** the profile directory is preserved regardless of other instances

#### Scenario: Stale PID cleanup on startup
- **WHEN** aoe starts and finds PID files for processes that are no longer running
- **THEN** those stale PID files are removed before registering the new instance

### Requirement: Profile commands skip auto-profile creation
Profile management CLI commands (`aoe profile list`, `create`, `delete`, `rename`) SHALL NOT trigger automatic profile resolution or directory creation. These commands are dispatched before the `resolve_profile` startup sequence, so deleting an auto-profile from its source directory does not cause the profile to be immediately recreated.

#### Scenario: Deleting auto-profile from its source directory
- **WHEN** user runs `aoe profile delete auto-myproject-abcd` from within the `myproject` directory
- **THEN** the profile is deleted and NOT recreated by the command itself

#### Scenario: Profile list does not create directories
- **WHEN** user runs `aoe profile list` from a directory with no existing auto-profile
- **THEN** no new auto-profile directory is created as a side effect

### Requirement: No side-effect directory creation during startup
Startup operations such as migration hints and profile resolution SHALL NOT create profile directories as a side effect. Profile directories SHALL only be created when explicitly requested (e.g., `aoe profile create`) or when needed for actual data writes (e.g., session save, instance registration).

#### Scenario: Migration hint check does not create default profile
- **WHEN** aoe starts with an auto-scoped profile and no `default` profile directory exists
- **THEN** the migration hint check SHALL NOT create the `default` profile directory

#### Scenario: Migration hint check does not create resolved profile
- **WHEN** aoe starts and the resolved auto-profile directory does not yet exist
- **THEN** the migration hint check SHALL NOT create the auto-profile directory

### Requirement: New session uses current profile only
The new session dialog SHALL NOT allow switching profiles. The profile for a new session SHALL always be the current active profile. The profile field SHALL be removed from the new session dialog.

#### Scenario: Creating new session
- **WHEN** user presses `n` to create a new session
- **THEN** the new session dialog opens without a profile selection field
- **AND** the session is created in the current active profile

### Requirement: Profile deletion error surfacing
When profile deletion fails, the system SHALL display the error message to the user in the TUI instead of silently ignoring it.

#### Scenario: Deletion fails due to permissions
- **WHEN** user attempts to delete a profile and the filesystem operation fails
- **THEN** the error message is displayed in the profile picker dialog

## Success Criteria

- **SC-001**: Sessions in profile "work" are invisible when profile "personal" is active.
- **SC-002**: Profile-specific sandbox settings apply only to sessions within that profile.
- **SC-003**: Deleting a profile removes all associated sessions from storage.
- **SC-004**: Renaming the active default profile keeps it as the default.
