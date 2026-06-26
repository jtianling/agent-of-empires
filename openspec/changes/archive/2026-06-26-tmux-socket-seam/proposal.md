## Why

Running `cargo test` (or an ad-hoc `tmux` command) on a developer machine has repeatedly destroyed the developer's live AoE tmux sessions. Root cause, confirmed from tmux source (`tmux.c` `main()`): a `tmux` client reads `$TMUX` for its socket **only when neither `-L` nor `-S` is given**, and `$TMUX` (set whenever the runner is inside tmux) overrides `TMUX_TMPDIR`. So env-based isolation (`TMUX_TMPDIR`) silently fails and commands hit the live default server. The authoritative fix is to pass an explicit `-L <socket-name>` on every AoE tmux command: `-L`/`-S` is precedence-rank #1-2 and cannot be overridden by `$TMUX`.

AoE currently spells out `Command::new("tmux")` in ~74 scattered places with no socket flag, so there is no single point to enforce isolation, and no way for a user to run AoE on a dedicated socket as defense-in-depth.

## What Changes

- Add a single `tmux_command()` builder in `src/tmux/mod.rs` that every AoE tmux invocation goes through. It applies `-L <socket-name>` based on a process-global socket name resolved once at startup. All ~74 `Command::new("tmux")` sites are routed through it (production behavior unchanged when no socket name is configured: bare `tmux` -> default socket).
- **Test isolation**: under unit-test builds the seam forces a private per-process socket (`-L aoe_test_<pid>`) even if a test forgets to opt in, so tests can NEVER reach the default/live server. `isolate_tmux_socket()` additionally clears `$TMUX`/`$TMUX_PANE` (belt-and-suspenders for the nested-attach edge). Integration tests that build tmux commands directly carry `-L` themselves.
- **Configurable production socket** (feature 2): add `TmuxConfig.tmux_socket_name: Option<String>`. When set, the seam runs every AoE tmux command on `-L <name>`, so the user can run real AoE on a dedicated socket (e.g. `jt`) as a second layer of protection. Editable in the settings TUI per the project rule. Default `None` = current default-socket behavior.
- Strengthen `tests/tmux_test_isolation_guard.rs`: forbid bare `Command::new("tmux")` outside the seam, and require a socket flag / isolation marker in test tmux usage.
- **BREAKING**: none for default config. When a user SETS `tmux_socket_name`, AoE only sees sessions on that socket; pre-existing default-socket sessions will not appear until recreated/recovered there. Documented in the settings UI.

## Capabilities

### New Capabilities
- `tmux-test-isolation`: every test that invokes tmux runs on a private `-L`-selected socket, never the default socket, so the suite can never create/kill/mutate the developer's live sessions -- even when the runner is inside tmux (`$TMUX` set).
- `tmux-socket-name`: AoE's tmux socket name is user-configurable; when set, all AoE tmux commands target `-L <name>`; editable in the settings TUI; default keeps the current default-socket behavior.

### Modified Capabilities
<!-- none -->

## Impact

- `src/tmux/mod.rs`: new `tmux_command()` seam + `init_tmux_socket_name()` / `isolate_tmux_socket()` + process-global socket-name `OnceLock`.
- ~74 call sites across `src/tmux/{session,utils,status_bar,mod,notification_monitor}.rs`, `src/tui/{app,status_poller}.rs`, `src/db/reconcile.rs`, `src/process/mod.rs`: `Command::new("tmux")` -> `crate::tmux::tmux_command()`.
- Attach paths (`src/cli/session.rs`, `src/tui/app.rs`): ensure attach/keybinding/option commands also honor the socket name.
- `src/session/config.rs`: `TmuxConfig.tmux_socket_name`; `src/main.rs` (or startup): call `init_tmux_socket_name()` after config load.
- Settings TUI wiring: `src/tui/settings/fields.rs` (FieldKey + SettingField), `apply_field_to_global/profile`, `clear_profile_override` (`src/tui/settings/input.rs`), and `TmuxConfigOverride` merge in `profile_config.rs`.
- `tests/tmux_test_isolation_guard.rs`: strengthened guard. `tests/tui_attach_detach.rs` + `src/tmux/{session,utils}.rs` tests: use the seam / `-L`.
- Verification is by asserting the built command contains `-L <private>` -- NO tmux is executed against any server during development.
