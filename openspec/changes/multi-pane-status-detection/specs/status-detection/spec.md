## ADDED Requirements

### Requirement: Claude Code content-based status detection
The `detect_claude_status` function SHALL detect Running, Waiting, and Idle states from pane content, replacing the current stub implementation.

#### Scenario: Claude Code Running detected by spinner
- **WHEN** pane content contains braille spinner characters (U+2800..U+28FF range)
- **THEN** the detected status SHALL be Running

#### Scenario: Claude Code Running detected by streaming indicators
- **WHEN** pane content contains tool-use output patterns (e.g., "Read(", "Edit(", "Write(", "Bash(", "Grep(", "Glob(") in the last 10 lines
- **AND** no idle prompt is visible
- **THEN** the detected status SHALL be Running

#### Scenario: Claude Code Waiting detected by permission prompt
- **WHEN** pane content contains permission/approval keywords in the last 15 lines (e.g., "Allow", "Deny", "Yes", "No" as interactive choices, "approve", "reject")
- **THEN** the detected status SHALL be Waiting

#### Scenario: Claude Code Waiting detected by question prompt
- **WHEN** pane content contains a question prompt pattern (e.g., lines ending with "?", interactive selection with numbered options)
- **AND** no Running indicators are present
- **THEN** the detected status SHALL be Waiting

#### Scenario: Claude Code Idle at input prompt
- **WHEN** pane content shows the Claude Code input prompt ("> " or similar) at the bottom of the screen
- **AND** no Running or Waiting indicators are present
- **THEN** the detected status SHALL be Idle

#### Scenario: Claude Code Idle as default
- **WHEN** no Running or Waiting patterns are detected
- **THEN** the detected status SHALL be Idle (safe default)

### Requirement: Process comm name lookup utility
The process module SHALL provide a cross-platform function to look up a process's comm name by PID.

#### Scenario: Get comm name on macOS
- **WHEN** `get_process_comm(pid)` is called with a valid PID on macOS
- **THEN** it SHALL return the process comm name (e.g., "claude", "codex-aarch64-apple-darwin", "zsh")

#### Scenario: Get comm name on Linux
- **WHEN** `get_process_comm(pid)` is called with a valid PID on Linux
- **THEN** it SHALL return the process comm name from `/proc/<pid>/comm`

#### Scenario: Process does not exist
- **WHEN** `get_process_comm(pid)` is called with a non-existent PID
- **THEN** it SHALL return None
