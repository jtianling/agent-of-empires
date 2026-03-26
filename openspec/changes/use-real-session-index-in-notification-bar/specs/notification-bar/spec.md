## MODIFIED Requirements

### Requirement: Notification format uses index and title
Each session in the notification bar SHALL be displayed as `[index] icon title` where index is the session's 1-based position in the global ordered session list (the same value as `@aoe_index`). This index SHALL be computed from the full `flatten_tree` order filtered to sessions that exist in tmux, matching the `Ctrl+b <N>` number-jump target. The index SHALL NOT be affected by notification visibility filtering (current session exclusion, collapsed group filtering).

#### Scenario: Multiple sessions displayed with real indices
- **WHEN** the global ordered session list is: chat (index 1), main (index 2), file (index 3), aoe-main (index 4)
- **AND** the user is attached to chat (index 1)
- **THEN** the notification bar shows `[2] icon main [3] icon file [4] icon aoe-main`

#### Scenario: Indices have gaps due to current session exclusion
- **WHEN** the global ordered session list is: api (index 1), web (index 2), db (index 3)
- **AND** the user is attached to web (index 2)
- **THEN** the notification bar shows `[1] icon api [3] icon db`

#### Scenario: Indices have gaps due to collapsed group filtering
- **WHEN** the global ordered session list is: chat (index 1), backend/api (index 2), backend/worker (index 3), frontend (index 4)
- **AND** the "backend" group is collapsed
- **AND** backend/api and backend/worker are both Idle
- **AND** the user is attached to chat
- **THEN** the notification bar shows `[4] icon frontend`
- **AND** indices 2 and 3 are not displayed but are not reassigned

#### Scenario: Waiting session in collapsed group retains real index
- **WHEN** the global ordered session list is: chat (index 1), backend/api (index 2), frontend (index 3)
- **AND** the "backend" group is collapsed
- **AND** backend/api is Waiting
- **AND** the user is attached to chat
- **THEN** the notification bar shows `[2] ◐ api`

### Requirement: Notification entries sorted by session index
Notification bar entries SHALL be sorted by session index (ascending), regardless of status. The index used for sorting SHALL be the real `@aoe_index` value, not a sequential enumeration.

#### Scenario: Mixed status sessions sorted by real index
- **WHEN** sessions with real index 2 (Running), 3 (Waiting), and 5 (Idle) are all visible
- **THEN** the notification shows `[2] ● run [3] ◐ wait [5] ○ idle` in index order
