## MODIFIED Requirements

### Requirement: All jump types record previous session
Every AoE session switch (global cycle via `Ctrl+,`/`Ctrl+.`, number jump 1-9, and back toggle `Ctrl+b b`) SHALL record the current session as the previous session before switching. This ensures `Ctrl+b b` always returns to wherever the user just came from, regardless of which navigation method was used.

#### Scenario: Previous session recorded on global cycle
- **WHEN** the user is in session #3 and presses `Ctrl+.` to cycle to session #4
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch to session #3

#### Scenario: Previous session recorded on reverse global cycle
- **WHEN** the user is in session #5 and presses `Ctrl+,` to cycle to session #4
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch to session #5

#### Scenario: Previous session recorded on number jump
- **WHEN** the user is in session #5 and presses `Ctrl+b 2 Space` to jump to session #2
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch to session #5

### Requirement: Back toggle keybinding lifecycle
The `Ctrl+b b` binding SHALL follow the same lifecycle as existing navigation bindings (`Ctrl+,`/`Ctrl+.`):
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

## REMOVED Requirements

### Requirement: Previous session recorded on group cycle
**Reason**: Group-scoped cycling (`Ctrl+b n`/`Ctrl+b p`) has been removed. All cycling now uses global order via `Ctrl+,`/`Ctrl+.`. The scenario "Previous session recorded on group cycle" from the original spec is replaced by "Previous session recorded on global cycle" above.
**Migration**: Users should use `Ctrl+,`/`Ctrl+.` for session cycling. Back-toggle continues to work with the new cycling keys.

### Requirement: Previous session recorded on global cycle
**Reason**: The original scenario referenced `Ctrl+b N` which no longer exists. This requirement is replaced by the updated "All jump types record previous session" above which uses the new `Ctrl+,`/`Ctrl+.` keybindings.
**Migration**: The requirement still exists, just with updated keybinding references. No functional change.
