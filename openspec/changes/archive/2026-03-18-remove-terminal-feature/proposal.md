## Why

The paired terminal feature (per-agent shell sessions) adds complexity without clear value. Users can split their own terminal emulator when they need a shell alongside an agent. The feature adds UI surface (ViewMode toggle, TerminalMode toggle), config fields, tmux session lifecycle management, and preview caching -- all for something the user's terminal already provides natively.

## What Changes

- **BREAKING**: Remove the Terminal `ViewMode` and all terminal view rendering/input in the TUI
- **BREAKING**: Remove `TerminalMode` (Host/Container) enum and toggle logic
- **BREAKING**: Remove `TerminalSession` and `ContainerTerminalSession` structs and all tmux lifecycle code
- **BREAKING**: Remove `terminal_info` field from session instances
- **BREAKING**: Remove `default_terminal_mode` from sandbox config and settings TUI
- Remove terminal preview caches from the home view
- Remove `t` and `c` key bindings from the TUI home screen
- Remove terminal session creation/attach/kill flows
- Clean up tmux binding setup to stop referencing terminal session prefixes where no longer needed

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `tui`: Remove Terminal ViewMode, `t`/`c` key bindings, terminal preview caches, and terminal attach action
- `sandbox`: Remove `default_terminal_mode` config field and `DefaultTerminalMode` enum
- `session-management`: Remove `terminal_info` / `TerminalInfo` from session instances, remove terminal session lifecycle
- `nested-tmux-detach`: Remove references to `aoe_term_` and `aoe_cterm_` session prefixes in binding/hook logic
- `configuration`: Remove `default_terminal_mode` from sandbox config and profile overrides

## Impact

- **Code**: `src/tmux/terminal_session.rs` (delete entire file), `src/tui/home/` (remove ViewMode::Terminal, TerminalMode, preview caches, key handlers), `src/session/instance.rs` (remove terminal methods), `src/session/config.rs` and `profile_config.rs` (remove terminal mode fields), `src/tui/settings/` (remove terminal mode setting), `src/tmux/utils.rs` (simplify managed session matching)
- **Config**: `default_terminal_mode` field removed from stored config. Existing configs with this field will have it ignored (or cleaned via migration).
- **Data**: `terminal_info` field removed from session JSON. Existing sessions with this field will have it ignored on load.
- **Key bindings**: `t` and `c` freed up for future use in the TUI home screen.
