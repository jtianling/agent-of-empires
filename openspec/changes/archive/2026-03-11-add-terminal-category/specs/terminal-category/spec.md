## ADDED Requirements

### Requirement: Terminal tool is available in tool picker
The system SHALL include "terminal" as a selectable tool in the new session dialog's tool picker. Terminal SHALL appear after "gemini" and before "cursor" in the tool list.

#### Scenario: Terminal shown in tool picker
- **WHEN** the user opens the new session dialog
- **THEN** "terminal" appears in the tool list between "gemini" and "cursor"

#### Scenario: Terminal is always available
- **WHEN** the system detects available tools at startup
- **THEN** "terminal" is always present regardless of installed binaries

### Requirement: Terminal session launches user shell
The system SHALL launch the user's default shell (`$SHELL`, falling back to `/bin/sh`) when creating a terminal session, in the specified working directory.

#### Scenario: Create terminal session with default shell
- **WHEN** the user creates a new session with tool set to "terminal"
- **THEN** a tmux session is created running the user's `$SHELL` in the specified path

#### Scenario: Shell fallback when SHELL is unset
- **WHEN** `$SHELL` is not set and the user creates a terminal session
- **THEN** the session falls back to `/bin/sh`

### Requirement: Agent-specific fields hidden for terminal
The system SHALL hide fields that do not apply to terminal sessions: YOLO Mode and Worktree/Branch.

#### Scenario: YOLO mode hidden for terminal
- **WHEN** the user selects "terminal" as the tool in the new session dialog
- **THEN** the YOLO Mode field is not displayed

#### Scenario: Worktree/Branch hidden for terminal
- **WHEN** the user selects "terminal" as the tool in the new session dialog
- **THEN** the Worktree and Branch fields are not displayed

### Requirement: Terminal has no YOLO mode
The terminal tool SHALL NOT have a YOLO/auto-approve mode configured (`yolo: None`).

#### Scenario: Terminal YOLO is None
- **WHEN** the terminal agent definition is queried for YOLO mode
- **THEN** it returns `None`

### Requirement: Terminal status detection returns Idle
The terminal tool's status detection function SHALL always return `Status::Idle`.

#### Scenario: Terminal status is always Idle
- **WHEN** status detection runs on a terminal session's pane content
- **THEN** the result is `Status::Idle`
