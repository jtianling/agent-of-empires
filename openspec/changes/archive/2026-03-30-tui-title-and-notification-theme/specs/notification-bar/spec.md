## MODIFIED Requirements

### Requirement: Notification text uses distinct color
The notification section SHALL use theme-consistent colors that align with the AoE TUI Empire palette. Specifically:
- Session index: `#22c55e` (theme `running` green)
- Session title text: `#cbd5e1` (theme `text` cool gray)
- Hint text (e.g., "Ctrl+b d detach"): `#94a3b8` (theme `hint` light slate)
- Notification/waiting text: `#fbbf24` (theme `waiting` amber)
- From-title text: `#64748b` (theme `dimmed` slate)

These colors SHALL be specified using tmux hex color syntax (`#[fg=#rrggbb]`).

#### Scenario: Notification visible
- **WHEN** notification text is displayed
- **THEN** it renders in `#fbbf24` (amber), contrasting with the `#94a3b8` (light slate) of "Ctrl+b d detach"

#### Scenario: Session index color matches theme
- **WHEN** a session index is displayed in the status bar
- **THEN** it renders in `#22c55e` (green), matching the TUI running indicator color

#### Scenario: Title text color matches theme
- **WHEN** session title text is displayed in the status bar
- **THEN** it renders in `#cbd5e1` (cool gray), matching the TUI text color
