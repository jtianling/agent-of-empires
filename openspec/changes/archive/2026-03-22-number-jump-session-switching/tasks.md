## 1. CLI: Add --index parameter to switch-session

- [x] 1.1 Add `--index` option to `aoe tmux switch-session` CLI subcommand (mutually exclusive with `--direction`)
- [x] 1.2 Implement `switch_aoe_session_by_index(index, profile, client_name)` in `src/tmux/utils.rs` using `ordered_profile_session_names()` (global, not group-scoped) to resolve 1-based index to tmux session name

## 2. Tmux keybindings: Number jump via key tables

- [x] 2.1 Add 1-9 prefix bindings in `setup_session_cycle_bindings()` that `switch-client -T aoe-N`
- [x] 2.2 Create aoe-1 through aoe-9 key tables with Space (single digit confirm) + 0-9 (two-digit auto-confirm) bindings, each calling `aoe tmux switch-session --index N`
- [x] 2.3 Add 1-9 prefix bindings in `apply_managed_session_bindings()` with profile-aware switch commands (nested mode override)
- [x] 2.4 Clean up 1-9 prefix bindings and aoe-N key table bindings in `cleanup_session_cycle_bindings()`
- [x] 2.5 Clean up 1-9 and aoe-N bindings in `cleanup_nested_detach_binding()`
- [x] 2.6 Update tmux status bar hint in `src/tmux/status_bar.rs` to include number jump info

## 3. TUI: Numeric index display in session list

- [x] 3.1 Compute a session-index map (`HashMap<flat_items index, session number>`) during `render_list()` by iterating `flat_items` and assigning 1-based numbers to `Item::Session` entries only, skipping `Item::Group`, max 99
- [x] 3.2 Render the numeric index as a right-aligned 2-char prefix before the status icon in `render_item()`, blank for groups

## 4. TUI: Pending jump state and digit key handling

- [x] 4.1 Add `PendingJump` struct (first_digit: u8) and `pending_jump: Option<PendingJump>` field to `HomeView`
- [x] 4.2 Handle digit keys 1-9 in `handle_key()`: if no pending jump, set pending_jump with first digit; if pending jump exists, form two-digit number and execute jump
- [x] 4.3 Handle Space key: if pending jump exists, execute single-digit jump and clear pending state
- [x] 4.4 Handle cancel: any non-digit, non-Space key clears pending state (Esc, Enter, letters, etc.)
- [x] 4.5 Skip pending jump logic when dialogs, search, settings, help, or diff views are active
- [x] 4.6 Add visual indicator for pending state: show `jump: N_` in status bar area while pending

## 5. Testing and cleanup

- [x] 5.1 Add unit tests for `switch_aoe_session_by_index()` index resolution logic
- [x] 5.2 Run `cargo fmt`, `cargo clippy`, `cargo test` to verify no regressions
