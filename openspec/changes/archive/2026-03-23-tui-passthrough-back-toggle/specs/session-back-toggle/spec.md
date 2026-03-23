## MODIFIED Requirements

### Requirement: Ctrl+b b toggles to previous session
When attached to an AoE-managed tmux session, pressing `Ctrl+b b` SHALL switch to the session the user was in before the last switch. Pressing `Ctrl+b b` again SHALL toggle back (since the switch itself records the source as the new previous session).

#### Scenario: Toggle back after number jump
- **WHEN** the user is in session #3 and presses `Ctrl+b 7 Space` to jump to session #7
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch back to session #3

#### Scenario: Toggle creates a two-session cycle
- **WHEN** the user toggles from session #7 back to session #3 via `Ctrl+b b`
- **AND** the user presses `Ctrl+b b` again
- **THEN** the system SHALL switch to session #7

#### Scenario: No previous session exists
- **WHEN** the user has just entered a session from the TUI for the first time with no prior session context (e.g., first launch or TUI opened directly)
- **AND** the user presses `Ctrl+b b`
- **THEN** no session switch SHALL occur

#### Scenario: TUI passthrough preserves back toggle
- **WHEN** the user is in Session A and detaches back to the AoE TUI via Ctrl+q
- **AND** the user selects Session B from the TUI home screen
- **AND** the user presses `Ctrl+b b`
- **THEN** the system SHALL switch to Session A

#### Scenario: TUI passthrough with same session re-entry
- **WHEN** the user is in Session A and detaches back to the AoE TUI via Ctrl+q
- **AND** the user selects Session A again from the TUI home screen
- **AND** the user presses `Ctrl+b b`
- **THEN** no session switch SHALL occur (source equals target)

#### Scenario: Previous session no longer exists
- **WHEN** the previous session has been deleted
- **AND** the user presses `Ctrl+b b`
- **THEN** no session switch SHALL occur

#### Scenario: Tmux navigation overwrites TUI-seeded previous
- **WHEN** the user detaches from Session A, enters Session B from TUI (previous = A)
- **AND** the user then navigates Session B -> Session C via Ctrl+b n
- **AND** the user presses `Ctrl+b b`
- **THEN** the system SHALL switch to Session B (not Session A)

### Requirement: TUI entry sets previous session from source context
When the user enters a session from the AoE TUI (via the home screen), the system SHALL set the `@aoe_prev_session_{client}` global option to the session the user was in before returning to the TUI, if such a source session exists and differs from the target session. If no source session exists or the source equals the target, the system SHALL unset the option (same as previous clear behavior).

#### Scenario: Previous session set from TUI source
- **WHEN** the user detaches from Session A back to the AoE TUI
- **AND** the user selects Session B from the TUI home screen
- **THEN** the @aoe_prev_session_{client} global option SHALL be set to Session A's tmux session name

#### Scenario: No source session available
- **WHEN** the user opens the AoE TUI directly (not by detaching from a managed session)
- **AND** the user selects Session B from the TUI home screen
- **THEN** the @aoe_prev_session_{client} global option SHALL be unset
- **AND** pressing Ctrl+b b SHALL NOT switch sessions

#### Scenario: Source equals target clears previous
- **WHEN** the user detaches from Session A back to the AoE TUI
- **AND** the user selects Session A again from the TUI home screen
- **THEN** the @aoe_prev_session_{client} global option SHALL be unset

#### Scenario: Subsequent tmux navigation records new previous session normally
- **WHEN** the user enters Session B from the TUI with source Session A
- **AND** the user then navigates Session B -> Session C via Ctrl+b n
- **THEN** pressing Ctrl+b b SHALL switch to Session B (newly recorded previous session)

### Requirement: TUI entry sets from-title from source context
When the user enters a session from the AoE TUI (via the home screen), the system SHALL set the `@aoe_from_title` session option on the target session to the source session's title, if such a source session exists and differs from the target. If no source session exists or the source equals the target, the system SHALL unset `@aoe_from_title`.

#### Scenario: From-title set from TUI source
- **WHEN** the user detaches from "Work Agent" (Session A) back to the AoE TUI
- **AND** the user selects "Helper Agent" (Session B) from the TUI home screen
- **THEN** the status bar in Session B SHALL show "from: Work Agent"

#### Scenario: No from-title when no source
- **WHEN** the user opens the AoE TUI directly (not by detaching from a managed session)
- **AND** the user selects a session from the TUI home screen
- **THEN** the status bar SHALL NOT display a "from:" section

#### Scenario: No from-title when re-entering same session
- **WHEN** the user detaches from Session A back to the AoE TUI
- **AND** the user selects Session A again from the TUI home screen
- **THEN** the status bar SHALL NOT display a "from:" section

## REMOVED Requirements

### Requirement: TUI entry clears stale from-title on target session
**Reason**: Replaced by "TUI entry sets from-title from source context" which sets the from-title from the source session instead of unconditionally clearing it.
**Migration**: No data migration needed. The new behavior is a superset -- it clears when no source exists, and sets when a source exists.

### Requirement: TUI entry clears stale previous-session for current client
**Reason**: Replaced by "TUI entry sets previous session from source context" which sets the previous session from the source instead of unconditionally clearing it.
**Migration**: No data migration needed. The new behavior is a superset -- it clears when no source exists, and sets when a source exists.
