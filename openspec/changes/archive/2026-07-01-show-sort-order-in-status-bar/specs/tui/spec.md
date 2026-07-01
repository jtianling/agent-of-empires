## ADDED Requirements

### Requirement: Status bar shows current sort order
The home-view status bar SHALL display the current session-list sort order and the key that cycles it, positioned right-aligned at the far right of the status bar so it does not push the left-aligned key hints. The indicator SHALL show the cycle key (`o`) and the current sort label (one of `Newest`, `Oldest`, `A-Z`, `Z-A`, `Manual`). When the current sort order is `Manual`, the status bar SHALL additionally show a `J/K` move hint, since manual reordering is only active in `Manual` sort. The status-bar area SHALL be split into a flexible left region (the existing key hints, which truncate first when the terminal is too narrow) and a fixed-width right region (the sort indicator) so the two regions never overlap.

#### Scenario: Sort order shown for a non-manual mode
- **WHEN** the session list sort order is `Newest`
- **THEN** the status bar SHALL show the cycle key `o` and the label `Newest`, right-aligned
- **AND** the status bar SHALL NOT show the `J/K` move hint

#### Scenario: Manual sort shows the move hint
- **WHEN** the session list sort order is `Manual`
- **THEN** the status bar SHALL show the cycle key `o` and the label `Manual`, right-aligned
- **AND** the status bar SHALL additionally show a `J/K` move hint

#### Scenario: Sort indicator updates when the order is cycled
- **WHEN** the user presses `o` to cycle the sort order
- **THEN** the status-bar sort label SHALL update to the newly selected order
- **AND** the `J/K` move hint SHALL appear only once the order becomes `Manual`

#### Scenario: Left hints truncate before overlapping the sort indicator
- **WHEN** the terminal is too narrow to fit both the left key hints and the right sort indicator
- **THEN** the left key hints SHALL truncate within their region
- **AND** the right sort indicator SHALL remain visible without overlapping the left hints
