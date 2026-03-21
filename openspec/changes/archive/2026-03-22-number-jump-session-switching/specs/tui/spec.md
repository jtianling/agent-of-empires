## MODIFIED Requirements

### Requirement: TUI integrates terminal tab title updates into event loop
The TUI event loop SHALL compute the current tab title state after processing events and before rendering, and update the terminal tab title when it changes. Title writes SHALL occur alongside the existing synchronized update sequence.

#### Scenario: Title update during normal event loop
- **WHEN** the event loop processes a state change (dialog open/close, view switch, creation start/finish)
- **THEN** the tab title SHALL be updated before the next draw call

#### Scenario: Title update with synchronized output
- **WHEN** the TUI writes a title update
- **THEN** it SHALL be written outside the synchronized update block (before `BeginSynchronizedUpdate`) to avoid interfering with frame rendering

## ADDED Requirements

### Requirement: Session list displays numeric indices
The TUI session list SHALL display a right-aligned numeric index (1-99) as a fixed-width prefix before the status icon for each visible session. Group headers SHALL show blank space in the index column to maintain alignment.

#### Scenario: Index display with single digits
- **WHEN** sessions 1-9 are visible
- **THEN** indices SHALL be displayed right-aligned in a 2-character-wide column (e.g., ` 1`, ` 2`)

#### Scenario: Index display with double digits
- **WHEN** more than 9 sessions are visible
- **THEN** single-digit indices SHALL be right-aligned (` 1`) and double-digit indices left-aligned (`10`)

#### Scenario: Index display for groups
- **WHEN** a group header is rendered
- **THEN** the index column SHALL be blank (spaces) to maintain alignment with session rows

### Requirement: Pending jump visual indicator
When a pending jump is active, the TUI SHALL display a visual indicator showing the pending digit. The status bar SHALL show the pending state (e.g., `jump: 3_`). The session matching the pending single digit SHALL be visually highlighted.

#### Scenario: Pending state shown in status bar
- **WHEN** the user presses `3` to start a jump
- **THEN** the status bar SHALL show `3_` or similar pending indicator

#### Scenario: Pending state clears after jump or cancel
- **WHEN** the pending jump completes (Space or second digit) or is cancelled
- **THEN** the status bar SHALL return to normal
