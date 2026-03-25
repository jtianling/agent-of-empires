## MODIFIED Requirements

### Requirement: Notification format uses index and title
Each session in the notification bar SHALL be displayed as `[index] icon title` where index is the session's real `@aoe_index` computed from the fully-expanded group tree. The index SHALL match the number used by `Ctrl+b <N>` jump keys. The notification bar SHALL NOT use sequential renumbering.

#### Scenario: Multiple sessions displayed with real indices
- **WHEN** sessions with stable index 2 ("api") and stable index 5 ("frontend") are both Waiting
- **THEN** the notification shows `[2] ◐ api [5] ◐ frontend`
- **AND** pressing `Ctrl+b 2 Space` SHALL jump to "api"
- **AND** pressing `Ctrl+b 5 Space` SHALL jump to "frontend"

#### Scenario: Indices have gaps from filtered sessions
- **WHEN** sessions with indices 1, 2, 3, 4, 5 exist
- **AND** sessions 1 (Stopped), 3 (current session), and 4 (Idle in collapsed group) are filtered out
- **THEN** the notification shows `[2] icon title2 [5] icon title5`
- **AND** the indices 1, 3, 4 SHALL NOT appear (those sessions are not shown)

#### Scenario: Notification index matches Ctrl+b N
- **WHEN** notification bar shows `[3] ◐ myapp`
- **AND** the user presses `Ctrl+b 3 Space`
- **THEN** the system SHALL switch to the "myapp" session

### Requirement: Notification entries sorted by session index
Notification bar entries SHALL be sorted by session index (ascending), regardless of status. The index used for sorting SHALL be the stable `@aoe_index` from the expanded group tree.

#### Scenario: Mixed status sessions sorted by stable index
- **WHEN** sessions with stable index 2 (Running), 3 (Waiting), and 7 (Idle) are all visible
- **THEN** the notification shows `[2] ● run [3] ◐ wait [7] ○ idle` in index order
