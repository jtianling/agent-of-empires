## Context

tmux socket selection precedence (from `tmux.c` `main()`): `-S <path>` > `-L <name>` > `$TMUX` (socket part) > `$TMUX_TMPDIR` > `/tmp`. Critically, `getenv("TMUX")` is read **only inside `if (path == NULL && label == NULL)`** -- i.e. only when neither `-L` nor `-S` is passed. So an explicit `-L`/`-S` makes the client ignore `$TMUX` entirely. The e2e harness already relies on this (it passes `-S <private path>` on every command and never clears `$TMUX`).

AoE has ~74 bare `Command::new("tmux")` sites and no central builder, so nothing can enforce a socket choice. Env-only isolation (`TMUX_TMPDIR`) is unreliable because the dev runs inside tmux (`$TMUX` set), which overrides it -- this is what destroyed live sessions.

## Goals / Non-Goals

**Goals:**
- One builder all AoE tmux commands go through, applying `-L <socket-name>`.
- Tests can NEVER reach the default/live socket, even with `$TMUX` set, even if a test forgets to opt in.
- User-configurable production socket name as defense-in-depth.
- Production behavior unchanged when no socket name is configured.

**Non-Goals:**
- Not changing the e2e harness (already isolated via `-S`).
- Not migrating existing default-socket sessions onto a configured socket (documented limitation).
- Not clearing `$TMUX` in production (only in tests).

## Decisions

### Decision 1: `-L` (socket name), not `-S` (path), for the seam

The seam applies `-L <name>`. `-L` is enough to override `$TMUX` (Decision rationale: precedence #2, read before `$TMUX`), keeps sockets under the standard `tmux-UID` dir, and a name is friendlier for a user config value than a full path. (e2e uses `-S` because it wants a path under the test's temp HOME; the seam wants a stable name.)

### Decision 2: process-global socket name via `OnceLock`, resolved once

```rust
static TMUX_SOCKET_NAME: OnceLock<Option<String>> = OnceLock::new();

pub fn init_tmux_socket_name(name: Option<String>) { let _ = TMUX_SOCKET_NAME.set(name); } // startup, from config

pub(crate) fn tmux_command() -> Command {
    let mut cmd = Command::new("tmux");
    if let Some(name) = resolved_socket_name() { cmd.arg("-L").arg(name); }
    cmd
}
```

`resolved_socket_name()` returns the configured name; **under `#[cfg(test)]` it never returns `None`** -- it lazily pins `aoe_test_<pid>` so a unit test that forgot to isolate still cannot touch the default socket. Read-hot-path uses `OnceLock` (no lock per call); a per-call `String` clone is negligible next to spawning a process.

- **Why OnceLock over env `set_var`:** `set_var`/`remove_var` are process-global and thread-unsafe (UB in edition 2024) and rely on every tmux test being `#[serial]`. `-L` via a builder is per-`Command` and immune to thread races and to `$TMUX`.

### Decision 3: test opt-in clears `$TMUX` too (belt-and-suspenders)

`#[cfg(test)] isolate_tmux_socket()` pins the private label AND `remove_var("TMUX")/("TMUX_PANE")`. `-L` alone already overrides `$TMUX` for socket choice; clearing it additionally removes the nested-attach guard edge (`server_client_check_nested`) for any attach-style test. Tests that touch tmux stay `#[serial]`.

### Decision 4: integration tests carry `-L` themselves

`#[cfg(test)]` is NOT active for the lib when compiled for `tests/*.rs`, so the seam's test safety-net does not apply there. Integration tests that build tmux commands (`tests/tui_attach_detach.rs`) use a local `isolated_tmux()` that passes `-L <unique>` + clears `$TMUX`. They do not exercise production tmux-spawning code, so they need no seam access.

### Decision 5: configurable socket name (feature 2)

`TmuxConfig.tmux_socket_name: Option<String>` (serde default `None`). Startup calls `init_tmux_socket_name(config.tmux.tmux_socket_name.clone())`. Wired into the settings TUI per the project rule (FieldKey, SettingField, apply_to_global/profile, clear_profile_override, `TmuxConfigOverride` merge). Attach/keybinding/option paths must also go through the seam so a configured socket is honored end-to-end.

## Risks / Trade-offs

- **Missed call site** -> a bare `Command::new("tmux")` left un-routed would, under a configured/test socket, hit the wrong server. Mitigation: the strengthened static guard forbids bare `Command::new("tmux")` outside the seam.
- **Attach path not routed** -> if `attach-session`/keybinding/option commands don't honor the socket name, a configured socket breaks attach. Mitigation: audit and route all attach-path tmux calls; add coverage.
- **Configured socket hides existing sessions** -> when a user sets `tmux_socket_name`, AoE only sees that socket; default-socket sessions vanish from AoE until recreated there. Documented in the settings UI; not auto-migrated.
- **OnceLock set-once** -> `init_tmux_socket_name` is effectively first-write-wins. Startup sets it before any tmux use; tests set it via the cfg(test) lazy path. Acceptable: the value is process-stable by design.
- **`$TMUX`/`$TMUX_PANE` removal not restored in tests** -> harmless (removal is the safe direction); later tests just see them unset.

## Migration Plan

No data migration. Default `tmux_socket_name = None` preserves today's behavior exactly. Setting a name is opt-in and only affects sessions created/recovered afterward.

## Open Questions

- Should a configured socket name trigger a one-time notice that existing default-socket sessions won't appear? (Lean: a settings-field help line is enough.)
