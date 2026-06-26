## 1. Seam foundation

- [x] 1.1 In `src/tmux/mod.rs` add `static TMUX_SOCKET_NAME: OnceLock<Option<String>>`, `pub fn init_tmux_socket_name(name: Option<String>)`, and `resolved_socket_name()` (returns the configured name; under `#[cfg(test)]` lazily pins `aoe_test_<pid>` so it NEVER returns `None`).
- [x] 1.2 Add `pub(crate) fn tmux_command() -> Command` (delegates to pure `build_tmux_command(Option<&str>)`) that appends `-L <name>` when a socket name is resolved.
- [x] 1.3 Replace the env-based `isolate_tmux_socket()` with one that pins the private label AND `remove_var("TMUX")`/`("TMUX_PANE")`. Kept `#[cfg(test)] pub(crate)`.
- [x] 1.4 Unit tests for the builder: configured name -> `-L name`; no name -> bare; cfg(test) default -> `-L aoe_test_<pid>` even with no opt-in (assert on args; execute nothing).

## 2. Route all tmux invocations through the seam

- [x] 2.1 Replaced all ~74 `Command::new("tmux")` sites (`src/tmux/{session,utils,status_bar,mod,notification_monitor}.rs`, `src/tui/{app,status_poller}.rs`, `src/db/reconcile.rs`, `src/process/mod.rs`) with the seam. Only the builder definition keeps a bare `Command::new("tmux")`.
- [x] 2.2 Attach/keybinding/option paths honored: `Session::attach`, status-bar options, and the session-cycle bindings all build via the seam now; the guard (2.3) proves no bare tmux remains, so a configured socket is honored end-to-end. (`aoe tmux switch-session` invoked from bindings re-resolves the socket name on its own startup.)
- [x] 2.3 Strengthened `tests/tmux_test_isolation_guard.rs`: `no_bare_tmux_command_outside_seam` + `integration_tests_pin_private_socket`.

## 3. Test isolation wiring

- [x] 3.1 `src/tmux/session.rs` + `src/tmux/utils.rs` tmux tests go through the seam (`-L aoe_test_<pid>`) and remain `#[serial]`; `isolate_tmux_socket()` also clears `$TMUX`/`$TMUX_PANE`.
- [x] 3.2 `tests/tui_attach_detach.rs` `isolated_tmux()` pins `-L aoe_test_attach_<pid>` and clears `$TMUX`/`$TMUX_PANE`; lifecycle test uses a pid-unique session name.

## 4. Configurable socket name (feature 2, GLOBAL-only)

- [x] 4.1 Added `socket_name: Option<String>` to `TmuxConfig` (`src/session/config.rs`, serde default `None`, `Default` updated).
- [x] 4.2 `main.rs` calls `agent_of_empires::tmux::init_tmux_socket_name(...)` right after CLI parse (best-effort `load_config`), covering all entry points incl. the early `aoe tmux ...` subcommands.
- [x] 4.3 Settings TUI: `FieldKey::TmuxSocketName` + a Global-scope-only `SettingField` (OptionalText) with the "only sees sessions on this socket / takes effect next launch" help text; `apply_field_to_global` sets it (blank -> None); `clear_profile_override` no-op (global-only).
- [x] 4.4 N/A by design: socket name is GLOBAL-only (all entry points must share one tmux server for cross-profile switching), so it is intentionally NOT added to `TmuxConfigOverride` / `merge_configs()`. The settings field is hidden outside Global scope to make this explicit.

## 5. Finalize and verify safely

- [x] 5.1 `cargo fmt` clean; `cargo clippy --all-targets` no new warnings (one pre-existing e2e doc-list warning untouched).
- [x] 5.2 Ran only the pure builder/settings/guard tests (assert built commands carry `-L`; no tmux executed). Full `cargo test` and destructive `tmux` subcommands deliberately NOT run.
- [ ] 5.3 OPTIONAL (user-run, not the agent): a read-only sanity check against a PRIVATE socket only -- `tmux -L aoe_test_probe new-session -d -s x \; kill-session -t x` -- never a bare/default `tmux`.
