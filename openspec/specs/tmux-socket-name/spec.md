# tmux-socket-name Specification

## Purpose
TBD - created by archiving change tmux-socket-seam. Update Purpose after archive.
## Requirements
### Requirement: Configurable tmux socket name

AoE SHALL support a user-configurable tmux socket name via `TmuxConfig.tmux_socket_name: Option<String>` (serde default `None`). At startup AoE SHALL resolve this value once into the process-global socket name used by `tmux_command()`. When set, every AoE tmux invocation (create, attach, kill, options, keybindings, status, reconcile, recovery) SHALL target `-L <name>`. When unset (default), AoE SHALL use the default tmux socket exactly as before.

#### Scenario: Configured name pins all tmux commands
- **WHEN** `tmux_socket_name = "jt"` is configured
- **AND** AoE starts and resolves the socket name
- **THEN** AoE tmux commands SHALL run on `-L jt`, including session create and attach

#### Scenario: Unset name preserves default behavior
- **WHEN** `tmux_socket_name` is unset
- **THEN** AoE tmux commands SHALL run on the default socket with no `-L` flag

#### Scenario: Attach honors the configured socket
- **WHEN** a socket name is configured
- **AND** the user attaches to a managed session
- **THEN** the attach and its keybinding/option setup SHALL target the same `-L <name>` socket

### Requirement: Socket name is a GLOBAL-only setting editable in the settings TUI

The tmux socket name SHALL be a GLOBAL setting: all entry points must share one tmux server so cross-profile session switching keeps working, so it SHALL NOT be profile-overridable (not added to `TmuxConfigOverride`/`merge_configs()`). It SHALL be editable in the settings TUI via a `FieldKey` variant and a `SettingField` entry that is shown ONLY in the Global scope, applied through `apply_field_to_global()`. Blank input SHALL normalize to `None` (default socket). The field SHALL carry help text noting that setting a socket name means AoE only sees sessions on that socket (pre-existing default-socket sessions will not appear until recreated there) and that it takes effect on next launch.

#### Scenario: Field is editable in Global scope and persists
- **WHEN** the user edits the tmux socket name in the Global settings scope and saves
- **THEN** the value SHALL persist to config and be applied on the next startup

#### Scenario: Field is not offered as a profile override
- **WHEN** the settings TUI is in the Profile or Repo scope
- **THEN** the tmux socket name field SHALL NOT be shown (it is global-only)

#### Scenario: Blank input clears to default socket
- **WHEN** the user sets the tmux socket name to an empty/whitespace value
- **THEN** the stored value SHALL be `None` (the default tmux socket)

