## ADDED Requirements

### Requirement: Ctrl+b b toggles to previous session
When attached to an AoE-managed tmux session, pressing `Ctrl+b b` SHALL switch to the session the user was in before the last switch. Pressing `Ctrl+b b` again SHALL toggle back (since the switch itself records the source as the new previous session).

#### Scenario: Toggle back after number jump
- **WHEN** the user is in session #3 and presses `Ctrl+b 7 Space` to jump to session #7
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch back to session #3

#### Scenario: Toggle creates a two-session cycle
- **WHEN** the user toggles from session #7 back to session #3 via `Ctrl+b b`
- **AND** the user presses `Ctrl+b b` again
- **THEN** the system SHALL switch to session #7

#### Scenario: No previous session exists
- **WHEN** the user has just entered a session from the TUI for the first time (no prior switch)
- **AND** the user presses `Ctrl+b b`
- **THEN** no session switch SHALL occur

#### Scenario: Previous session no longer exists
- **WHEN** the previous session has been deleted
- **AND** the user presses `Ctrl+b b`
- **THEN** no session switch SHALL occur

### Requirement: All jump types record previous session
Every AoE session switch (group cycle n/p, global cycle N/P, number jump 1-9, and back toggle b) SHALL record the current session as the previous session before switching. This ensures Ctrl+b b always returns to wherever the user just came from, regardless of which navigation method was used.

#### Scenario: Previous session recorded on group cycle
- **WHEN** the user is in session #3 and presses `Ctrl+b n` to cycle to session #4
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch to session #3

#### Scenario: Previous session recorded on global cycle
- **WHEN** the user is in session #3 and presses `Ctrl+b N` to cycle to session #8 in another group
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch to session #3

#### Scenario: Previous session recorded on number jump
- **WHEN** the user is in session #5 and presses `Ctrl+b 2 Space` to jump to session #2
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch to session #5

### Requirement: Back toggle keybinding lifecycle
The `Ctrl+b b` binding SHALL follow the same lifecycle as existing navigation bindings (n/p/N/P):
- Set up in `setup_session_cycle_bindings()` (works in both nested and non-nested modes)
- Overridden in `apply_managed_session_bindings()` for nested mode with profile-from-option
- Cleaned up in `cleanup_session_cycle_bindings()` and `cleanup_nested_detach_binding()`

#### Scenario: Binding set in non-nested mode
- **WHEN** `setup_session_cycle_bindings()` is called with a profile
- **THEN** key `b` SHALL be bound in the prefix table to execute the back-toggle command

#### Scenario: Binding cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** key `b` SHALL be unbound from the prefix table

#### Scenario: Binding cleaned up in nested mode exit
- **WHEN** `cleanup_nested_detach_binding()` is called
- **THEN** key `b` SHALL be unbound

### Requirement: CLI switch-session supports --back parameter
The `aoe tmux switch-session` command SHALL accept a `--back` flag that reads the stored previous session for the given client and switches to it. The `--back` flag conflicts with `--direction` and `--index`.

#### Scenario: Switch back via CLI
- **WHEN** `aoe tmux switch-session --back --profile default --client-name /dev/pts/0` is called
- **AND** a previous session is stored for that client
- **THEN** the system SHALL switch to the stored previous session

#### Scenario: No stored previous session
- **WHEN** `aoe tmux switch-session --back` is called
- **AND** no previous session is stored for the client
- **THEN** no switch SHALL occur
- **AND** the command SHALL exit successfully

### Requirement: Status bar shows from-title
When a session has a recorded source session (set during the last switch into it), the status bar SHALL display "from: <source_title>" after the session title. When no source is recorded, the "from:" section SHALL NOT appear.

#### Scenario: From title displayed after jump
- **WHEN** the user switches from "Work Agent" to "Helper Agent" via any navigation method
- **THEN** the status bar in "Helper Agent" SHALL show "from: Work Agent"

#### Scenario: No from title on first entry
- **WHEN** a session is entered from the TUI for the first time
- **THEN** the status bar SHALL NOT show a "from:" section

### Requirement: Status bar shows session index
The status bar SHALL display the current session's 1-based index number (matching the TUI display order) at the leftmost position. The index is stored as a tmux session option `@aoe_index` and set on each session switch.

#### Scenario: Index displayed in status bar
- **WHEN** the user switches to the 3rd session in the global list
- **THEN** the status bar SHALL show "3" as the index

#### Scenario: Index set on every switch type
- **WHEN** the user navigates to a session via any method (n/p, N/P, 1-9, b)
- **THEN** the target session's `@aoe_index` SHALL be updated to its current position in the ordered session list

### Requirement: Status bar layout
The status bar SHALL use the following layout:
- status-left: session index (green, bold), session title (white), conditional "from: <title>" (dim), single hint "Ctrl+b d detach" (dim)
- status-right: conditional branch (cyan), conditional sandbox (orange), time (white)
- Window list (window-status-format and window-status-current-format) SHALL be set to empty strings to hide the default tmux window list

#### Scenario: Full status bar with all elements
- **WHEN** a session has index 3, title "My Agent", from-title "Helper", branch "main", and sandbox "container-1"
- **THEN** status-left SHALL show: `3 My Agent  from: Helper  Ctrl+b d detach`
- **AND** status-right SHALL show: `main | container-1 | 14:30`

#### Scenario: Minimal status bar
- **WHEN** a session has index 1, title "Claude", no from-title, no branch, no sandbox
- **THEN** status-left SHALL show: `1 Claude  Ctrl+b d detach`
- **AND** status-right SHALL show: `14:30`

#### Scenario: Window list hidden
- **WHEN** status bar is applied to a session
- **THEN** the tmux window-status-format and window-status-current-format SHALL be empty strings
