## Context

The TUI dialogs evolved organically. Each dialog render function hard-codes a `dialog_width` constant (50, 56, 72, or 80) chosen when the dialog first shipped. These widths predate nested group paths and don't use the space users have on typical 150+ column terminals. Truncation happens silently because `Paragraph::new(Line::from(spans))` just clips the right edge.

Relevant current call sites:

- `src/tui/dialogs/new_session/render.rs:41` — New Session main, 80
- `src/tui/dialogs/new_session/render.rs:635` — Sandbox Config sub, 72
- `src/tui/dialogs/new_session/render.rs:721` — Tool Config sub, 72
- `src/tui/dialogs/new_session/render.rs:820` — Worktree Config sub, 72
- `src/tui/dialogs/rename.rs:314` — Edit Session, 50
- `src/tui/dialogs/rename.rs:385` — Edit Group, 50
- `src/tui/dialogs/fork_session.rs:157` — Fork Session, 56

All seven sites already pass `area: Rect` into their render functions and route through `crate::tui::dialogs::centered_rect(area, dialog_width, dialog_height)`, which internally clamps to `area` size, so no site currently overflows.

## Goals / Non-Goals

**Goals:**
- Long group paths (`work/frontend/xxx`) and filesystem paths display in full on normal terminals.
- Dialogs use available terminal width up to a readable cap.
- One helper, one call pattern, applied uniformly across dialog sites.
- No behavior change to field layout, inputs, focus order, or keybindings.

**Non-Goals:**
- No changes to field-level truncation/scrolling logic inside inputs.
- No redesign of dialog content, field order, or padding.
- No per-field width adjustments (e.g., stretching the Path input specifically).
- No attempt to preserve usability on phone-class terminals (< ~60 cols) — user explicitly accepted degraded behavior there.

## Decisions

### Decision 1: Single helper in `src/tui/dialogs/mod.rs`

```rust
pub fn responsive_width(area: Rect, max: u16) -> u16 {
    area.width.saturating_sub(4).min(max)
}
```

- `saturating_sub(4)` prevents underflow on tiny terminals and leaves room for borders/margins.
- Cap is an argument, not a constant, so sub-dialogs can pick a smaller cap in the future if needed (though all seven sites will use 120 for now).
- `min(max)` after `saturating_sub` means on terminals ≥ 124 cols the cap applies; below that, the dialog scales down.

**Alternative considered**: Per-dialog `const DIALOG_WIDTH: u16 = 120;` plus inlined clamp logic. Rejected — it reintroduces the hard-coded constant drift this change is removing.

**Alternative considered**: Make the helper return `Rect` directly (combine with `centered_rect`). Rejected — `centered_rect` is also called from non-dialog sites, and separating width computation from centering keeps each helper small.

### Decision 2: Cap at 120 columns

120 is wide enough for deep group paths (`work/clients/big-company-name/frontend/feature-x`) and common macOS workspace paths, and narrow enough that lines remain comfortable to read. User confirmed this number in discussion.

### Decision 3: Update the error-wrap calculation in `new_session/render.rs`

The error line count computation at `render.rs:68` uses `dialog_width - 4` to compute the inner width for line wrapping. After the switch, the computation must use the same responsive width variable, not the pre-existing constant, or error messages will wrap at the wrong boundary.

### Decision 4: No spec changes to existing requirements

The new responsive-width requirement is an **ADDED** requirement in the `tui` capability. Existing `tui` requirements (session list display, title updates, etc.) are untouched. This keeps the archive-time diff minimal.

## Risks / Trade-offs

- [Risk: Dialog that was visually balanced at 72 now looks empty at 120] → Mitigation: visual check during implementation; accept the tradeoff since truncation is a worse problem than whitespace.
- [Risk: A dialog's internal `Layout::Constraint` rows assume a specific inner width for column alignment] → Mitigation: grep the affected files for hard-coded column offsets; rely on ratatui's flex layout which handles width changes cleanly.
- [Risk: Snapshot / e2e tests capture specific dialog dimensions] → Mitigation: run `cargo test` after changes; update snapshots only if the test was asserting on width specifically, not on content.
- [Risk: Terminals between 60 and 124 cols now see dialogs at a non-standard width every time] → Accepted: users on those widths already experienced the clamp, now they also experience dynamic scaling. Behavior is still bounded and correct.

## Open Questions

None. User has confirmed the cap (120) and the degradation policy (narrow terminals accepted as-is).
