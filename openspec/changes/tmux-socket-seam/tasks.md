## 1. Seam foundation

- [ ] 1.1 In `src/tmux/mod.rs` add `static TMUX_SOCKET_NAME: OnceLock<Option<String>>`, `pub fn init_tmux_socket_name(name: Option<String>)`, and `resolved_socket_name()` (returns the configured name; under `#[cfg(test)]` lazily pins `aoe_test_<pid>` so it NEVER returns `None`).
- [ ] 1.2 Add `pub(crate) fn tmux_command() -> Command` that builds `Command::new("tmux")` and appends `-L <name>` when `resolved_socket_name()` is `Some`.
- [ ] 1.3 Replace the env-based `isolate_tmux_socket()` with one that pins the private label (via `init`/the cfg(test) path) AND `remove_var("TMUX")`/`("TMUX_PANE")`. Keep it `#[cfg(test)] pub(crate)`.
- [ ] 1.4 Unit tests for the builder: configured name -> `-L name`; cfg(test) default -> `-L aoe_test_<pid>` even with no opt-in (assert on args; execute nothing).

## 2. Route all tmux invocations through the seam

- [ ] 2.1 Replace every production `Command::new("tmux")` (~74 sites in `src/tmux/{session,utils,status_bar,mod,notification_monitor}.rs`, `src/tui/{app,status_poller}.rs`, `src/db/reconcile.rs`, `src/process/mod.rs`) with `crate::tmux::tmux_command()` (or local `tmux_command()` within the module). Production behavior unchanged when no socket name set.
- [ ] 2.2 Audit attach/keybinding/option paths (`src/cli/session.rs`, `src/tui/app.rs`, `src/tmux/status_bar.rs`) so `attach-session`, `setup_session_cycle_bindings`, and `set-option` calls also go through the seam and honor the socket name.
- [ ] 2.3 Strengthen `tests/tmux_test_isolation_guard.rs`: fail on any bare `Command::new("tmux")` in production source outside `tmux_command()`; keep the destructive-command isolation marker check for tests.

## 3. Test isolation wiring

- [ ] 3.1 `src/tmux/session.rs` + `src/tmux/utils.rs` tmux tests already call `isolate_tmux_socket()`; confirm they now go through the seam (`-L`) and stay `#[serial]`.
- [ ] 3.2 `tests/tui_attach_detach.rs` `isolated_tmux()`: add `-L <unique>` (e.g. pid-based) in addition to clearing `$TMUX`/`$TMUX_PANE`; give the lifecycle test a pid-unique session name.

## 4. Configurable socket name (feature 2)

- [ ] 4.1 Add `tmux_socket_name: Option<String>` to `TmuxConfig` (`src/session/config.rs`, serde default `None`, update `Default`).
- [ ] 4.2 At startup (after config load) call `crate::tmux::init_tmux_socket_name(resolved_config.tmux.tmux_socket_name.clone())`.
- [ ] 4.3 Settings TUI: add `FieldKey` variant + `SettingField` entry (`src/tui/settings/fields.rs`) with help text about the "only sees sessions on that socket" caveat; wire `apply_field_to_global()` / `apply_field_to_profile()`; add `clear_profile_override()` case (`src/tui/settings/input.rs`).
- [ ] 4.4 Add `tmux_socket_name` to `TmuxConfigOverride` in `src/session/profile_config.rs` with merge logic in `merge_configs()`.

## 5. Finalize and verify safely

- [ ] 5.1 `cargo fmt`, `cargo clippy --all-targets` (no new warnings).
- [ ] 5.2 Run ONLY the new pure builder/guard tests (assert built commands contain `-L <private>`; no tmux executed). Do NOT run the full `cargo test`, and do NOT run any destructive `tmux` subcommand on any server.
- [ ] 5.3 If desired, a single OPTIONAL read-only sanity check may be run by the USER (not the agent) against a private socket only: `tmux -L aoe_test_probe new-session -d -s x \; kill-session -t x` -- never a bare/default `tmux`.
