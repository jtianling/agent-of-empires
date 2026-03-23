## 1. TUI state tracking

- [x] 1.1 Add `session_before_tui: Option<String>` field to `App` struct in `src/tui/app.rs`
- [x] 1.2 Populate `session_before_tui` in `try_restore_selection_from_client_context` -- read `last_detached_session` value before it is consumed and store it on `self`

## 2. Expose tmux helpers

- [x] 2.1 Make `set_previous_session_for_client` pub in `src/tmux/utils.rs`
- [x] 2.2 Make `set_target_from_title` pub in `src/tmux/utils.rs`

## 3. Conditional set-or-clear in attach

- [x] 3.1 Replace unconditional `clear_from_title` + `clear_previous_session_for_client` in `attach_to_session` with conditional logic: if `session_before_tui` is Some and differs from target, call set helpers; otherwise call clear helpers

## 4. Spec update

- [x] 4.1 Apply delta spec to `openspec/specs/session-back-toggle/spec.md` -- replace "TUI entry clears stale" requirements with "TUI entry sets from source context" requirements, update scenarios on "Ctrl+b b toggles to previous session"

## 5. Verification

- [x] 5.1 Run `cargo check`, `cargo clippy`, `cargo test`
- [ ] 5.2 Manual test: nested mode -- Session A -> TUI -> Session B, verify Ctrl+b b returns to A
- [ ] 5.3 Manual test: non-nested mode -- same flow
- [ ] 5.4 Manual test: first launch (no source) -- verify Ctrl+b b is no-op
- [ ] 5.5 Manual test: re-enter same session -- verify Ctrl+b b is no-op
