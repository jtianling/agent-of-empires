## MODIFIED Requirements

### Requirement: Graceful restart captures resume token from agent output
When an agent supports resume and the instance uses a standard (non-custom) command, the restart flow SHALL attempt a graceful exit sequence: send configured exit keys to the agent pane, wait for the process to exit, capture pane output, and extract a resume token via regex. The captured token SHALL be passed to the new agent command so the conversation context is preserved.

If the agent pane is already dead at restart time, the system SHALL check for a stored resume token on the Instance. If a valid stored token exists, the system SHALL use it directly for respawn without attempting the exit-key/wait/capture cycle.

#### Scenario: Successful graceful restart with resume token
- **WHEN** the user presses R on a session whose agent has a `ResumeConfig`
- **AND** the instance does not use a custom command
- **AND** the agent pane is alive
- **THEN** the system SHALL send the configured exit key sequence to the agent pane
- **AND** wait for the pane process to exit (up to the configured timeout)
- **AND** capture the pane output and extract the resume token using the configured regex pattern
- **AND** respawn the agent pane with the resume token inserted into the command

#### Scenario: Dead pane restart with stored resume token
- **WHEN** the user presses R on a session whose agent has a `ResumeConfig`
- **AND** the instance does not use a custom command
- **AND** the agent pane is already dead
- **AND** the Instance has a stored `resume_token`
- **THEN** the system SHALL respawn the agent pane using the stored resume token
- **AND** clear the stored `resume_token` after successful respawn

#### Scenario: Dead pane restart without stored resume token falls back to fresh start
- **WHEN** the user presses R on a session whose agent has a `ResumeConfig`
- **AND** the agent pane is already dead
- **AND** the Instance has no stored `resume_token`
- **THEN** the system SHALL respawn the agent pane with a fresh command (no resume token)

#### Scenario: Resume token not found in output falls back to fresh restart
- **WHEN** the graceful exit completes (pane process exits)
- **AND** the resume token regex does not match any content in the captured pane output
- **THEN** the system SHALL respawn the agent pane with a fresh command (no resume token)

#### Scenario: Graceful exit timeout falls back to fresh restart
- **WHEN** the exit key sequence has been sent
- **AND** the agent process does not exit within the configured timeout
- **THEN** the system SHALL kill the agent pane process tree
- **AND** respawn the agent pane with a fresh command (no resume token)

#### Scenario: Custom command instances skip resume
- **WHEN** the user presses R on a session that uses a custom command (`instance.command` is non-empty)
- **THEN** the system SHALL use the current kill-and-fresh-start behavior regardless of agent `ResumeConfig`

#### Scenario: Agent without ResumeConfig uses current behavior
- **WHEN** the user presses R on a session whose agent has no `ResumeConfig` (resume is `None`)
- **THEN** the system SHALL use the current kill-and-fresh-start behavior

## ADDED Requirements

### Requirement: Resume token is persisted to session storage
The Instance struct SHALL include a `resume_token: Option<String>` field that is serialized to sessions.json. The field SHALL use serde default so that old session files without this field deserialize without error.

#### Scenario: Resume token survives AoE restart
- **WHEN** the status poller captures a resume token for an instance
- **AND** AoE is closed and reopened
- **THEN** the stored resume token SHALL be available on the deserialized Instance

#### Scenario: Old sessions.json without resume_token field deserializes correctly
- **WHEN** sessions.json contains Instance entries without a `resume_token` field
- **THEN** deserialization SHALL succeed with `resume_token` set to `None`

### Requirement: Stored resume token is cleared after consumption
After a resume token is used to restart an agent (whether from stored token or live extraction), the system SHALL clear the `resume_token` field on the Instance. The token SHALL also be cleared when the agent pane is freshly started without resume.

#### Scenario: Token cleared after successful resume restart
- **WHEN** the system respawns an agent pane using a stored resume token
- **THEN** the Instance's `resume_token` SHALL be set to `None`
- **AND** sessions.json SHALL be saved with the cleared token

#### Scenario: Token cleared on fresh restart
- **WHEN** the system respawns an agent pane without a resume token (fresh start)
- **THEN** the Instance's `resume_token` SHALL be set to `None`

#### Scenario: Token cleared when new agent session starts
- **WHEN** an Instance transitions to `Starting` status via a new `start()` call
- **THEN** any previously stored `resume_token` SHALL be cleared
