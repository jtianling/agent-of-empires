## ADDED Requirements

### Requirement: PathGhostCompletion computes filesystem directory completions
The `PathGhostCompletion` component SHALL compute ghost text suggestions by scanning the filesystem for matching directories based on the current input value and cursor position. It SHALL only produce a suggestion when the cursor is at the end of the input.

#### Scenario: Single directory match produces ghost with trailing slash
- **WHEN** the input value is `/home/user/pro` with cursor at end
- **AND** the filesystem contains directory `/home/user/projects` and no other `pro*` directories
- **THEN** `PathGhostCompletion::compute()` SHALL return ghost text `jects/`

#### Scenario: Multiple matches with common prefix extension
- **WHEN** the input value is `/home/user/d` with cursor at end
- **AND** the filesystem contains directories `/home/user/docs` and `/home/user/downloads`
- **THEN** `PathGhostCompletion::compute()` SHALL return ghost text `o` (the common prefix extension)

#### Scenario: Multiple matches with no common prefix extension
- **WHEN** the input value is `/home/user/do` with cursor at end
- **AND** the filesystem contains directories `/home/user/docs` and `/home/user/downloads`
- **THEN** `PathGhostCompletion::compute()` SHALL return ghost text `cs/` (first sorted match remainder with trailing slash)

#### Scenario: No matching directories
- **WHEN** the input value is `/home/user/zzz` with cursor at end
- **AND** no directories matching `zzz*` exist in `/home/user/`
- **THEN** `PathGhostCompletion::compute()` SHALL return `None`

#### Scenario: Cursor not at end of input
- **WHEN** the cursor is positioned before the end of the input value
- **THEN** `PathGhostCompletion::compute()` SHALL return `None`

#### Scenario: Hidden directories excluded unless prefix starts with dot
- **WHEN** the input value is `/home/user/` with cursor at end
- **THEN** `PathGhostCompletion::compute()` SHALL NOT include directories starting with `.` in candidates
- **WHEN** the input value is `/home/user/.c` with cursor at end
- **THEN** `PathGhostCompletion::compute()` SHALL include directories starting with `.c`

### Requirement: PathGhostCompletion supports tilde expansion
The component SHALL expand a leading `~` to the user's home directory when resolving the filesystem base path for completion.

#### Scenario: Tilde-prefixed path resolves to home directory
- **WHEN** the input value is `~/pro` with cursor at end
- **AND** the user's home directory is `/home/user`
- **AND** directory `/home/user/projects` exists
- **THEN** `PathGhostCompletion::compute()` SHALL return ghost text `jects/`

#### Scenario: Bare tilde resolves to home directory
- **WHEN** the input value is `~` with cursor at end
- **THEN** `PathGhostCompletion::compute()` SHALL list directories inside the user's home directory for completion

### Requirement: PathGhostCompletion accept validates staleness
The `accept()` method SHALL verify that the input value and cursor position have not changed since `compute()` was called. If the ghost is stale, `accept()` SHALL return `None`.

#### Scenario: Accept with unchanged input succeeds
- **WHEN** `PathGhostCompletion::compute()` returned a ghost for input `~/pro` at cursor 4
- **AND** the input is still `~/pro` with cursor at position 4
- **THEN** `accept()` SHALL return the input value with ghost text appended

#### Scenario: Accept with changed input returns None
- **WHEN** `PathGhostCompletion::compute()` returned a ghost for input `~/pro` at cursor 4
- **AND** the input has since changed to `~/proj`
- **THEN** `accept()` SHALL return `None`

### Requirement: PathGhostCompletion is a standalone reusable component
`PathGhostCompletion` SHALL be implemented as a standalone struct in `src/tui/components/path_ghost.rs` with `compute()` and `accept()` methods that take `&Input` parameters. It SHALL NOT depend on any specific dialog's internal state.

#### Scenario: Component used by NewSessionDialog
- **WHEN** `NewSessionDialog` uses `PathGhostCompletion` for its path field
- **THEN** the existing path autocomplete behavior SHALL be identical to before extraction

#### Scenario: Component used by GroupRenameDialog
- **WHEN** `GroupRenameDialog` uses `PathGhostCompletion` for its directory field
- **THEN** the directory field SHALL have filesystem path autocomplete with ghost text
