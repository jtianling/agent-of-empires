## 1. Remove terminal session structs and tmux module code

- [x] 1.1 Delete `src/tmux/terminal_session.rs` entirely (TerminalSession, ContainerTerminalSession, build_terminal_create_args)
- [x] 1.2 Remove `mod terminal_session` and `pub use terminal_session::*` from `src/tmux/mod.rs`, remove `TERMINAL_PREFIX` and `CONTAINER_TERMINAL_PREFIX` constants
- [x] 1.3 In `src/tmux/utils.rs`, remove TerminalSession and ContainerTerminalSession references from `matches_managed_tmux_name` (keep only Session::generate_name match)

## 2. Remove terminal from session model and instance

- [x] 2.1 Remove `TerminalInfo` struct and `terminal_info` field from Instance in `src/session/mod.rs` (or wherever defined). Ensure serde still tolerates the field in old JSON via `#[serde(default)]` or by not using `deny_unknown_fields`
- [x] 2.2 Remove terminal-related methods from `src/session/instance.rs`: `terminal_tmux_session()`, `container_terminal_tmux_session()`, `has_terminal()`, `start_terminal()`, `start_terminal_with_size()`, `apply_terminal_tmux_options()`, `refresh_terminal_tmux_options()`, `refresh_container_terminal_tmux_options()`, and any container terminal start/create methods

## 3. Remove terminal from configuration

- [x] 3.1 Remove `DefaultTerminalMode` enum and `default_terminal_mode` field from `SandboxConfig` in `src/session/config.rs`
- [x] 3.2 Remove `default_terminal_mode` from `SandboxConfigOverride` in `src/session/profile_config.rs` and update merge logic
- [x] 3.3 Remove the `DefaultTerminalMode` / `FieldKey::DefaultTerminalMode` setting from `src/tui/settings/fields.rs` and related apply/clear logic in `src/tui/settings/input.rs`

## 4. Remove terminal from TUI home view

- [x] 4.1 Remove `ViewMode` enum and `view_mode` field from HomeView in `src/tui/home/mod.rs`. Remove `TerminalMode` enum, `terminal_modes` map, `default_terminal_mode` field, and all terminal mode methods (`get_terminal_mode`, `toggle_terminal_mode`)
- [x] 4.2 Remove terminal preview caches (`terminal_preview_cache`, `container_terminal_preview_cache`) and their refresh methods from `src/tui/home/mod.rs`
- [x] 4.3 Remove `start_terminal_for_instance_with_size` and `start_container_terminal_for_instance_with_size` methods from HomeView
- [x] 4.4 In `src/tui/home/input.rs`: remove `t` key handler (ViewMode toggle), remove `c` key handler (TerminalMode toggle), simplify Enter handler to always dispatch `Action::AttachSession` (remove ViewMode::Terminal branch and AttachTerminal action)
- [x] 4.5 In `src/tui/home/render.rs`: remove all ViewMode::Terminal rendering branches, remove TerminalMode display ([host]/[container] labels), remove terminal preview rendering. Simplify to always render agent view.

## 5. Remove terminal from TUI app

- [x] 5.1 In `src/tui/app.rs`: remove `Action::AttachTerminal` variant and `attach_terminal()` method. Remove the TerminalMode import.
- [x] 5.2 Remove any terminal-related session cleanup in session deletion paths (or keep minimal cleanup for orphaned aoe_term_* sessions during transition)

## 6. Update tests

- [x] 6.1 Remove or update tests in `src/tmux/terminal_session.rs` (file deleted), `src/tui/home/tests.rs` (remove terminal-related test cases like `test_select_session_by_managed_tmux_name_matches_terminal_session`)
- [x] 6.2 Run `cargo test`, `cargo clippy`, `cargo fmt` and fix any remaining compilation errors or warnings
