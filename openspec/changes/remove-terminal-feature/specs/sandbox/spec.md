## REMOVED Requirements

### Requirement: Default terminal mode
**Reason**: The paired terminal feature is removed. There is no longer a host/container terminal toggle.
**Migration**: The `default_terminal_mode` field is removed from `SandboxConfig`. Existing config files with this field will have it silently ignored during deserialization.

### Requirement: Container terminal
**Reason**: Container terminals (docker exec shells paired with agent sessions) are removed along with the entire paired terminal feature.
**Migration**: Existing `aoe_cterm_*` tmux sessions will be orphaned and can be manually killed. Users can run `docker exec` directly if they need a container shell.

## MODIFIED Requirements

### Requirement: Sandbox configuration
The `SandboxConfig` SHALL contain the following fields:

| Field | Type | Default |
|-------|------|---------|
| enabled | bool | false |
| image | String | "ubuntu:latest" |
| volumes | Vec\<String\> | [] |
| environment | Vec\<String\> | [] |
| cpu_limit | Option\<f64\> | None |
| memory_limit | Option\<String\> | None |
| port_mappings | Vec\<String\> | [] |
| ssh_mount | bool | false |
| auth_volumes | bool | false |
| runtime | ContainerRuntime | Docker |
| custom_instructions | Option\<String\> | None |

The `default_terminal_mode` field SHALL NOT be present.

#### Scenario: Config without default_terminal_mode loads correctly
- **WHEN** a SandboxConfig is loaded from TOML
- **THEN** it loads without error and does not include a default_terminal_mode field

#### Scenario: Old config with default_terminal_mode is tolerated
- **WHEN** a SandboxConfig TOML file contains an unknown `default_terminal_mode` key
- **THEN** deserialization succeeds, ignoring the unknown field
