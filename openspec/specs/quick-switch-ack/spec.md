## Purpose

Quick-switch acknowledgment enables automatic Waiting status acknowledgment when users switch to a session via any quick-switch mechanism (number jump, root-key cycle, or back toggle), removing the need for dedicated notification keybindings.

## Requirements

### Requirement: Quick-switch writes ack signal for target session
When any quick-switch function (`switch_aoe_session`, `switch_aoe_session_by_index`, `switch_aoe_session_back`) successfully switches to a target session, the system SHALL write the target session's instance_id to the ack signal file. This allows the notification monitor to acknowledge the session's Waiting status on the next poll cycle.

#### Scenario: Number jump acknowledges Waiting session
- **WHEN** the user presses Ctrl+b 3 Space to jump to session #3
- **AND** session #3 is in Waiting status
- **AND** the switch succeeds
- **THEN** the system SHALL write session #3's instance_id to the ack signal file
- **AND** on the next monitor poll, session #3's status SHALL be downgraded from Waiting to Idle

#### Scenario: Root-key cycle acknowledges Waiting session
- **WHEN** the user presses Ctrl+. to cycle to the next session
- **AND** the target session is in Waiting status
- **AND** the switch succeeds
- **THEN** the system SHALL write the target session's instance_id to the ack signal file

#### Scenario: Back toggle acknowledges Waiting session
- **WHEN** the user presses Ctrl+b b to toggle back to the previous session
- **AND** the previous session is in Waiting status
- **AND** the switch succeeds
- **THEN** the system SHALL write the previous session's instance_id to the ack signal file

#### Scenario: Switch to non-Waiting session still writes ack signal
- **WHEN** the user switches to a session that is Running or Idle
- **AND** the switch succeeds
- **THEN** the system SHALL still write the target session's instance_id to the ack signal file
- **AND** the monitor SHALL set acknowledged=true but the status mapping is unaffected (Running stays Running, Idle stays Idle)

#### Scenario: Switch failure does not write ack signal
- **WHEN** the switch-client command fails (e.g., target session no longer exists)
- **THEN** the system SHALL NOT write to the ack signal file

### Requirement: Resolve instance_id from loaded instances
The quick-switch functions SHALL resolve the target session's instance_id by matching the target session name against `Session::generate_name(&instance.id, &instance.title)` for each loaded instance. If no match is found, the ack signal write SHALL be silently skipped.

#### Scenario: Instance found for target session
- **WHEN** the target session name is "aoe_my_agent_abcd1234"
- **AND** an instance exists where `Session::generate_name(id, title)` produces "aoe_my_agent_abcd1234"
- **THEN** the system SHALL use that instance's id for the ack signal

#### Scenario: No instance matches target session
- **WHEN** the target session name does not match any loaded instance
- **THEN** the system SHALL skip the ack signal write without error
- **AND** the switch SHALL still proceed normally

### Requirement: Ack signal write is a public utility
The notification monitor module SHALL expose a public `write_ack_signal(instance_id: &str)` function that writes the given instance_id to the ack signal file. The quick-switch functions in `utils.rs` SHALL call this function. The signal file path and format SHALL remain unchanged from the existing `ack_signal_path()` mechanism.

#### Scenario: write_ack_signal creates signal file
- **WHEN** `write_ack_signal("abc123")` is called
- **THEN** the ack signal file SHALL contain "abc123"
- **AND** the next call to `take_ack_signal()` SHALL return "abc123"

#### Scenario: write_ack_signal overwrites previous signal
- **WHEN** `write_ack_signal("abc123")` is called
- **AND** `write_ack_signal("def456")` is called before the monitor polls
- **THEN** `take_ack_signal()` SHALL return "def456"
