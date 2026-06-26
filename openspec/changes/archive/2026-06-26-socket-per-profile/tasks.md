## 1. Remove the configurable socket-name surface

- [x] 1.1 Remove `tmux_socket_name` field from `TmuxConfig` in `src/session/config.rs` (and any doc comment for it)
- [x] 1.2 Remove `FieldKey::TmuxSocketName` and its `build_tmux_fields` `SettingField` entry in `src/tui/settings/fields.rs`
- [x] 1.3 Remove the `apply_field_to_global` case for `TmuxSocketName` (and `apply_field_to_profile` if present) in `src/tui/settings/fields.rs`
- [x] 1.4 Remove the `clear_profile_override` no-op case for `TmuxSocketName` in `src/tui/settings/input.rs`
- [x] 1.5 Remove the now-obsolete socket-name tests (`test_socket_name_is_global_only_field`, `test_apply_socket_name_normalizes_empty_to_none`)

## 2. Derive the socket from the resolved profile

- [x] 2.1 In `src/tmux/mod.rs`, rework `resolved_socket_name()` to the order: explicit init -> `AGENT_OF_EMPIRES_PROFILE` env -> `#[cfg(test)] aoe_test_<pid>` -> None (bare). Ensure the `#[cfg(test)]` branch precedes the None fallback.
- [x] 2.2 Add an overlong-name guard: if the derived socket name would exceed a safe socket-path length, substitute a deterministic hash-based short name (apply consistently wherever the name is derived).
- [x] 2.3 In `src/main.rs`, replace the `config.tmux.tmux_socket_name` init with profile-derived init: the main TUI inits the socket from its resolved profile (after `resolve_profile`), keeping `AGENT_OF_EMPIRES_PROFILE` set before the server starts.
- [x] 2.4 Ensure profile-carrying subcommands init the socket from their profile: `monitor-notifications` and `switch-session` init from their `--profile` arg; verify `monitor-codex-title` and `record-pane` resolve via inherited env / bare fallback (no cwd re-derivation).

## 3. Tests

- [x] 3.1 Keep/adjust `test_tmux_command_is_private_under_test_even_without_optin` so it still asserts `-L aoe_test_<pid>` under test
- [x] 3.2 Update the builder unit tests in `src/tmux/mod.rs` for the new resolution (profile -> `-L <profile>`; env -> `-L <profile>`; none -> bare)
- [x] 3.3 Add a unit test that a long profile name yields a shortened hash-based socket name
- [x] 3.4 Confirm `tests/tmux_test_isolation_guard.rs` still passes unchanged (no bare `Command::new("tmux")` outside the seam; integration tests pin `-L`)

## 4. OpenSpec spec sync

- [x] 4.1 Verify the change validates: `openspec validate socket-per-profile --strict`
- [x] 4.2 (At archive time) confirm `tmux-socket-name` capability is removed and `socket-per-profile` is added; `tmux-test-isolation` unchanged

## 5. Verify

- [x] 5.1 `cargo fmt`
- [x] 5.2 `cargo clippy` clean
- [x] 5.3 `cargo test --lib` green (1185 passed); the `#[cfg(test)]` net keeps the suite off live sockets. NOTE: the full `cargo test` still has 21 PRE-EXISTING e2e failures unrelated to this change (see section 7) -- this change adds no net e2e failures.
- [x] 5.4 Note the one-time session-orphaning caveat in the final summary; do NOT auto-install -- the user controls install timing

## 6. E2E harness alignment (socket-per-profile makes the e2e socket profile-derived)

- [x] 6.1 The old harness assumed `aoe` adopts its `-S` socket via `$TMUX`; with profile-derived sockets `aoe` uses `-L default`. Pin the harness's tmux commands to `aoe`'s actual `-L default` socket path so both share one server.
- [x] 6.2 Give each test its OWN private `TMUX_TMPDIR` (`<root>/<pid>-<seq>`) so every test gets a unique socket -- restores the structural per-test isolation that a shared `default` profile/socket would otherwise lose (fixes cross-test contamination).
- [x] 6.3 Harden the cross-run leak sweep (`reap_stale_aoe_test_sessions`) to scan per-test tmpdirs; tear down each test's private server in `Drop` (scoped `-S` kill-server, immune to `$TMUX`).

## 7. Out of scope: pre-existing e2e failures (tracked separately)

- [ ] 7.1 (SEPARATE) `[all]`-view tests (profile_picker/unified_view, ~11): harness launches `AGENT_OF_EMPIRES_PROFILE=default` so the title is `[default]`, but tests expect `[all]`. Determined by `App::new(profile)` (untouched by this change) -- fails identically on HEAD.
- [ ] 7.2 (SEPARATE) override-resume tests (multi_pane_restart/cold_start + 1 cli, ~10): fixtures use `--cmd-override sh`, which trips `has_command_override()` (b3fe6385) so the primary pane respawns the override shell instead of `--resume`. Fix = drop `--cmd-override` from the fixture. Pure-function command output, socket-independent -- fails identically on HEAD.
