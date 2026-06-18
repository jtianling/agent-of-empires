## ADDED Requirements

### Requirement: Zoom-aware pane cycling
The `C-;` keybinding SHALL detect whether the current pane is zoomed. When zoomed, it SHALL switch to the next pane and re-zoom it. When not zoomed, it SHALL behave as before (`select-pane -t :.+`).

#### Scenario: Switch panes while zoomed
- **WHEN** the current pane is zoomed and user presses `C-;`
- **THEN** the next pane becomes active and is zoomed to fill the window

#### Scenario: Switch panes while not zoomed
- **WHEN** the current pane is not zoomed and user presses `C-;`
- **THEN** the next pane becomes active without changing zoom state (existing behavior)

#### Scenario: Single pane zoomed
- **WHEN** only one pane exists and it is zoomed and user presses `C-;`
- **THEN** the same pane remains active and zoomed (no-op)

### Requirement: Auto-zoom on narrow attach
When attaching to a session from a narrow terminal, AoE SHALL auto-zoom pane 0 (the left/agent pane) if the session has more than one pane. The narrow threshold SHALL match the TUI single-panel threshold (`available_width < list_width + 20`).

#### Scenario: Attach from narrow terminal with split panes
- **WHEN** terminal width is 40 columns, list_width is 45, and session has 2 panes
- **THEN** pane 0 is zoomed before attach, user sees full-screen agent pane

#### Scenario: Attach from narrow terminal with single pane
- **WHEN** terminal width is 40 columns and session has only 1 pane
- **THEN** no zoom is applied (unnecessary)

#### Scenario: Attach from wide terminal with split panes
- **WHEN** terminal width is 120 columns and session has 2 panes
- **THEN** no auto-zoom is applied, both panes remain visible

### Requirement: Zoom cleanup in keybinding teardown
The `C-;` zoom-aware binding SHALL be cleaned up by `cleanup_session_cycle_bindings()` alongside other AoE keybindings.

#### Scenario: Keybinding cleanup on TUI exit
- **WHEN** the TUI exits and cleanup runs
- **THEN** the `C-;` binding is unbound (same as current behavior, no new cleanup needed since the key is the same)
