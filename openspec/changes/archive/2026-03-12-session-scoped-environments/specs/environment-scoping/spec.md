## ADDED Requirements

### Requirement: Environment key derivation
The system SHALL derive an environment key from the current execution context to determine which profile to use. The derivation follows this priority:
1. Explicit `--profile` flag or `AGENT_OF_EMPIRES_PROFILE` env var (highest priority)
2. tmux session name (when `TMUX` env var is set and tmux server is reachable)
3. Canonical working directory (fallback)

#### Scenario: Explicit profile takes precedence
- **WHEN** user runs `aoe -p work`
- **THEN** profile `work` is used regardless of tmux session or directory

#### Scenario: tmux session isolation
- **WHEN** user runs `aoe` inside tmux session named "project-a"
- **THEN** the environment key resolves to profile `auto-project-a`

#### Scenario: Different tmux sessions get different environments
- **WHEN** user runs `aoe` in tmux session "project-a" and separately in tmux session "project-b"
- **THEN** each instance loads a different profile (`auto-project-a` and `auto-project-b`)

#### Scenario: Directory-based isolation outside tmux
- **WHEN** user runs `aoe` outside any tmux session from `/home/user/project-a`
- **THEN** the environment key resolves to profile `auto-project-a-<hash>` where `<hash>` is the first 4 hex chars of SHA-256 of the canonical path

#### Scenario: Same directory outside tmux shares environment
- **WHEN** user runs `aoe` twice from the same directory `/home/user/project-a` outside tmux
- **THEN** both invocations resolve to the same profile

#### Scenario: Different directories get different environments
- **WHEN** user runs `aoe` from `/home/user/project-a` and then from `/home/user/project-b`, both outside tmux
- **THEN** each resolves to a different profile

### Requirement: Auto-profile creation
The system SHALL automatically create a profile when the resolved environment key maps to a non-existent profile. The auto-created profile MUST have an empty sessions list and groups list.

#### Scenario: First run in new context
- **WHEN** user runs `aoe` in a tmux session "new-project" for the first time
- **THEN** profile `auto-new-project` is created automatically with empty sessions and groups
- **AND** the TUI launches showing an empty session list

#### Scenario: Subsequent runs reuse existing profile
- **WHEN** user runs `aoe` in tmux session "new-project" after having previously added sessions
- **THEN** the existing `auto-new-project` profile is loaded with all previously saved sessions

### Requirement: Auto-profile naming convention
Auto-created profiles MUST use the prefix `auto-` to distinguish them from user-created profiles. For tmux-based keys, the format is `auto-<sanitized_session_name>`. For directory-based keys, the format is `auto-<sanitized_dir_name>-<4char_hash>`.

#### Scenario: tmux session name sanitization
- **WHEN** tmux session is named "My Project!!"
- **THEN** the auto-profile name is `auto-my-project` (lowercase, special chars removed, spaces become hyphens)

#### Scenario: Directory name with hash
- **WHEN** working directory is `/home/user/repos/my-project`
- **THEN** the auto-profile name includes both the dir name and a short hash: `auto-my-project-<hash>`

### Requirement: tmux session name detection
When `TMUX` env var is set, the system SHALL obtain the current tmux session name by running `tmux display-message -p '#S'`. If the tmux command fails (e.g., server unreachable), the system SHALL fall back to directory-based key derivation.

#### Scenario: tmux server reachable
- **WHEN** `TMUX` env var is set and `tmux display-message` succeeds
- **THEN** the session name from tmux is used for the environment key

#### Scenario: tmux server unreachable
- **WHEN** `TMUX` env var is set but `tmux display-message` fails
- **THEN** the system falls back to directory-based key derivation

### Requirement: Migration hint for default profile
When the resolved environment key differs from `default` and the `default` profile contains sessions while the resolved profile is empty, the system SHALL display a one-time informational message directing the user to access their existing sessions.

#### Scenario: Existing user first run with new scoping
- **WHEN** user has sessions in `default` profile and runs `aoe` which resolves to `auto-myproject-a1b2`
- **AND** `auto-myproject-a1b2` has no sessions
- **THEN** a hint is displayed: "Your existing sessions are in the 'default' profile. Use `aoe -p default` to access them."
