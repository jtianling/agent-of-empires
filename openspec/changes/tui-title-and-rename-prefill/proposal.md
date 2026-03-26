## Why

Two small UX improvements: the main TUI title is unnecessarily long, and the rename dialog forces users to retype the current title from scratch instead of editing it.

## What Changes

- Shorten the TUI home screen title from `Agent of Empires [profile]` to `AoE [profile]`
- Pre-fill the "New title" field in the rename dialog with the current session title so users can edit rather than retype

## Capabilities

### New Capabilities

_None_

### Modified Capabilities

- `tui`: Title display text changes from "Agent of Empires" to "AoE"

## Impact

- `src/tui/home/render.rs`: title format string change
- `src/tui/dialogs/rename.rs`: initialize `new_title` with current title instead of empty
- Existing rename dialog tests will need updating to reflect pre-filled title
