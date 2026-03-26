## MODIFIED Requirements

### Requirement: Skip capture-pane when window activity unchanged
The status poller and the notification monitor SHALL track the `window_activity` timestamp for each session across poll cycles. When the current `window_activity` matches the previously recorded value, the system SHALL skip `capture-pane` and reuse the previous detection result.

#### Scenario: No activity since last poll
- **WHEN** the status poller or notification monitor polls an instance
- **AND** the `window_activity` timestamp has not changed since the last poll
- **AND** at least one successful capture has been performed previously
- **THEN** the system SHALL skip `capture-pane` for that instance
- **AND** reuse the previously detected status

#### Scenario: Activity detected since last poll
- **WHEN** the status poller or notification monitor polls an instance
- **AND** the `window_activity` timestamp differs from the last recorded value
- **THEN** the system SHALL perform a full `capture-pane` and content-based detection
- **AND** update the recorded `window_activity` timestamp

#### Scenario: Periodic full check even without activity
- **WHEN** the status poller or notification monitor polls an instance with no activity change
- **AND** more than 10 seconds have elapsed since the last full capture for that instance
- **THEN** the system SHALL perform a full `capture-pane` regardless of activity
- **AND** reset the full-check timer

#### Scenario: First poll for a new instance
- **WHEN** the system polls an instance for the first time
- **AND** no previous `window_activity` is recorded
- **THEN** the system SHALL perform a full `capture-pane`
- **AND** record the current `window_activity` timestamp

### Requirement: Activity gate does not apply to hook-based agents
The activity gate SHALL be bypassed for agents that use hook-based status detection (Claude, Cursor). Hook-based detection reads a file and does not invoke `capture-pane`, so the optimization is not applicable.

#### Scenario: Hook-based agent always reads hook file
- **WHEN** the status poller or notification monitor polls a Claude or Cursor instance
- **AND** the `window_activity` timestamp has not changed
- **THEN** the system SHALL still read the hook status file
- **AND** SHALL NOT skip detection based on activity
