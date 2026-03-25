## 1. Remove nested-only functions from src/tmux/utils.rs

- [x] 1.1 Delete `setup_nested_detach_binding()` function
- [x] 1.2 Delete `cleanup_nested_detach_binding()` function
- [x] 1.3 Delete `apply_managed_session_bindings()` function
- [x] 1.4 Delete `refresh_bindings()` function
- [x] 1.5 Delete `store_client_attach_context()` function
- [x] 1.6 Delete `detach_run_shell_cmd()` helper
- [x] 1.7 Delete `back_toggle_run_shell_cmd_from_option()` helper
- [x] 1.8 Delete `cycle_run_shell_cmd()` helper
- [x] 1.9 Delete `index_jump_run_shell_cmd_from_option()` helper
- [x] 1.10 Delete constants: `NESTED_DETACH_HOOK`, `AOE_ORIGIN_PROFILE_OPTION_PREFIX`, `AOE_RETURN_SESSION_OPTION_PREFIX`
- [x] 1.11 Remove any now-unused imports in utils.rs

## 2. Simplify attach flow in src/tmux/session.rs

- [x] 2.1 Remove the `switch-client` branch in `attach_with_client()` (the `if std::env::var("TMUX").is_ok()` block)
- [x] 2.2 Ensure `attach-session` is always used regardless of TMUX env var
- [x] 2.3 Remove any now-unused imports in session.rs

## 3. Simplify TUI code

- [x] 3.1 Remove TMUX-gated mouse mode save/restore in src/tui/mod.rs
- [x] 3.2 Remove nested-specific cleanup call in src/tui/mod.rs (call to `cleanup_nested_detach_binding`)
- [x] 3.3 Simplify `attach_client_name` in src/tui/app.rs to just use `get_tty_name()` (remove TMUX env var check branch)
- [x] 3.4 Remove or rename any `pending_nested_detach_client` field if it exists

## 4. Remove refresh-bindings CLI subcommand

- [x] 4.1 Remove `refresh-bindings` variant from tmux CLI subcommand enum in src/cli/tmux.rs
- [x] 4.2 Remove the handler/match arm for refresh-bindings in src/cli/tmux.rs

## 5. Update tests

- [x] 5.1 Remove nested-specific e2e test in tests/e2e/cli.rs
- [x] 5.2 Remove nested test helpers from tests/e2e/harness.rs if they become unused
- [x] 5.3 Run `cargo test` to verify all remaining tests pass

## 6. Update documentation

- [x] 6.1 Remove or simplify the "Tmux Nested vs Non-Nested Environments" section in AGENTS.md (which CLAUDE.md symlinks to)
- [x] 6.2 Update the keybinding checklist in AGENTS.md to remove nested-specific steps (b, c, d references to `apply_managed_session_bindings` and `cleanup_nested_detach_binding`)

## 7. Verification

- [x] 7.1 Run `cargo fmt` to ensure formatting is clean
- [x] 7.2 Run `cargo clippy` to verify no warnings from removed code or dangling references
- [x] 7.3 Run `cargo test` to confirm all tests pass
- [x] 7.4 Run `cargo build` to confirm the binary compiles successfully
