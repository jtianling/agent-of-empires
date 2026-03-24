## ADDED Requirements

### Requirement: Status poller captures resume token on pane death transition
When the status poller detects that an agent pane has transitioned from a non-error status to dead (Error), it SHALL attempt to extract a resume token from the pane output using the agent's configured `resume_pattern`. The extracted token SHALL be included in the `StatusUpdate` message sent back to the TUI.

#### Scenario: Resume token captured on first pane death detection
- **WHEN** the status poller polls an instance whose previous status was not `Error`
- **AND** the current poll detects the pane is dead (status transitions to `Error`)
- **AND** the agent has a `ResumeConfig` with a `resume_pattern`
- **THEN** the poller SHALL capture pane output and extract the resume token
- **AND** include the token in the `StatusUpdate` for that instance

#### Scenario: No token captured for agent without ResumeConfig
- **WHEN** the status poller detects a pane death transition
- **AND** the agent has no `ResumeConfig`
- **THEN** the `StatusUpdate` SHALL have `resume_token` set to `None`

#### Scenario: No token captured on subsequent polls of dead pane
- **WHEN** the status poller polls an instance whose previous status was already `Error`
- **AND** the pane is still dead
- **THEN** the poller SHALL NOT attempt to extract a resume token
- **AND** `resume_token` in the `StatusUpdate` SHALL be `None`

#### Scenario: Invalid token extracted is discarded
- **WHEN** the poller extracts a resume token that fails validation (non-hex/dash characters)
- **THEN** the `StatusUpdate` SHALL have `resume_token` set to `None`

### Requirement: StatusUpdate includes optional resume token field
The `StatusUpdate` struct SHALL include a `resume_token: Option<String>` field to carry captured resume tokens from the background poller thread to the TUI event loop.

#### Scenario: TUI applies resume token from status update
- **WHEN** the TUI receives a `StatusUpdate` with a non-None `resume_token`
- **THEN** it SHALL store the token on the corresponding Instance's `resume_token` field
- **AND** trigger a session save to persist the token

#### Scenario: StatusUpdate without token does not overwrite existing stored token
- **WHEN** the TUI receives a `StatusUpdate` with `resume_token` set to `None`
- **AND** the Instance already has a stored `resume_token`
- **THEN** the existing stored token SHALL NOT be overwritten

### Requirement: Status poller tracks previous status for transition detection
The status poller SHALL maintain a map of previous statuses per instance to distinguish first-time pane death (transition) from ongoing dead state (already known).

#### Scenario: Status map updated after each poll
- **WHEN** the status poller completes a poll cycle for an instance
- **THEN** it SHALL record the detected status in its previous-status map
- **AND** use this map on the next poll to determine if a transition occurred
