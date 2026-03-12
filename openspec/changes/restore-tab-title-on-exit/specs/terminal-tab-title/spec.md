## MODIFIED Requirements

### Requirement: Terminal title is saved on TUI startup
The TUI SHALL save the current terminal title before setting its own title, by writing a CSI 22;2 t (push title) escape sequence to stdout.

#### Scenario: TUI starts with dynamic tab title enabled
- **WHEN** the TUI launches with `dynamic_tab_title` set to `true`
- **THEN** the system SHALL push the current terminal title onto the title stack before writing any AoE title

#### Scenario: TUI starts with dynamic tab title disabled
- **WHEN** the TUI launches with `dynamic_tab_title` set to `false`
- **THEN** the system SHALL NOT push any title onto the stack

### Requirement: Title is cleared on TUI exit
The TUI SHALL restore the terminal tab title to its pre-launch state during terminal teardown, using the xterm title stack mechanism (CSI 22;2 t to push, CSI 23;2 t to pop).

#### Scenario: Normal TUI exit
- **WHEN** the user quits the TUI (via `q` or `Ctrl+c`)
- **THEN** the terminal tab title SHALL be restored to the title that was active before AoE launched

#### Scenario: Panic or abnormal exit
- **WHEN** the TUI exits due to a panic (handled by the existing panic hook)
- **THEN** the terminal title restore SHALL be included in the panic cleanup sequence

#### Scenario: Dynamic tab title disabled mid-session
- **WHEN** the user disables `dynamic_tab_title` in settings while the TUI is running
- **THEN** the terminal tab title SHALL be restored to the title that was active before AoE launched
