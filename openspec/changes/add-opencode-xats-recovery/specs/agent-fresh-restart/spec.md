## ADDED Requirements

### Requirement: Shift+C clears OpenCode context and recovers xats identity

`Shift+C` SHALL restart every tracked host OpenCode pane without its previous native conversation.  Each pane SHALL create a new exact session, while a Cross Agent Team pane SHALL reuse its existing xats identity key under a newly reserved runtime generation.

#### Scenario: Fresh restart changes session but keeps identity
- **WHEN** a Cross Agent Team OpenCode slot holds session S, identity K and generation N
- **AND** the user presses `Shift+C`
- **THEN** the slot SHALL start a new session different from S
- **AND** SHALL reuse K with generation N+1

#### Scenario: Fresh restart clears each pane independently
- **WHEN** two OpenCode panes are restarted with `Shift+C`
- **THEN** each pane SHALL create its own new session
- **AND** neither pane SHALL load either previous session

#### Scenario: Old runtime callback is fenced
- **WHEN** generation N+1 has been reserved for a fresh restart
- **AND** generation N reports session ready late
- **THEN** the old callback SHALL be rejected by xats as stale
- **AND** AoE SHALL not replace the new slot session with the old session id
