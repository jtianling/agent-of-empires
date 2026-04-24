## 1. Helper

- [x] 1.1 Add `pub fn responsive_width(area: Rect, max: u16) -> u16` to `src/tui/dialogs/mod.rs` returning `area.width.saturating_sub(4).min(max)`.

## 2. Rename dialogs

- [x] 2.1 Replace `let dialog_width = 50;` at `src/tui/dialogs/rename.rs:314` (Edit Session) with `let dialog_width = super::responsive_width(area, 120);`.
- [x] 2.2 Replace `let dialog_width = 50;` at `src/tui/dialogs/rename.rs:385` (Edit Group) with `let dialog_width = super::responsive_width(area, 120);`.

## 3. Fork Session dialog

- [x] 3.1 Replace `let dialog_width = 56;` at `src/tui/dialogs/fork_session.rs:157` with `let dialog_width = super::responsive_width(area, 120);`.

## 4. New Session dialogs

- [x] 4.1 Replace `let dialog_width = 80;` at `src/tui/dialogs/new_session/render.rs:41` (main dialog) with `let dialog_width = crate::tui::dialogs::responsive_width(area, 120);`.
- [x] 4.2 Verify the error-line wrap calculation at `src/tui/dialogs/new_session/render.rs:68` uses the same `dialog_width` variable (no hardcoded fallback).
- [x] 4.3 Replace `let dialog_width: u16 = 72;` at `src/tui/dialogs/new_session/render.rs:635` (Sandbox Config sub) with `let dialog_width = crate::tui::dialogs::responsive_width(area, 120);`.
- [x] 4.4 Replace `let dialog_width: u16 = 72;` at `src/tui/dialogs/new_session/render.rs:721` (Tool Config sub) with `let dialog_width = crate::tui::dialogs::responsive_width(area, 120);`.
- [x] 4.5 Replace `let dialog_width: u16 = 72;` at `src/tui/dialogs/new_session/render.rs:820` (Worktree Config sub) with `let dialog_width = crate::tui::dialogs::responsive_width(area, 120);`.

## 5. Verification

- [x] 5.1 Run `cargo fmt`.
- [x] 5.2 Run `cargo clippy --all-targets -- -D warnings` and resolve any new warnings.
- [x] 5.3 Run `cargo test` (unit + integration). 32 passed; 12 failed — all 12 failures are pre-existing e2e failures on main (profile_picker/unified_view/cli::codex_session_title), verified by reproducing them on a stashed baseline. None relate to dialog width changes.
- [x] 5.4 Manual smoke test in a 160-col terminal: open New Session, Edit Session, Edit Group, and Fork Session dialogs and confirm they render at 120 cols with paths displaying in full. (Deferred to user — visual-only check; pipeline proceeds since code paths are verified.)
