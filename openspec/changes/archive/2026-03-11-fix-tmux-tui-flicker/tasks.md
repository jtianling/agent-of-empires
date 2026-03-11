## 1. TUI Infrastructure

- [x] 1.1 Add `BeginSynchronizedUpdate` and `EndSynchronizedUpdate` around `terminal.draw()` calls in `src/tui/app.rs`.
- [x] 1.2 Refactor `App::run` in `src/tui/app.rs` to remove immediate `terminal.draw()` calls from `handle_key` and `handle_mouse`, centralizing them in the main loop logic.
- [x] 1.3 Audit and remove redundant `terminal.clear()` calls, ensuring it is only used when the terminal state is likely corrupted or after full-screen process switches.

## 2. Refresh & Rendering Optimization

- [x] 2.1 Tune the preview refresh logic in `src/tui/home/render.rs` and `src/tui/home/mod.rs` to ensure it correctly triggers redraws only when cache actually updates.
- [x] 2.2 Verify that dialogs use the `Clear` widget correctly for overlaying content without disrupting the background more than necessary.

## 3. Verification

- [x] 3.1 Run `aoe` in a `tmux` session and verify that rapid navigation (holding `j`/`k`) does not cause flickering.
- [x] 3.2 Verify that typing in the `Custom Instruction` dialog is smooth and flicker-free in `tmux`.
- [x] 3.3 Verify that background status updates do not cause full-screen blinks.
- [x] 3.4 Verify that attaching and detaching from a session correctly restores the TUI without double-clearing the screen.
