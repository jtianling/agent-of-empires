## ADDED Requirements

### Requirement: Fan-out resume restart falls back to the instance's stored resume token

When a resume restart fans out across an instance's tracked panes, and slot 0's durable record carries no native session id, AoE SHALL resume that pane from the instance's stored resume token.

The instance's stored resume token is scraped from the primary pane's own output and is the only resume source available before a capture exists. A restart with no tracked panes already consults it; once every launched pane has a slot record from launch, the fan-out path becomes the one that runs in that window too, and without this fallback a restart that used to reattach the conversation would silently start a fresh one.

The fallback applies to slot 0 alone, which is the pane the instance's resume token describes.

#### Scenario: Slot 0 with no native session id resumes from the stored token

- **WHEN** an instance is restarted in resume mode
- **AND** slot 0's record carries no native session id
- **AND** the instance has a stored resume token
- **THEN** slot 0's pane SHALL be relaunched with that resume token

#### Scenario: A recorded native session id takes precedence

- **WHEN** an instance is restarted in resume mode
- **AND** slot 0's record carries a native session id
- **THEN** slot 0's pane SHALL resume from that native session id
- **AND** the instance's stored resume token SHALL NOT override it

#### Scenario: A fresh restart ignores the stored token

- **WHEN** an instance is restarted clean
- **AND** slot 0's record carries no native session id
- **THEN** the pane SHALL be launched with no resume token
