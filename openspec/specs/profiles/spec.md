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
    default/
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

The active profile is tracked in the global `config.toml` as `default_profile`.

## Profile Lifecycle

| Operation | Behavior |
|-----------|----------|
| Create | Create directory, validate name (no path separators, non-empty) |
| Delete | Remove entire profile directory; cannot delete `"default"` |
| Rename | Move directory; if renamed profile was the default, update `default_profile` in global config |
| Switch | Update `default_profile` in global config; reload sessions and groups |

## Name Constraints

- Non-empty string
- No `/` or `\` characters (no path traversal)
- The name `"default"` is reserved and cannot be deleted

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
aoe profile delete <name>     -- delete a profile (not "default")
aoe profile rename <old> <new>
aoe profile set-default <name>
```

## TUI Operations

Profiles can be created, renamed, deleted, and switched from the TUI home screen.

## Functional Requirements

- **FR-001**: The `"default"` profile MUST always exist and MUST NOT be deletable.
- **FR-002**: Profile names MUST NOT contain `/` or `\`.
- **FR-003**: Creating a profile with an existing name MUST return an error.
- **FR-004**: Renaming a profile that is currently the default MUST update `default_profile` in global config.
- **FR-005**: Deleting a profile MUST remove all its sessions, groups, and config (the directory and all contents).
- **FR-006**: Profile config overrides MUST be `Option<T>` fields that merge with global config on load.
- **FR-007**: Switching profiles MUST immediately reload the session list and groups in the TUI.
- **FR-008**: Profile listing MUST be sorted alphabetically.

## Success Criteria

- **SC-001**: Sessions in profile "work" are invisible when profile "personal" is active.
- **SC-002**: Profile-specific sandbox settings apply only to sessions within that profile.
- **SC-003**: Deleting a profile removes all associated sessions from storage.
- **SC-004**: Renaming the active default profile keeps it as the default.
