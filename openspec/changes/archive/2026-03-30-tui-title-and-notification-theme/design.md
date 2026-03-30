## Context

The TUI left-panel title currently reads `" Agent of Empires [{profile}] "` (in `src/tui/home/render.rs:131`), which is verbose and truncates profile names on narrower terminals. The terminal tab title already uses the shorter `AoE[{profile}]` format (`src/tui/tab_title.rs:11`).

The tmux notification bar (`src/tmux/status_bar.rs`) uses hardcoded 256-color palette values (colour46, colour220, colour245, colour252) that were chosen independently of the TUI theme. The TUI uses a comprehensive theme system (`src/tui/styles.rs`) with RGB values for the Empire palette, but this palette is not reflected in the tmux status bar.

## Goals / Non-Goals

**Goals:**
- Shorten TUI panel title to "AoE" for better profile name visibility
- Align tmux notification bar colors with AoE's Empire theme palette
- Maintain visual consistency between TUI and tmux status bar

**Non-Goals:**
- Making tmux notification bar colors theme-switchable (follow current theme dynamically)
- Changing the notification bar layout or content format
- Modifying the terminal tab title format (already uses "AoE")

## Decisions

### D1: Title string change
Change `" Agent of Empires [{profile}] "` to `" AoE [{profile}] "` in `src/tui/home/render.rs`. This matches the terminal tab title convention and frees ~15 characters for the profile name.

**Alternative considered**: Using just the profile name without "AoE" prefix. Rejected because the prefix provides context when glancing at the TUI.

### D2: Use tmux hex color syntax for exact theme match
Modern tmux (3.2+) supports `#[fg=#rrggbb]` hex color syntax. Use hex values directly from the Empire theme instead of approximating with 256-color palette indices.

Color mapping from Empire theme (`src/tui/styles.rs`):

| Element | Current tmux color | New hex color | Theme token |
|---------|-------------------|---------------|-------------|
| Session index | `colour46` (bright green) | `#22c55e` | `running` |
| Session title | `colour252` (light gray) | `#cbd5e1` | `text` |
| Hint text ("Ctrl+b d") | `colour245` (gray) | `#94a3b8` | `hint` |
| Notification (waiting) | `colour220` (yellow) | `#fbbf24` | `waiting` |
| From-title text | `colour245` (gray) | `#64748b` | `dimmed` |

**Alternative considered**: Keep 256-color values and find closest matches. Rejected because hex colors give exact theme parity and modern tmux supports them.

### D3: Update notification-bar spec to be color-agnostic
The existing spec hardcodes `colour220` and `colour245`. Update to reference theme-consistent colors by role (e.g., "waiting color", "hint color") rather than specific tmux color codes, since the actual values come from the theme.

## Risks / Trade-offs

- **[Risk] Older tmux versions may not support hex colors** -> Mitigation: tmux 3.2+ is widely available (released 2021). AoE already requires modern tmux features. This is acceptable.
- **[Risk] Users may have custom tmux themes that clash** -> Mitigation: AoE already sets its own status bar styles per-session; this just changes the foreground colors to better defaults.
