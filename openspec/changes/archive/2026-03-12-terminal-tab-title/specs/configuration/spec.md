## MODIFIED Requirements

### Requirement: Global Config Structure
The global configuration SHALL include a `dynamic_tab_title` field in the `[app_state]` section. This field controls whether the TUI dynamically updates the terminal tab/window title.

```toml
[app_state]
has_seen_welcome = false
last_seen_version = ""
home_list_width = 0
diff_file_list_width = 0
sort_order = "newest"
dynamic_tab_title = true    # NEW: enable/disable dynamic terminal tab title
```

#### Scenario: Default value for new installs
- **WHEN** a user runs AoE for the first time with no config file
- **THEN** `dynamic_tab_title` SHALL default to `true`

#### Scenario: Missing field in existing config
- **WHEN** an existing config file does not contain the `dynamic_tab_title` field
- **THEN** the system SHALL treat the missing field as `true` (default enabled)

### Requirement: TUI Settings Requirement
The `dynamic_tab_title` field MUST be editable in the Settings TUI under the General tab. It SHALL follow the standard settings pattern: `FieldKey` variant, `SettingField` entry, and `apply_field_to_global()` wiring.

#### Scenario: User toggles dynamic tab title in settings
- **WHEN** the user navigates to General settings and toggles `dynamic_tab_title`
- **THEN** the setting SHALL be saved to `config.toml` and take effect immediately (title updates stop or start without restart)
