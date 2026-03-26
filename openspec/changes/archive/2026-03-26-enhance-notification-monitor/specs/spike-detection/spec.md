## MODIFIED Requirements

### Requirement: Confirm Running detection with a 1-second window
When the detection pipeline first detects `Running` after a non-Running state, the system SHALL hold the previous status for up to 1 second while confirming the Running signal. The status SHALL only commit to `Running` after the signal persists across at least one additional poll cycle. This applies to both the TUI status poller and the notification monitor.

#### Scenario: First Running detection enters confirmation window
- **WHEN** an instance was in a non-Running status (Idle, Waiting, Unknown)
- **AND** the current poll detects `Running`
- **THEN** the reported status SHALL remain at the previous status
- **AND** a spike confirmation window SHALL begin

#### Scenario: Running confirmed after persistence
- **WHEN** an instance is in the spike confirmation window
- **AND** the next poll also detects `Running`
- **THEN** the reported status SHALL transition to `Running`
- **AND** the spike confirmation window SHALL be cleared

#### Scenario: Running not confirmed (transient spike)
- **WHEN** an instance is in the spike confirmation window
- **AND** the next poll detects a non-Running status
- **THEN** the spike confirmation window SHALL be cleared
- **AND** the reported status SHALL remain at the non-Running status

#### Scenario: Notification monitor applies spike detection
- **WHEN** the notification monitor detects a status change for a session
- **THEN** it SHALL apply the same spike detection logic as the TUI status poller
- **AND** use its `MonitorSessionState` to track `spike_start` and `pre_spike_status`

### Requirement: Spike detection does not apply to hook-based or title fast-path
The spike detection SHALL only apply to content-based detection results. Status from hook files (Claude/Cursor) and title fast-path (spinner in title) SHALL be trusted immediately without confirmation.

#### Scenario: Hook-based Running is trusted immediately
- **WHEN** a hook status file reports `Running`
- **THEN** the status SHALL be set to `Running` without entering a spike confirmation window

#### Scenario: Title fast-path Running is trusted immediately
- **WHEN** the pane title contains a spinner character indicating Running
- **THEN** the status SHALL be set to `Running` without entering a spike confirmation window
