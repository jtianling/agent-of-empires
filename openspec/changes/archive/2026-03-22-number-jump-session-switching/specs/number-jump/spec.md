## ADDED Requirements

### Requirement: Global numeric index assignment
The system SHALL assign 1-based numeric indices to all visible sessions in the TUI list, following the current display order (respecting sort order and group structure). Group headers SHALL NOT receive indices. Sessions inside collapsed groups SHALL NOT receive indices. Indices SHALL be recalculated on every render. Maximum index is 99.

#### Scenario: Simple flat list numbering
- **WHEN** the session list contains 5 ungrouped sessions
- **THEN** sessions SHALL be numbered 1 through 5 in display order

#### Scenario: Grouped list numbering skips group headers
- **WHEN** the session list contains a group "backend" with 3 sessions and a group "frontend" with 2 sessions
- **THEN** the group headers SHALL have no number
- **AND** sessions SHALL be numbered 1 through 5 consecutively across groups

#### Scenario: Collapsed group sessions are not numbered
- **WHEN** a group containing 3 sessions is collapsed
- **THEN** those 3 sessions SHALL NOT receive indices
- **AND** sessions after the collapsed group SHALL be numbered contiguously (no gaps)

#### Scenario: More than 99 sessions
- **WHEN** there are more than 99 visible sessions
- **THEN** only the first 99 SHALL receive numeric indices
- **AND** sessions 100+ SHALL have no number displayed

### Requirement: TUI digit key triggers jump with pending state
When the user presses a digit key (1-9) on the TUI home screen, the system SHALL enter a pending jump state showing the first digit. A second digit (0-9) auto-confirms and jumps to the two-digit session. Space confirms the single-digit jump. Any other key cancels the pending state.

#### Scenario: Single digit jump with Space confirmation
- **WHEN** the user presses `3`
- **THEN** the TUI SHALL show a pending indicator for "3"
- **WHEN** the user then presses Space
- **THEN** the TUI SHALL attach to session #3

#### Scenario: Two digit jump auto-confirms
- **WHEN** the user presses `1`
- **THEN** the TUI SHALL show a pending indicator for "1"
- **WHEN** the user then presses `3`
- **THEN** the TUI SHALL immediately attach to session #13 (no Space needed)

#### Scenario: Cancel pending jump
- **WHEN** the user presses `2`
- **AND** the user then presses Esc or any non-digit non-Space key
- **THEN** the pending state SHALL be cleared
- **AND** no session jump SHALL occur

#### Scenario: Jump to nonexistent index
- **WHEN** the user completes a jump to index 15
- **AND** there are only 10 visible sessions
- **THEN** no session jump SHALL occur
- **AND** the pending state SHALL be cleared

#### Scenario: Digit 0 does not start a jump
- **WHEN** the user presses `0` without a pending first digit
- **THEN** no pending jump state SHALL be entered

#### Scenario: Pending state does not interfere with dialogs
- **WHEN** a dialog (new session, delete confirm, etc.) is open
- **AND** the user presses a digit key
- **THEN** no pending jump state SHALL be entered
- **AND** the digit SHALL be handled by the dialog

### Requirement: Tmux keybindings for number jump
When attached to an AoE-managed tmux session, `Ctrl+b 1` through `Ctrl+b 9` SHALL enter tmux key tables (`aoe-1` through `aoe-9`). Within each key table, Space confirms single-digit jump, digits 0-9 auto-confirm two-digit jump. Pressing any unbound key cancels.

#### Scenario: Single digit jump in tmux
- **WHEN** the user presses `Ctrl+b 3` then Space
- **THEN** the system SHALL switch to session #3

#### Scenario: Two digit jump in tmux
- **WHEN** the user presses `Ctrl+b 1` then `3`
- **THEN** the system SHALL immediately switch to session #13

#### Scenario: Cancel in tmux
- **WHEN** the user presses `Ctrl+b 5` then Escape
- **THEN** no session switch SHALL occur
- **AND** tmux SHALL return to the root key table

#### Scenario: Jump to nonexistent index in tmux
- **WHEN** the user presses `Ctrl+b 9 Space` and session #9 does not exist
- **THEN** no session switch SHALL occur

### Requirement: Number jump keybinding lifecycle
The number jump tmux bindings (1-9 in prefix table, aoe-1 through aoe-9 key tables) SHALL follow the same lifecycle as existing n/p/h/j/k/l bindings: set up on attach, cleaned up on detach/exit.

#### Scenario: Bindings set in both nested and non-nested modes
- **WHEN** `setup_session_cycle_bindings()` is called
- **THEN** keys 1-9 SHALL be bound in the prefix table
- **AND** key tables aoe-1 through aoe-9 SHALL be created with Space + digit bindings

#### Scenario: Bindings cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** keys 1-9 SHALL be unbound from the prefix table
- **AND** all aoe-N key table bindings SHALL be unbound

#### Scenario: Nested mode overrides use correct switch command
- **WHEN** `apply_managed_session_bindings()` is called in nested mode
- **THEN** the 1-9 bindings SHALL use the same profile-aware switch command as the n/p bindings

### Requirement: CLI switch-session supports --index parameter
The `aoe tmux switch-session` command SHALL accept an `--index N` parameter (1-based) as an alternative to `--direction`. The index resolves against the global ordered session list (same order as TUI display), not scoped to the current group.

#### Scenario: Switch by index
- **WHEN** `aoe tmux switch-session --index 3 --profile default` is called
- **THEN** the system SHALL switch to the 3rd session in the global display order

#### Scenario: Index out of range
- **WHEN** `aoe tmux switch-session --index 50` is called
- **AND** there are only 10 sessions
- **THEN** no switch SHALL occur
- **AND** the command SHALL exit successfully (no error)

#### Scenario: Index resolves at runtime
- **WHEN** sessions are created or deleted between the time bindings were set and the jump is triggered
- **THEN** the index SHALL resolve against the current session list at the time of the jump
