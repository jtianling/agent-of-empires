## 1. Core Tab Title Module

- [x] 1.1 Create `src/tui/tab_title.rs` with `TabTitleState` enum (InputRequired, Creating, Settings, Diff, Idle) and `compute_title(state) -> String` function
- [x] 1.2 Implement `set_terminal_title(writer, title)` and `clear_terminal_title(writer)` using crossterm's `SetTitle` command
- [x] 1.3 Add unit tests for `compute_title` covering all states and their expected icon+text output

## 2. Configuration

- [x] 2.1 Add `dynamic_tab_title: bool` field to `AppStateConfig` (default `true`) with serde default
- [x] 2.2 Add `FieldKey::DynamicTabTitle` variant and wire into settings TUI (General tab): `build_general_fields()`, `apply_field_to_global()`
- [x] 2.3 Verify missing field in existing config.toml defaults to `true` (deserialization test)

## 3. TUI Integration

- [x] 3.1 Add `last_tab_title: String` field to `App` struct for deduplication
- [x] 3.2 Add method on `HomeView` (or `App`) to derive `TabTitleState` from current state (check dialogs, settings_view, diff_view, creation_poller)
- [x] 3.3 Integrate title update into the event loop: compute state, compare with `last_tab_title`, write if changed (before `BeginSynchronizedUpdate`)
- [x] 3.4 Pass `dynamic_tab_title` config value into the TUI so title updates are skipped when disabled
- [x] 3.5 Set initial title on TUI startup (after terminal setup)

## 4. Cleanup

- [x] 4.1 Add `clear_terminal_title` call to terminal teardown in `src/tui/mod.rs` (before `LeaveAlternateScreen`)
- [x] 4.2 Add title reset to the panic hook cleanup path
- [x] 4.3 Ensure title is only written when TUI is active (not in CLI-only mode)

## 5. Validation

- [x] 5.1 Run `cargo fmt`, `cargo clippy`, `cargo test` and fix any issues
- [x] 5.2 Manual test: launch aoe in Alacritty, verify tab title changes when opening dialogs, settings, diff view, and returns to idle on close
