## ADDED Requirements

### Requirement: Narrow-screen detection
The TUI home screen SHALL determine whether the terminal is too narrow for the two-panel layout by checking if `available_width < list_width + 20`. This check SHALL use the user's configured (or default) `list_width` value.

#### Scenario: iPhone portrait terminal (~40 columns)
- **WHEN** the terminal width is 40 columns and list_width is 45
- **THEN** the TUI detects narrow mode (40 < 45 + 20 = 65)

#### Scenario: Normal desktop terminal (120 columns)
- **WHEN** the terminal width is 120 columns and list_width is 45
- **THEN** the TUI uses normal two-panel mode (120 >= 65)

#### Scenario: Mac split-screen (~70 columns)
- **WHEN** the terminal width is 70 columns and list_width is 45
- **THEN** the TUI uses normal two-panel mode (70 >= 65)

#### Scenario: User with custom narrow list_width
- **WHEN** the terminal width is 50 columns and list_width is 25
- **THEN** the TUI uses normal two-panel mode (50 >= 45)

### Requirement: Single-panel list rendering
When narrow mode is detected, the TUI SHALL render only the session list panel at full terminal width. The preview panel SHALL NOT be rendered.

#### Scenario: List fills full width in narrow mode
- **WHEN** narrow mode is active on a 40-column terminal
- **THEN** the session list occupies all 40 columns
- **THEN** no preview panel is visible

### Requirement: Skip preview cache in narrow mode
When narrow mode is detected, the TUI SHALL skip calling `update_caches` to avoid unnecessary tmux `capture-pane` overhead.

#### Scenario: No capture-pane in narrow mode
- **WHEN** narrow mode is active
- **THEN** `update_caches` is not called during the render loop
