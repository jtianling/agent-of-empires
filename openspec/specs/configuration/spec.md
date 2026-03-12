# Capability Spec: Configuration System

**Capability**: `configuration`
**Created**: 2026-03-06
**Status**: Stable

## Overview

AoE uses a three-level hierarchical configuration system. Each level can override the previous:

```
Global Config  (config.toml in app dir)
    │
    ▼
Profile Config  (profiles/<name>/config.toml)
    │
    ▼
Repo Config    (.aoe/config.toml in project directory)
```

Settings that are "personal" (theme, updates, tmux status bar, claude config dir) are only
configurable at the global level. All operational settings (sandbox, worktree, session, hooks)
support profile and/or repo overrides.

## File Locations

| Platform | App Directory |
|----------|---------------|
| Linux | `$XDG_CONFIG_HOME/agent-of-empires/` (default: `~/.config/agent-of-empires/`) |
| macOS / Windows | `~/.agent-of-empires/` |

| File | Location |
|------|----------|
| Global config | `<app-dir>/config.toml` |
| Profile config | `<app-dir>/profiles/<name>/config.toml` |
| Session storage | `<app-dir>/profiles/<name>/sessions.json` |
| Group storage | `<app-dir>/profiles/<name>/groups.json` |
| Schema version | `<app-dir>/.schema_version` |
| Repo config | `<project>/.aoe/config.toml` |

## Global Config Structure (`Config`)

```toml
default_profile = "default"

[theme]
name = ""                    # theme name (empty = default)

[claude]
config_dir = ""              # custom Claude config directory

[updates]
check_enabled = true
auto_update = false
check_interval_hours = 24
notify_in_cli = true

[worktree]
enabled = false
path_template = "../{repo-name}-worktrees/{branch}"
bare_repo_path_template = "./{branch}"
auto_cleanup = true
show_branch_in_tui = true
delete_branch_on_cleanup = false

[sandbox]
enabled_by_default = false
default_image = "ghcr.io/njbrake/aoe-sandbox:latest"
extra_volumes = []
environment = ["TERM", "COLORTERM", "FORCE_COLOR", "NO_COLOR"]
auto_cleanup = true
cpu_limit = ""               # optional
memory_limit = ""            # optional
port_mappings = []
default_terminal_mode = "host"
volume_ignores = []
mount_ssh = false
custom_instruction = ""      # optional
container_runtime = "docker"

[tmux]
status_bar = "auto"          # auto | enabled | disabled
mouse = "auto"               # auto | enabled | disabled

[session]
default_tool = ""            # optional: claude | opencode | vibe | codex | gemini | cursor
yolo_mode_default = false

[diff]
default_branch = ""          # optional: e.g. "main"
context_lines = 3

[hooks]
on_create = []
on_launch = []

[sound]
# see sounds spec

[app_state]
has_seen_welcome = false
last_seen_version = ""
home_list_width = 0          # optional
diff_file_list_width = 0     # optional
sort_order = "newest"        # optional
```

## Override Pattern

Profile and repo configs use `Option<T>` fields (the `*Override` structs). A `None` value
means "inherit from the parent level". Merging logic:

```
resolved_value = repo_override
    .or(profile_override)
    .unwrap_or(global_value)
```

### Overridable at Profile Level

`SandboxConfigOverride`, `WorktreeConfigOverride`, `SessionConfigOverride`,
`HooksConfigOverride`, `TmuxConfigOverride`, `UpdatesConfigOverride`, `SoundConfigOverride`

### Overridable at Repo Level

`SandboxConfigOverride`, `WorktreeConfigOverride`, `SessionConfigOverride`,
`HooksConfigOverride`, `TmuxConfigOverride`, `UpdatesConfigOverride`, `SoundConfigOverride`

### Not Overridable (global only)

`ThemeConfig`, `ClaudeConfig`, `DiffConfig`, `AppStateConfig`

## Global Config: `dynamic_tab_title`

The global configuration SHALL NOT expose a `dynamic_tab_title` field once AoE TUI title
management has been removed.

#### Scenario: New config file is written
- **WHEN** AoE writes or rewrites `config.toml`
- **THEN** it SHALL NOT emit a `dynamic_tab_title` entry under `[app_state]`

#### Scenario: Existing config file is migrated
- **WHEN** AoE starts with an existing `config.toml` that still contains `[app_state].dynamic_tab_title`
- **THEN** the migration system SHALL remove that field from the persisted config file

#### Scenario: Settings UI is shown
- **WHEN** the user opens General settings
- **THEN** the settings list SHALL NOT include a `Dynamic Tab Title` field

## TUI Settings Requirement

Every configurable field MUST be editable in the Settings TUI. Adding a new field requires:
1. A `FieldKey` variant in `src/tui/settings/fields.rs`
2. A `SettingField` entry in the corresponding `build_*_fields()` function
3. Wiring in `apply_field_to_global()` and `apply_field_to_profile()`
4. A `clear_profile_override()` case in `src/tui/settings/input.rs`
5. The override field in the corresponding `*ConfigOverride` struct with merge logic

## Functional Requirements

- **FR-001**: Global config MUST be stored as TOML at the platform-appropriate app directory.
- **FR-002**: Missing config files MUST be treated as "all defaults" without error.
- **FR-003**: Profile configs MUST be stored under `profiles/<name>/` within the app directory.
- **FR-004**: Repo configs MUST be loaded from `.aoe/config.toml` relative to the project path.
- **FR-005**: Config merging MUST be per-field, not per-section (each field independently resolved).
- **FR-006**: `app_state` fields (UI state, window widths, sort order) MUST be saved without user action as the user interacts with the TUI.
- **FR-007**: The `tmux.status_bar = "auto"` mode MUST disable the status bar when the user has an existing `~/.tmux.conf` or `~/.config/tmux/tmux.conf`.
- **FR-008**: The `tmux.mouse = "auto"` mode MUST enable mouse for users without custom tmux config, and leave it untouched for those who do.

## Success Criteria

- **SC-001**: A fresh install with no config file uses sensible defaults for all settings.
- **SC-002**: Profile-level overrides apply only to sessions under that profile.
- **SC-003**: Repo-level config overrides profile and global for sessions in that project.
- **SC-004**: All settings are accessible and editable through the TUI without manual file editing.
