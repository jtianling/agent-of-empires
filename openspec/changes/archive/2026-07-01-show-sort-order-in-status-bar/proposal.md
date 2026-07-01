## Why

The session list has five sort modes (Newest, Oldest, A-Z, Z-A, Manual) but the
home-view status bar shows none of them, and it does not even hint the `o` key
that cycles them. Worse, `J`/`K` reorder only works in Manual sort, so in any
other mode those keys silently do nothing. With no on-screen sort indicator, this
reads as a broken feature (a real confusion this week: a session reorder looked
"broken" purely because there was no way to see the list was in Newest, not
Manual). The state and its toggle key need to be visible.

## What Changes

- Add a right-aligned segment to the home-view status bar showing the current
  sort order and the key that cycles it, e.g. `o Sort: Newest`.
- When the sort order is Manual, append a `· J/K Move` hint so the reorder
  capability (only active in Manual) is discoverable exactly when it applies.
- Right-align the segment by splitting the status-bar rect into a flexible left
  region (existing hints, truncates first on narrow terminals) and a fixed-width
  right region (the sort segment), so the two never overlap.
- No behavior change to sorting or key handling; this is display only.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `tui`: the home-view status bar SHALL display the current sort order and its
  cycle key, right-aligned, and SHALL surface the `J/K` move hint when (and only
  when) the sort order is Manual.

## Impact

- `src/tui/home/render.rs`: `render_status_bar` gains a right-aligned sort
  segment via a two-region split of the status-bar rect.
- `openspec/specs/tui/spec.md`: new requirement for the status-bar sort
  indicator.
- No config schema change, no data migration, no keybinding change.
