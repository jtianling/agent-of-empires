## Why

AoE already isolates each launch directory into its own profile (`resolve_profile(None)` -> `auto-<dir>-<hash>`) with its own session store. But all profiles still share one tmux socket, so a second AoE (e.g. tests, or an AoE started in another directory) can see and disrupt the live AoE's sessions. The just-shipped global `tmux_socket_name` config tried to address this as an opt-in, but there is no real use case for forcing all profiles onto one server: AoE's session cycling/switch/status all operate within a single profile, and cross-profile session sharing is never needed. Binding the socket to the profile instead makes isolation the automatic default and removes the need for manual configuration.

## What Changes

- **BREAKING**: Remove the global `TmuxConfig.tmux_socket_name` config field and its settings-TUI surface (`FieldKey::TmuxSocketName`, the `build_tmux_fields` entry, `apply_field_to_global` case, and the `clear_profile_override` no-op case).
- Derive the tmux socket name from the resolved profile name (identical string, e.g. profile `auto-agent-of-empires-3f2a` -> socket `-L auto-agent-of-empires-3f2a`). Each profile/directory therefore gets its own tmux server automatically.
- Change socket resolution so the value comes from an explicit profile only (the main TUI inits it from its resolved profile; subcommands carrying a profile arg/`AGENT_OF_EMPIRES_PROFILE` env init from that). When no profile is resolvable, fall back to a bare `tmux` command (ambient `$TMUX`) instead of forcing a wrong `-L`. This keeps in-server helpers (`aoe tmux status` from the user's `.tmux.conf`, the codex-title monitor, the record-pane hook) correct without any config.
- **Preserve the `#[cfg(test)]` safety net unchanged**: under test the socket is still forced to `aoe_test_<pid>` BEFORE any bare fallback, so the full suite can never touch a live socket.
- Guard against overlong socket paths (unix domain socket path limit ~104 chars on macOS): if the derived name would exceed the limit, fall back to a hash-based short name.
- **BREAKING (one-time)**: On upgrade, existing sessions on the bare default socket become invisible to the new binary (it now looks at `-L <profile>`); they appear as gone and go through cold-start recovery. Document this; warn before installing.

## Capabilities

### New Capabilities
- `socket-per-profile`: AoE derives its tmux socket name from the resolved profile so each profile/directory runs on its own tmux server, with a safe bare-`tmux` fallback for in-server helpers that have no profile, and the test-only private socket preserved.

### Modified Capabilities
- `tmux-socket-name`: REMOVED. The configurable global socket name is superseded by profile-derived sockets; the config field, its requirements, and its settings-TUI surface are removed.

## Impact

- `src/session/config.rs`: remove `TmuxConfig.tmux_socket_name`.
- `src/main.rs`: init the socket from the resolved profile (and at the right points for the profile-carrying subcommands), not from config.
- `src/tmux/mod.rs`: rework `resolved_socket_name()` (explicit/env profile -> `-L <profile>`; test -> `aoe_test_<pid>`; else None -> bare); add overlong-name fallback; update unit tests.
- `src/tui/settings/fields.rs` + `src/tui/settings/input.rs`: remove the `TmuxSocketName` field, apply case, and clear-override case, plus its tests.
- `openspec/specs/tmux-socket-name/`: removed via delta; `openspec/specs/tmux-test-isolation/` unchanged.
- Behavior change for users with `tmux_socket_name` set (field disappears) and a one-time orphaning of default-socket sessions on first launch of the new binary.
