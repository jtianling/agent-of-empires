## ADDED Requirements

### Requirement: Cross-group session cycling keybindings
`Ctrl+b N` (shift+n) and `Ctrl+b P` (shift+p) SHALL cycle through ALL sessions in the global
display order, ignoring group boundaries. The cycling SHALL wrap around at both ends.

#### Scenario: Cross-group next from last session in a group
- **WHEN** the current session is the last session in group "work"
- **AND** the next session in global order belongs to group "personal"
- **AND** the user presses `Ctrl+b N`
- **THEN** the system SHALL switch to that session in group "personal"

#### Scenario: Cross-group prev wraps to last session
- **WHEN** the current session is the first session in global order
- **AND** the user presses `Ctrl+b P`
- **THEN** the system SHALL wrap to the last session in global order

#### Scenario: Cross-group cycling ignores collapsed groups
- **WHEN** the current session is in an expanded group
- **AND** the next session in global order is inside a collapsed group
- **AND** the user presses `Ctrl+b N`
- **THEN** the system SHALL switch to that session inside the collapsed group

### Requirement: Cross-group keybinding lifecycle
The `Ctrl+b N/P` bindings SHALL follow the same lifecycle as existing `n/p` bindings: set up in
`setup_session_cycle_bindings()`, overridden in `apply_managed_session_bindings()` for nested mode,
and cleaned up in `cleanup_session_cycle_bindings()` and `cleanup_nested_detach_binding()`.

#### Scenario: Bindings work in non-nested mode
- **WHEN** AoE attaches to a session in non-nested mode
- **AND** `setup_session_cycle_bindings()` runs
- **THEN** `Ctrl+b N` and `Ctrl+b P` SHALL be bound

#### Scenario: Bindings work in nested mode
- **WHEN** AoE attaches to a session in nested mode
- **AND** `apply_managed_session_bindings()` runs
- **THEN** `Ctrl+b N` and `Ctrl+b P` SHALL be bound with profile-aware commands

#### Scenario: Bindings cleaned up on exit
- **WHEN** `cleanup_session_cycle_bindings()` runs
- **THEN** `N` and `P` SHALL be unbound from the prefix table

### Requirement: CLI switch-session supports --global flag
The `aoe tmux switch-session` command SHALL accept a `--global` flag. When set, session cycling
SHALL traverse all sessions regardless of group boundaries, ignoring collapse state.

#### Scenario: Global switch next
- **WHEN** `aoe tmux switch-session --direction next --global --profile default` is called
- **THEN** the system SHALL resolve the next session from the full unscoped session list

#### Scenario: Global switch without flag uses group scope
- **WHEN** `aoe tmux switch-session --direction next --profile default` is called (no --global)
- **THEN** the system SHALL use the existing group-scoped behavior
