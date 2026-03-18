## MODIFIED Requirements

### Requirement: Sandbox configuration fields
The sandbox section of the global config (`config.toml`) SHALL contain the following fields:

```toml
[sandbox]
enabled = false
image = "ubuntu:latest"
volumes = []
environment = []
port_mappings = []
ssh_mount = false
auth_volumes = false
runtime = "docker"
```

The `default_terminal_mode` field SHALL NOT be present in the sandbox configuration schema. If present in an existing config file, it SHALL be silently ignored during parsing.

#### Scenario: Config loads without default_terminal_mode
- **WHEN** config.toml sandbox section does not contain default_terminal_mode
- **THEN** config loads successfully

#### Scenario: Config tolerates old default_terminal_mode field
- **WHEN** config.toml sandbox section contains `default_terminal_mode = "host"`
- **THEN** config loads successfully, ignoring the unknown field

### Requirement: Settings TUI sandbox fields
The settings TUI sandbox tab SHALL NOT include a "Default Terminal Mode" field. All other sandbox fields remain editable.

#### Scenario: Settings TUI has no terminal mode setting
- **WHEN** user opens the settings TUI and navigates to the sandbox tab
- **THEN** there is no "Default Terminal Mode" field visible

### Requirement: Profile and repo overrides for sandbox
The `SandboxConfigOverride` struct SHALL NOT include a `default_terminal_mode` field. Profile and repo-level overrides for this field are removed.

#### Scenario: Profile override without terminal mode
- **WHEN** a profile config overrides sandbox settings
- **THEN** no default_terminal_mode override is available or applied
