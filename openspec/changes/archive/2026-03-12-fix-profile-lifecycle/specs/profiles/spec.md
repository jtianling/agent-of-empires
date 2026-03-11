## ADDED Requirements

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

## MODIFIED Requirements

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
