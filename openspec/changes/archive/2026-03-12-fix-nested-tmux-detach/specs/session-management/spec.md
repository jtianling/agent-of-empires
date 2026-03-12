## ADDED Requirements

### Requirement: Session attach configures nested detach binding
When attaching to any aoe-managed tmux session from within an existing tmux session, the attach operation SHALL configure the tmux `d` key binding to enable graceful return navigation.

#### Scenario: Agent session attach sets binding
- **WHEN** `Session::attach()` is called and TMUX env var is set
- **AND** `switch-client` succeeds
- **THEN** the tmux `d` key binding is updated to use conditional switch-back behavior

#### Scenario: Terminal session attach sets binding
- **WHEN** `TerminalSession::attach()` is called and TMUX env var is set
- **AND** `switch-client` succeeds
- **THEN** the tmux `d` key binding is updated to use conditional switch-back behavior

#### Scenario: Container terminal session attach sets binding
- **WHEN** `ContainerTerminalSession::attach()` is called and TMUX env var is set
- **AND** `switch-client` succeeds
- **THEN** the tmux `d` key binding is updated to use conditional switch-back behavior
