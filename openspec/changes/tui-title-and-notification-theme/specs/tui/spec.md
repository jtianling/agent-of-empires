## ADDED Requirements

### Requirement: TUI panel title uses short form
The TUI home screen left-panel title SHALL display `AoE [{profile}]` instead of `Agent of Empires [{profile}]`, matching the terminal tab title convention and providing more space for the profile name.

#### Scenario: Panel title shows short form
- **WHEN** the TUI home screen is rendered
- **THEN** the left panel title SHALL display `" AoE [{profile}] "` where `{profile}` is the active profile name

#### Scenario: Profile name has more display room
- **WHEN** the terminal width is 80 columns
- **THEN** the shortened title SHALL allow approximately 15 more characters of profile name to be visible compared to the previous format
