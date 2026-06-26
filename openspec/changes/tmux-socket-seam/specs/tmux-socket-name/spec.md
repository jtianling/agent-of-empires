## ADDED Requirements

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

### Requirement: Socket name editable in the settings TUI

The `tmux_socket_name` field SHALL be editable in the settings TUI like every other configurable field: a `FieldKey` variant, a `SettingField` entry, `apply_field_to_global()`/`apply_field_to_profile()` wiring, a `clear_profile_override()` case, and `TmuxConfigOverride` merge logic. The field SHALL carry help text noting that setting a socket name means AoE only sees sessions on that socket (pre-existing default-socket sessions will not appear until recreated there).

#### Scenario: Field is editable and persists
- **WHEN** the user edits the tmux socket name in the settings TUI and saves
- **THEN** the value SHALL persist to config and be applied on the next startup

#### Scenario: Profile override can be cleared
- **WHEN** a profile sets a `tmux_socket_name` override and the user clears it
- **THEN** the effective value SHALL fall back to the global config value via `merge_configs()`
