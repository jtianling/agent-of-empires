## ADDED Requirements

### Requirement: TUI entry clears stale from-title on target session
When the user enters a session from the AoE TUI (via the home screen), the system SHALL unset the `@aoe_from_title` session option on the target session before attaching. This prevents the status bar from displaying a stale "from:" label that was left over from a previous tmux-level navigation.

#### Scenario: From-title cleared when entering from TUI
- **WHEN** the user navigates Session A -> Session B via Ctrl+b n (setting @aoe_from_title on Session B)
- **AND** the user returns to the AoE TUI via Ctrl+q
- **AND** the user selects Session B from the TUI home screen
- **THEN** the @aoe_from_title option on Session B SHALL be unset
- **AND** the status bar SHALL NOT display a "from:" section

#### Scenario: From-title cleared when entering a different session from TUI
- **WHEN** the user navigates Session A -> Session B via Ctrl+b n (setting @aoe_from_title on Session B)
- **AND** the user returns to the AoE TUI via Ctrl+q
- **AND** the user selects Session C from the TUI home screen
- **THEN** the @aoe_from_title option on Session C SHALL be unset (if it had a stale value)
- **AND** the status bar SHALL NOT display a "from:" section

### Requirement: TUI entry clears stale previous-session for current client
When the user enters a session from the AoE TUI (via the home screen), the system SHALL unset the `@aoe_prev_session_{client}` global option for the current tmux client. This prevents Ctrl+b b from jumping to a session that the user did not navigate from in the current context.

#### Scenario: Previous session cleared when entering from TUI
- **WHEN** the user navigates Session A -> Session B via Ctrl+b n (recording @aoe_prev_session for the client)
- **AND** the user returns to the AoE TUI via Ctrl+q
- **AND** the user selects Session C from the TUI home screen
- **THEN** the @aoe_prev_session_{client} global option SHALL be unset
- **AND** pressing Ctrl+b b SHALL NOT switch sessions (no previous session recorded)

#### Scenario: Subsequent tmux navigation records new previous session normally
- **WHEN** the user enters Session C from the TUI (clearing stale previous session)
- **AND** the user then navigates Session C -> Session D via Ctrl+b n
- **THEN** pressing Ctrl+b b SHALL switch to Session C (newly recorded previous session)

### Requirement: Public helper functions for clearing session context
The tmux utilities module SHALL expose two public functions for clearing session navigation context:
- `clear_from_title(session_name: &str)`: unsets the `@aoe_from_title` session option on the given session.
- `clear_previous_session_for_client(client_name: &str)`: unsets the `@aoe_prev_session_{client}` global option for the given client.

#### Scenario: clear_from_title removes the option
- **WHEN** `clear_from_title("aoe_my_session")` is called
- **THEN** the tmux session option `@aoe_from_title` on session `aoe_my_session` SHALL be unset

#### Scenario: clear_previous_session_for_client removes the option
- **WHEN** `clear_previous_session_for_client("/dev/ttys003")` is called
- **THEN** the tmux global option `@aoe_prev_session_/dev/ttys003` (sanitized) SHALL be unset

## MODIFIED Requirements

### Requirement: No previous session exists
When the user has just entered a session from the TUI (no prior tmux-level switch in this context), pressing Ctrl+b b SHALL NOT switch sessions. The TUI entry path clears any stale previous-session state, so entering from the TUI always starts with a clean navigation context.

#### Scenario: No previous session exists
- **WHEN** the user has just entered a session from the TUI for the first time (no prior switch)
- **AND** the user presses `Ctrl+b b`
- **THEN** no session switch SHALL occur

#### Scenario: Stale previous session cleared on TUI entry
- **WHEN** the user previously navigated Session A -> Session B via Ctrl+b n
- **AND** the user returns to the TUI and enters Session B again
- **AND** the user presses `Ctrl+b b`
- **THEN** no session switch SHALL occur (stale previous session was cleared on TUI entry)
