## ADDED Requirements

### Requirement: TUI dialogs that render paths use responsive widths with a 120-column cap

Dialogs that display or edit session paths or nested group paths (New Session, Edit Session, Edit Group, Fork Session, and the New Session sub-dialogs for Sandbox / Tool / Worktree configuration) SHALL compute their container width as `min(terminal_area_width - 4, 120)` rather than using a fixed width.

The cap of 120 columns keeps long lines readable on wide terminals; the `terminal_area_width - 4` floor allows the centered-rect clamp to degrade gracefully on narrow terminals without introducing overflow or panics.

Field layout, input behavior, and keybindings inside the dialogs are unchanged; only the outer container width is affected.

#### Scenario: Wide terminal shows dialogs at the 120-column cap

- **WHEN** the terminal width is 160 columns
- **AND** the user opens the New Session dialog, Edit Session dialog, Edit Group dialog, or Fork Session dialog
- **THEN** the dialog SHALL render at 120 columns wide
- **AND** group paths and filesystem paths up to roughly 110 characters SHALL display without truncation

#### Scenario: Medium terminal scales dialog to available width

- **WHEN** the terminal width is 100 columns
- **AND** the user opens any of the affected dialogs
- **THEN** the dialog SHALL render at 96 columns wide (terminal width minus 4)
- **AND** the dialog SHALL NOT overflow the terminal bounds

#### Scenario: Narrow terminal falls back to clamp behavior

- **WHEN** the terminal width is below 60 columns
- **AND** the user opens any of the affected dialogs
- **THEN** the dialog SHALL render within the terminal bounds using the existing `centered_rect` clamp
- **AND** the application SHALL NOT panic or produce an overflow layout

#### Scenario: Error wrap width tracks the responsive dialog width

- **WHEN** the New Session dialog renders an error message
- **THEN** the error-line wrap calculation SHALL use the same responsive width as the dialog container, not a stale fixed value
