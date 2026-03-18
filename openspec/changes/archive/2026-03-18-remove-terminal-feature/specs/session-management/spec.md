## REMOVED Requirements

### Requirement: Terminal info tracking
**Reason**: The paired terminal feature is removed. Sessions no longer track terminal metadata.
**Migration**: The `terminal_info` field is removed from `Instance`. Existing session JSON files with this field will have it silently ignored during deserialization (serde default behavior).

### Requirement: Terminal session attach sets bindings
**Reason**: `TerminalSession::attach()` and `ContainerTerminalSession::attach()` no longer exist.
**Migration**: Agent session attach continues to set bindings as before. Only terminal-specific attach is removed.

### Requirement: Container terminal pane ID storage
**Reason**: `ContainerTerminalSession` is removed entirely.
**Migration**: None needed. Pane ID storage for agent sessions is unaffected.

## MODIFIED Requirements

### Requirement: Session entity
An `Instance` SHALL contain the following fields:

| Field | Type | Description |
|-------|------|-------------|
| id | String | 16-char hex identifier |
| title | String | Display name |
| project_path | String | Working directory |
| group_path | Option\<String\> | Group hierarchy path |
| tool | String | Agent tool identifier |
| command | Option\<String\> | Custom launch command |
| status | Status | Current session status |
| created_at | DateTime\<Utc\> | Creation timestamp |
| last_attached | Option\<DateTime\<Utc\>\> | Last attach timestamp |
| worktree_info | Option\<WorktreeInfo\> | Git worktree metadata |
| sandbox_info | Option\<SandboxInfo\> | Container sandbox metadata |
| last_error | Option\<String\> | Last error message |

The `terminal_info` field SHALL NOT be present.

#### Scenario: Session loads without terminal_info
- **WHEN** a session JSON file is loaded that does not contain terminal_info
- **THEN** deserialization succeeds normally

#### Scenario: Old session with terminal_info is tolerated
- **WHEN** a session JSON file contains a terminal_info field
- **THEN** deserialization succeeds, ignoring the unknown field
