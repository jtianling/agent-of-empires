## MODIFIED Requirements

### Requirement: Terminal tab title reflects TUI state
When the AoE TUI session itself is active, AoE SHALL follow the `ddba37c` behavior and SHALL NOT manage the outer terminal title. Specifically, the TUI SHALL NOT write OSC 0 title escape sequences, SHALL NOT push or pop the xterm title stack, and SHALL NOT mutate tmux `set-titles` options as part of TUI startup or teardown.

#### Scenario: TUI launches inside tmux
- **WHEN** the TUI launches
- **THEN** AoE SHALL NOT call tmux `set-option -g set-titles ...`
- **AND** AoE SHALL NOT call tmux `set-option -g set-titles-string ...`

#### Scenario: TUI state changes while AoE session is active
- **WHEN** the user opens dialogs, settings, diff view, or returns to the home screen
- **THEN** AoE SHALL NOT write an OSC 0 terminal title escape sequence for those state changes

#### Scenario: TUI exits or panics
- **WHEN** the TUI exits normally or through the panic cleanup path
- **THEN** AoE SHALL NOT emit xterm title-stack restore sequences as part of teardown
