## Context

The AoE TUI home screen uses a two-panel horizontal layout: session list (left, default 45 cols) + preview pane (right, Min(40) cols). This layout is hardcoded in `src/tui/home/render.rs` and mirrored in `src/tui/app.rs` for preview cache pre-calculation.

On terminals narrower than ~85 columns, both panels get squeezed to the point of being unusable. The preview pane's `Min(40)` constraint steals space from the list, and the list's `max(10)` floor means both panels show truncated, unreadable content.

## Goals / Non-Goals

**Goals:**
- Render only the session list (no preview) when terminal is too narrow for both panels
- Skip preview cache updates when preview is not visible (avoid wasted tmux capture-pane calls)
- Work automatically based on terminal width -- no user configuration needed

**Non-Goals:**
- Adding a manual toggle to show/hide preview (users can attach to see details)
- Making the preview pane responsive or scrollable on narrow screens
- Changing the narrow-screen layout for other views (diff view, settings view)
- Adding a minimum terminal size check or warning dialog

## Decisions

### 1. Threshold: `available_width < list_width + 20`

Use a dynamic threshold relative to the user's configured list width rather than a fixed column count. When the preview pane would get fewer than 20 columns (too narrow to show anything useful), switch to single-panel mode.

Alternative considered: fixed threshold (e.g., `< 80`). Rejected because it doesn't adapt to users who have adjusted their list_width setting.

### 2. Single-panel layout uses full width for list

In narrow mode, the list panel gets `Constraint::Min(0)` (full available width) instead of `Constraint::Length(effective_list_width)`. No preview panel is rendered at all.

Alternative considered: keeping a minimal preview stub. Rejected because on ~40-column screens even a stub is unusable.

### 3. Skip update_caches in narrow mode

In `app.rs`, the pre-render layout calculation mirrors `render.rs` to compute preview dimensions for `update_caches()`. In narrow mode, skip this call entirely since there's no preview to cache. This avoids unnecessary tmux `capture-pane` calls.

### 4. Same threshold logic in app.rs and render.rs

Both files must use the same narrow-screen check to stay in sync. Extract the threshold into a method on `HomeView` (e.g., `is_narrow_layout(available_width: u16) -> bool`) to avoid duplication.

## Risks / Trade-offs

- [Users lose at-a-glance preview on narrow screens] -> Acceptable because preview is unreadable at those widths anyway. Users can still Enter to attach.
- [Threshold may not be perfect for all font/terminal combinations] -> The dynamic threshold adapts to list_width settings, and 20 columns is a conservative minimum for any useful preview content.
