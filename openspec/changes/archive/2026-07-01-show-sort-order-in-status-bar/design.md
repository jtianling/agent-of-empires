## Context

`render_status_bar` in `src/tui/home/render.rs` builds one left-to-right `Line`
of key-hint spans and renders it as a single left-aligned `Paragraph` into the
status-bar `area`. It shows navigation/action keys but neither the sort order nor
the `o` cycle key (both live only in the `?` help overlay). `SortOrder::label()`
already maps each variant to a short string (`Newest`/`Oldest`/`A-Z`/`Z-A`/
`Manual`). The view holds `self.sort_order`.

## Goals / Non-Goals

**Goals:**
- Show `o Sort: <label>` right-aligned at the far right of the status bar.
- When `sort_order == Manual`, append `· J/K Move`.
- Never overlap the left hints, including on narrow terminals.

**Non-Goals:**
- No change to sort behavior, key handling, or the `J/K`/`o` semantics.
- Not reconciling the stale `tui` keybinding table row (`Tab` vs `o`) -- out of
  scope for this display-only change.
- No new config field.

## Decisions

**Decision: Split the status-bar rect into left (flex) + right (fixed) regions.**
Use a horizontal `Layout` on `area` with constraints `[Min(0), Length(w)]`, where
`w` is the display width of the right sort segment (plus a small margin). Render
the existing hint spans into the left chunk (unchanged, left-aligned) and the
sort segment into the right chunk. Because the left chunk is bounded, its hints
truncate at the boundary instead of colliding with the sort segment. Chosen over
overlapping two paragraphs on the same rect (approach "甲"), which can collide on
narrow terminals where the left hints are longer than the available width. The
sort labels and hint text are ASCII, so `str::len()` equals display width and `w`
is trivial to compute.

**Decision: Compose the right segment from styled spans.**
`o` uses the accent key style; `Sort: <label>` uses the dimmed description style,
matching the existing hint styling. The `· J/K Move` suffix is appended only in
`Manual`, with `J/K` in the key style. Both regions share the same
`bg(theme.selection)` so the bar looks continuous.

**Decision: Compute width from the actual rendered string.**
Build the right segment first, measure its width, then derive the layout
constraint from it. This keeps the fixed region exactly as wide as the content
(Manual is widest because of the appended move hint), so the left region always
gets the maximum remaining space.

## Risks / Trade-offs

- [Very narrow terminal where even the right segment does not fit] -> The right
  chunk is clamped by the layout to the available width and the paragraph
  truncates; no panic, worst case the sort label is clipped. Acceptable for
  extreme widths already degraded by the existing hint list.
- [Width drift if labels ever become non-ASCII] -> Current labels are ASCII; if
  that changes, switch the width computation to a unicode-width measure. Noted,
  not guarded now (YAGNI).
