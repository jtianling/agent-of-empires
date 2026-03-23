## MODIFIED Requirements

### Requirement: Force-restart running agent pane
When the agent pane is alive and the user triggers restart via R keybinding, the system SHALL attempt a graceful resume restart if the agent supports it and the instance does not use a custom command. If the agent does not support resume, or the instance uses a custom command, the system SHALL use the existing kill-and-respawn behavior.

#### Scenario: Force-restart running agent pane
- **WHEN** the agent pane is alive (process running)
- **AND** the user triggers agent pane restart via `R` keybinding
- **AND** the agent has a `ResumeConfig`
- **AND** the instance does not use a custom command
- **THEN** the system SHALL initiate the graceful resume restart flow
- **AND** set the instance status to `Restarting`

#### Scenario: Force-restart without resume support
- **WHEN** the agent pane is alive (process running)
- **AND** the user triggers agent pane restart via `R` keybinding
- **AND** the agent has no `ResumeConfig` OR the instance uses a custom command
- **THEN** the system SHALL kill the agent pane's process tree
- **AND** respawn the pane with the original agent command
- **AND** all user-created panes SHALL be preserved
