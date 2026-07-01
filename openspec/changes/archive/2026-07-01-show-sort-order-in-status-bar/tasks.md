## 1. Build the right-aligned sort segment

- [x] 1.1 In `src/tui/home/render.rs` `render_status_bar`, build the sort-indicator spans: `o` in the key style, `Sort: <SortOrder::label()>` in the description style
- [x] 1.2 Append `· J/K Move` (with `J/K` in the key style) only when `self.sort_order == SortOrder::Manual`

## 2. Right-align via a two-region split

- [x] 2.1 Compute the display width of the sort segment (ASCII, so char count) plus a small margin
- [x] 2.2 Split the status-bar `area` with a horizontal `Layout` `[Min(0), Length(width)]`
- [x] 2.3 Render the existing hint spans into the left chunk (unchanged) and the sort segment into the right chunk; both keep `bg(theme.selection)`
- [x] 2.4 Verify the left hints truncate at the boundary rather than overlapping the right segment

## 3. Tests

- [x] 3.1 Add a home-view render/unit test asserting the status bar contains the sort label for a non-manual order (e.g. `Newest`) and does NOT contain the `J/K` move hint
- [x] 3.2 Add a test asserting that in `Manual` sort the status bar contains both the `Manual` label and the `J/K` move hint
- [x] 3.3 Add a test asserting the sort label reflects the order after cycling (e.g. after pressing `o`, or by setting the order and re-rendering)

## 4. Validate

- [x] 4.1 Run `cargo fmt`, `cargo clippy --all-targets`, and the home-view tests; ensure all pass
- [x] 4.2 Run `openspec validate show-sort-order-in-status-bar --strict` and fix any issues
