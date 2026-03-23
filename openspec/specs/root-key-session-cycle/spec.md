# Capability Spec: Root Key Session Cycle

**Capability**: `root-key-session-cycle`
**Created**: 2026-03-23
**Status**: Draft

## Purpose

Root key session cycling provides prefix-free session navigation via `Ctrl+,` and `Ctrl+.` in the tmux root key table. These bindings cycle through all AoE-managed sessions in global display order (across all groups), replacing the previous prefix-based `n`/`p`/`N`/`P` bindings. Non-AoE sessions pass the keystrokes through transparently.

## Requirements

### Requirement: Ctrl+, cycles to the previous session in global order
When attached to an AoE-managed tmux session, pressing `Ctrl+,` SHALL switch to the previous session in the global display order (all sessions across all groups). The cycling SHALL wrap from the first session to the last.

#### Scenario: Previous session in same group
- **WHEN** the current session is the 3rd session in group "work" and the 2nd session exists in "work"
- **AND** the user presses `Ctrl+,`
- **THEN** the system SHALL switch to the 2nd session in "work"

#### Scenario: Previous session crosses group boundary
- **WHEN** the current session is the first session in group "personal"
- **AND** the previous session in global order is the last session in group "work"
- **AND** the user presses `Ctrl+,`
- **THEN** the system SHALL switch to the last session in group "work"

#### Scenario: Wrap from first to last session
- **WHEN** the current session is the first session in global order
- **AND** the user presses `Ctrl+,`
- **THEN** the system SHALL wrap to the last session in global order

### Requirement: Ctrl+. cycles to the next session in global order
When attached to an AoE-managed tmux session, pressing `Ctrl+.` SHALL switch to the next session in the global display order (all sessions across all groups). The cycling SHALL wrap from the last session to the first.

#### Scenario: Next session in same group
- **WHEN** the current session is the 2nd session in group "work" and a 3rd session exists in "work"
- **AND** the user presses `Ctrl+.`
- **THEN** the system SHALL switch to the 3rd session in "work"

#### Scenario: Next session crosses group boundary
- **WHEN** the current session is the last session in group "work"
- **AND** the next session in global order is the first session in group "personal"
- **AND** the user presses `Ctrl+.`
- **THEN** the system SHALL switch to the first session in group "personal"

#### Scenario: Wrap from last to first session
- **WHEN** the current session is the last session in global order
- **AND** the user presses `Ctrl+.`
- **THEN** the system SHALL wrap to the first session in global order

### Requirement: Root-table bindings only act in AoE-managed sessions
The `Ctrl+,` and `Ctrl+.` bindings SHALL be in the tmux root key table (no prefix required). When pressed in a non-AoE session (session name does not start with `aoe_`), the keystroke SHALL be passed through to the application via `tmux send-keys`.

#### Scenario: Binding fires in AoE session
- **WHEN** the user is attached to a session named `aoe_my_agent`
- **AND** the user presses `Ctrl+.`
- **THEN** the system SHALL cycle to the next session

#### Scenario: Binding passes through in non-AoE session
- **WHEN** the user is attached to a session named `my-other-session` (not an aoe_* session)
- **AND** the user presses `Ctrl+.`
- **THEN** the system SHALL send the raw `C-.` keystroke to the active pane

### Requirement: Cycling records previous session for back-toggle
Every session switch triggered by `Ctrl+,` or `Ctrl+.` SHALL record the current session as the previous session before switching, so that `Ctrl+b b` returns to the session the user came from.

#### Scenario: Back-toggle after Ctrl+. cycle
- **WHEN** the user is in session #3 and presses `Ctrl+.` to cycle to session #4
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch back to session #3

#### Scenario: Back-toggle after Ctrl+, cycle
- **WHEN** the user is in session #5 and presses `Ctrl+,` to cycle to session #4
- **AND** the user then presses `Ctrl+b b`
- **THEN** the system SHALL switch back to session #5

### Requirement: Keybinding lifecycle for Ctrl+, and Ctrl+.
The `Ctrl+,` and `Ctrl+.` bindings SHALL follow a simplified lifecycle with only setup and cleanup:
- Set up in `setup_session_cycle_bindings()` with the profile hardcoded in the shell command
- Cleaned up in `cleanup_session_cycle_bindings()`

#### Scenario: Bindings set during session cycle setup
- **WHEN** `setup_session_cycle_bindings()` is called with a profile
- **THEN** `C-,` and `C-.` SHALL be bound in the root key table with session-guard logic and the profile hardcoded

#### Scenario: Bindings cleaned up on exit
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** `C-,` and `C-.` SHALL be unbound from the root key table

### Requirement: n/p/N/P prefix bindings are removed
The keys `n`, `p`, `N`, and `P` SHALL NOT be bound in the tmux prefix table for session cycling. All session cycling (previously split between group-scoped and global) is consolidated into `Ctrl+,`/`Ctrl+.`.

#### Scenario: n key is not bound after setup
- **WHEN** `setup_session_cycle_bindings()` is called
- **THEN** no binding for key `n` SHALL exist in the prefix table for session cycling

#### Scenario: N key is not bound after setup
- **WHEN** `setup_session_cycle_bindings()` is called
- **THEN** no binding for key `N` SHALL exist in the prefix table for session cycling

### Requirement: CLI switch-session removes --global flag
The `aoe tmux switch-session` command SHALL remove the `--global` flag. When `--direction` is specified, the system SHALL always use global (cross-group) session ordering. The group-scoped cycling code path SHALL be removed.

#### Scenario: Direction next uses global order
- **WHEN** `aoe tmux switch-session --direction next --profile default --client-name /dev/pts/0` is called
- **THEN** the system SHALL resolve the next session from the full global session list (ignoring group boundaries)

#### Scenario: --global flag is rejected
- **WHEN** `aoe tmux switch-session --direction next --global --profile default` is called
- **THEN** the command SHALL fail with an unrecognized flag error
