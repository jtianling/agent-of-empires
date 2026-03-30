## Why

The TUI panel title "Agent of Empires [profile]" is unnecessarily long, causing profile names to be truncated on narrower terminals. Shortening to "AoE [profile]" matches the terminal tab title format and gives more room for the profile name. Additionally, the tmux notification bar uses hardcoded 256-color values (colour220, colour46, colour252, colour245) that don't match the AoE TUI theme palette, creating visual inconsistency between the TUI and the tmux status bar.

## What Changes

- Shorten the TUI left-panel title from `" Agent of Empires [{profile}] "` to `" AoE [{profile}] "` in the home screen render
- Update tmux status-bar notification colors in `STATUS_LEFT_FORMAT` to use values that align with the Empire theme palette (amber for notifications, teal/green for index, cool gray for text, slate for hints)

## Capabilities

### New Capabilities

_(none)_

### Modified Capabilities

- `notification-bar`: Update default color values to align with the AoE TUI theme palette instead of using arbitrary tmux 256-color values
- `tui`: Shorten the left-panel title format from "Agent of Empires" to "AoE"

## Impact

- `src/tui/home/render.rs`: Title format string change
- `src/tmux/status_bar.rs`: `STATUS_LEFT_FORMAT` color values
- `openspec/specs/notification-bar/spec.md`: Update colour references to match new theme-aligned values
- `openspec/specs/tui/spec.md`: Update title format reference if present
