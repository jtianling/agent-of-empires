## ADDED Requirements

### Requirement: Graceful restart captures resume token from agent output
When an agent supports resume and the instance uses a standard (non-custom) command, the restart flow SHALL attempt a graceful exit sequence: send configured exit keys to the agent pane, wait for the process to exit, capture pane output, and extract a resume token via regex. The captured token SHALL be passed to the new agent command so the conversation context is preserved.

#### Scenario: Successful graceful restart with resume token
- **WHEN** the user presses R on a session whose agent has a `ResumeConfig`
- **AND** the instance does not use a custom command
- **THEN** the system SHALL send the configured exit key sequence to the agent pane
- **AND** wait for the pane process to exit (up to the configured timeout)
- **AND** capture the pane output and extract the resume token using the configured regex pattern
- **AND** respawn the agent pane with the resume token inserted into the command

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

### Requirement: Resume restart state machine is tick-driven
The graceful restart flow SHALL be modeled as a state machine on the Instance, driven by the TUI tick loop. The TUI SHALL remain responsive throughout the graceful exit wait.

#### Scenario: Exit keys sent in steps across ticks
- **WHEN** a graceful restart is initiated
- **THEN** the system SHALL send one group of exit keys per tick
- **AND** after all groups are sent, transition to waiting for process exit

#### Scenario: Duplicate R press during pending restart is ignored
- **WHEN** a graceful restart is already in progress for an instance
- **AND** the user presses R again on the same instance
- **THEN** the system SHALL ignore the second press

### Requirement: Restarting status provides user feedback
The system SHALL set the instance status to `Restarting` during the graceful exit flow so the UI can display appropriate feedback.

#### Scenario: Status shows Restarting during graceful exit
- **WHEN** a graceful restart is initiated
- **THEN** the instance status SHALL be `Restarting`
- **AND** when the respawn completes (with or without resume), the status SHALL transition to `Starting`

### Requirement: Send keys to agent pane
The system SHALL provide a method to send arbitrary key sequences to the agent pane via `tmux send-keys`, targeting the stored `@aoe_agent_pane`.

#### Scenario: Send keys targets the correct pane
- **WHEN** the system sends keys to the agent pane
- **AND** the session has multiple panes (user-created splits)
- **THEN** the keys SHALL be sent only to the pane identified by `@aoe_agent_pane`
- **AND** other panes SHALL not receive the keys

### Requirement: Resume token inserted into agent command
When a resume token is captured, the system SHALL insert the agent's `resume_flag` (with token substituted) into the command immediately after the binary name, before extra_args and other flags.

#### Scenario: Claude restart with resume token
- **WHEN** the system captures resume token `4dc7a3c8-934e-40c1-95f8-8b00fe11cf11` from Claude
- **THEN** the restart command SHALL include `--resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11` after the `claude` binary
- **AND** other flags (yolo, instruction, env vars) SHALL remain present and follow the resume flag

#### Scenario: Codex restart with resume token
- **WHEN** the system captures resume token `019d1af9-a899-7df1-8f7d-a244126e5ded` from Codex
- **THEN** the restart command SHALL include `resume 019d1af9-a899-7df1-8f7d-a244126e5ded` after the `codex` binary
- **AND** other flags (yolo, instruction, env vars) SHALL remain present and follow the resume subcommand
