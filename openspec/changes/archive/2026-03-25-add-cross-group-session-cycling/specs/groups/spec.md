## ADDED Requirements

### Requirement: Status bar shows accurate keybinding hints
The tmux status bar SHALL display `Ctrl+b 1-9 space jump` instead of `Ctrl+b 1-9 jump` to
accurately reflect the Space confirmation step. The `Ctrl+b n/p switch` hint SHALL be removed
from the status bar.

#### Scenario: Status bar after attach
- **WHEN** a session is attached and the status bar is configured
- **THEN** the status-left SHALL contain `Ctrl+b 1-9 space jump`
- **AND** the status-left SHALL NOT contain `n/p`
