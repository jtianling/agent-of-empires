## MODIFIED Requirements

### Requirement: Profile selection default behavior
Profiles are isolated workspaces within AoE. Each profile has its own session list, group structure, and configuration overrides. When no explicit profile is specified, the system SHALL use the environment-scoped profile resolution instead of always defaulting to `"default"`. The `"default"` profile remains as a fallback and continues to be undeletable.

#### Scenario: No explicit profile outside tmux
- **WHEN** user runs `aoe` without `--profile` and outside any tmux session
- **THEN** the profile is resolved from the working directory via environment scoping

#### Scenario: No explicit profile inside tmux
- **WHEN** user runs `aoe` without `--profile` inside a tmux session
- **THEN** the profile is resolved from the tmux session name via environment scoping

#### Scenario: Explicit profile still works
- **WHEN** user runs `aoe -p myprofile`
- **THEN** profile `myprofile` is used directly, bypassing environment scoping

#### Scenario: AGENT_OF_EMPIRES_PROFILE env var still works
- **WHEN** `AGENT_OF_EMPIRES_PROFILE=work` is set and no `--profile` flag is given
- **THEN** profile `work` is used directly, bypassing environment scoping

## ADDED Requirements

### Requirement: Auto-profile visibility in profile list
Auto-created profiles (prefixed with `auto-`) SHALL appear in `aoe profile list` output. They are regular profiles that can be renamed, deleted, or managed like any other profile.

#### Scenario: Listing profiles shows auto-profiles
- **WHEN** user runs `aoe profile list` after using `aoe` in multiple contexts
- **THEN** both user-created and auto-created profiles appear in the list
