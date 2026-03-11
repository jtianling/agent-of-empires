## Context

The TUI currently uses ratatui for rendering and crossterm for terminal control. The main event loop in `src/tui/app.rs` already has access to `terminal.backend_mut()` for writing escape sequences (used for synchronized updates). Terminal setup/teardown is in `src/tui/mod.rs`. The TUI tracks state through `HomeView` fields (dialogs, settings_view, diff_view, view_mode, etc.), providing all the signals needed to derive the current "title state."

The OSC 0 escape sequence (`\x1b]0;...\x07`) is universally supported by modern terminals (Alacritty, iTerm2, kitty, WezTerm, macOS Terminal.app, Windows Terminal, GNOME Terminal, etc.) for setting the window/tab title. Terminals that don't support it silently ignore the sequence.

## Goals / Non-Goals

**Goals:**
- Update the terminal tab title to reflect TUI state so users can tell at a glance what's happening
- Use distinctive icons/emoji for each state for quick visual identification
- Clean up (reset) the title when the TUI exits
- Allow users to disable the feature via configuration

**Non-Goals:**
- Customizable title format or icons (keep it simple for v1)
- Showing specific session names or detailed info in the title (just the state)
- Supporting terminals that require non-standard title mechanisms
- Padding the title to fixed width (unlike Gemini CLI -- unnecessary for our use case)

## Decisions

### 1. Use OSC 0 escape sequence via direct stdout write

Write `\x1b]0;{title}\x07` directly to stdout using `crossterm::execute!` with `SetTitle` command, or a thin wrapper around `std::io::Write`.

**Rationale**: crossterm already has `crossterm::terminal::SetTitle(title)` which emits the correct OSC sequence. This keeps us within our existing terminal abstraction and handles platform differences.

**Alternative considered**: Writing raw bytes -- lower-level but no real benefit since crossterm already handles this.

### 2. Derive title from TUI state in the event loop

Add a `compute_tab_title()` method that examines the current `HomeView` state and returns the appropriate title string. Call this in the event loop after state changes, before the draw call. Only write to stdout when the title actually changes (deduplicate with a `last_tab_title` field).

**State priority** (highest to lowest):

| Priority | State | Icon | Title |
|----------|-------|------|-------|
| 1 | Dialog open (any confirmation/input dialog) | `✋` | `✋ Input Required - AoE` |
| 2 | Session creating | `⏳` | `⏳ Creating... - AoE` |
| 3 | Settings view | `⚙` | `⚙ Settings - AoE` |
| 4 | Diff view | `📊` | `📊 Diff - AoE` |
| 5 | Home (idle) | `◇` | `◇ AoE` |

**Rationale**: Priority order ensures the most actionable state (needs user input) always wins. Icons match Gemini CLI's style -- distinctive and recognizable at small sizes.

**Alternative considered**: More granular states (per-dialog-type icons) -- rejected for simplicity in v1.

### 3. New module `src/tui/tab_title.rs`

A small, focused module containing:
- `TabTitleState` enum matching the states above
- `fn compute_title(state: TabTitleState) -> String` -- pure function, easy to test
- `fn set_terminal_title(writer: &mut impl Write, title: &str) -> io::Result<()>` -- writes the escape sequence
- `fn clear_terminal_title(writer: &mut impl Write) -> io::Result<()>` -- resets title on exit

**Rationale**: Keeps title logic isolated from the main app. Pure `compute_title` function is trivially testable.

### 4. Configuration: `dynamic_tab_title` in `[app_state]` section (global only)

Add `dynamic_tab_title: bool` (default `true`) to `AppStateConfig`. This is a personal UI preference, so global-only (not overridable per profile/repo), similar to `theme`.

**Rationale**: Matches existing pattern for UI-preference settings. Default-on because the feature is useful and harmless (terminals that don't support it ignore the escape codes).

### 5. Cleanup on exit

Reset the title by writing an empty OSC sequence (`\x1b]0;\x07`) during terminal teardown in `src/tui/mod.rs`, alongside the existing `LeaveAlternateScreen` cleanup.

**Rationale**: Prevents stale AoE titles from lingering in the tab after exit. The empty title restores the terminal's default behavior.

## Risks / Trade-offs

- **[Risk] Title escape sequences leak into piped output** -- Mitigated: The TUI already requires an interactive terminal (raw mode + alternate screen). Title setting only runs within the TUI context, never in CLI mode.
- **[Risk] Performance impact from frequent title writes** -- Mitigated: Deduplication (only write when title changes) means at most a few writes per user interaction, not per frame.
- **[Risk] Some terminal multiplexers (tmux, screen) may not propagate titles** -- Mitigated: tmux propagates OSC titles when `set -g set-titles on` is configured. We won't try to force this. Users in tmux can disable the feature if it doesn't work for their setup.
- **[Trade-off] Default-on may surprise users** -- Accepted: The feature is helpful and non-destructive. Easy to disable in settings.
