## ADDED Requirements

### Requirement: TUI integrates terminal tab title updates into event loop
The TUI event loop SHALL compute the current tab title state after processing events and before rendering, and update the terminal tab title when it changes. Title writes SHALL occur alongside the existing synchronized update sequence.

#### Scenario: Title update during normal event loop
- **WHEN** the event loop processes a state change (dialog open/close, view switch, creation start/finish)
- **THEN** the tab title SHALL be updated before the next draw call

#### Scenario: Title update with synchronized output
- **WHEN** the TUI writes a title update
- **THEN** it SHALL be written outside the synchronized update block (before `BeginSynchronizedUpdate`) to avoid interfering with frame rendering

### Requirement: Terminal teardown includes title reset
The terminal teardown sequence in `src/tui/mod.rs` SHALL include a title reset step alongside the existing `LeaveAlternateScreen` and `DisableMouseCapture` cleanup.

#### Scenario: Teardown sequence order
- **WHEN** the TUI exits and restores the terminal
- **THEN** the title reset SHALL execute as part of the teardown sequence, before `LeaveAlternateScreen`
