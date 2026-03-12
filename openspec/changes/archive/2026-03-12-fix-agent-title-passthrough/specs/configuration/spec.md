## MODIFIED Requirements

### Requirement: Global Config: `dynamic_tab_title`
The global configuration SHALL NOT expose a `dynamic_tab_title` field once AoE TUI title management has been removed.

#### Scenario: New config file is written
- **WHEN** AoE writes or rewrites `config.toml`
- **THEN** it SHALL NOT emit a `dynamic_tab_title` entry under `[app_state]`

#### Scenario: Existing config file is migrated
- **WHEN** AoE starts with an existing `config.toml` that still contains `[app_state].dynamic_tab_title`
- **THEN** the migration system SHALL remove that field from the persisted config file

#### Scenario: Settings UI is shown
- **WHEN** the user opens General settings
- **THEN** the settings list SHALL NOT include a `Dynamic Tab Title` field
