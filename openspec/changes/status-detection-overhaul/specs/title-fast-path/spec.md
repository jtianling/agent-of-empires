## ADDED Requirements

### Requirement: Detect Running status from pane title spinner characters
The status detection pipeline SHALL check the tmux pane title for Braille spinner characters (U+2800-U+28FF range) before falling back to content-based detection. If a spinner character is found in the pane title, the detection SHALL return `Running` immediately without calling `capture-pane`.

#### Scenario: Pane title contains Braille spinner
- **WHEN** the status detection checks an instance's pane title
- **AND** the title contains one or more Braille spinner characters (e.g., U+280B, U+2819, U+2839)
- **THEN** the detection SHALL return `Status::Running`
- **AND** SHALL NOT call `capture-pane`

#### Scenario: Pane title has no spinner
- **WHEN** the status detection checks an instance's pane title
- **AND** the title contains no Braille spinner characters
- **THEN** the detection SHALL fall through to the next detection layer (activity gate or content-based)

#### Scenario: Pane title contains done marker
- **WHEN** the status detection checks an instance's pane title
- **AND** the title contains a done/completion marker character (e.g., checkmark) but no spinner
- **THEN** the detection SHALL NOT return Running from the title fast path
- **AND** SHALL fall through to content-based detection

### Requirement: Title fast-path uses batch-cached pane info
The title fast-path SHALL read the pane title from the batch pane info cache, not from a separate tmux subprocess call. This ensures zero additional cost for the title check.

#### Scenario: Title read from cache
- **WHEN** the title fast-path checks a pane title
- **THEN** it SHALL read the title from the `PaneInfoCache` populated by the batch query
- **AND** SHALL NOT spawn a tmux subprocess to query the title
