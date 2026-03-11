## MODIFIED Requirements

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

## REMOVED Requirements

### Requirement: tmux session isolation
**Reason**: Replaced by directory-only isolation. tmux session names are arbitrary and do not reflect project boundaries. Using directory as the sole identifier ensures consistent behavior.
**Migration**: Existing `auto-<tmux-name>` profiles remain on disk but will no longer be auto-selected. Users can rename them or manually access with `-p`.
