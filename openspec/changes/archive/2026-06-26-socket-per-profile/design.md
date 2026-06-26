## Context

AoE routes every tmux invocation through the single `tmux_command()` seam in `src/tmux/mod.rs`, which optionally prepends `-L <socket-name>`. Today the socket name comes from a process-global value set once in `main.rs` from `TmuxConfig.tmux_socket_name` (a global config field). When unset (the default), `tmux_command()` builds a bare `tmux` command and selection falls to `$TMUX` / `$TMUX_TMPDIR` / `/tmp` per tmux's own precedence.

Separately, `resolve_profile(None)` already derives a per-directory profile (`auto-<sanitized-dir>-<hash-of-full-path>`); each profile has its own session store under `profiles/<profile>/`. So directories are already isolated at the store level, but not at the tmux-server level. This change binds the socket to the profile so the two are consistent and isolation is automatic.

Constraint that dominates the design: AoE spawns several helper processes that also issue tmux commands, and they must resolve the SAME socket as the main TUI or they will talk to the wrong server.

## Goals / Non-Goals

**Goals:**
- Each profile/directory runs on its own tmux server automatically, with zero configuration.
- Socket name is identical to the resolved profile name (no extra prefix/transform) so it is trivially predictable.
- Remove the global `tmux_socket_name` config entirely (no use case for forcing all profiles onto one server).
- Keep all in-server helpers (`aoe tmux status`, codex-title monitor, record-pane hook) correct without new config.
- Keep the `#[cfg(test)]` private-socket safety net fully intact.

**Non-Goals:**
- Cross-profile session switching on one server (never an AoE feature).
- Migrating already-running default-socket sessions onto the new per-profile socket (accepted one-time break; cold-start recovery handles re-creation).
- Changing the `tmux-test-isolation` capability or the integration-test `-L` pinning (unchanged).

## Decisions

### D1: Socket name = resolved profile name (verbatim)

Use the resolved profile string directly as the `-L` socket name. Profile names are already filename-safe (auto profiles sanitize to `[a-z0-9-]`; explicit profiles must already be valid path segments because they key `profiles/<profile>/`). Alternative considered: an `aoe-<profile>` prefix for namespacing in `$TMUX_TMPDIR` -- rejected as unnecessary indirection; the user explicitly wants the names identical to reduce confusion.

### D2: Socket comes from an EXPLICIT profile, never re-derived from cwd in helpers

The main TUI is the authority that creates the server: it resolves the profile from its launch cwd (or `--profile`) and explicitly initializes the socket. Every other process obtains the profile by being handed it, not by re-running `resolve_profile(None)` against its own cwd (which, inside a worktree pane, would yield a DIFFERENT profile and thus the wrong socket). Resolution order in `resolved_socket_name()`:

```
1. explicit init (main TUI, or a subcommand that carries a profile)   -> -L <profile>
2. AGENT_OF_EMPIRES_PROFILE env present                                -> -L <profile>
3. #[cfg(test)]                                                        -> -L aoe_test_<pid>
4. none of the above                                                   -> None (bare tmux, ambient $TMUX)
```

Entry-point matrix (why this is correct and safe):

| Process | Profile source | Socket |
|---|---|---|
| main TUI (`aoe` in a dir) | `resolve_profile(cwd/--profile)`, explicit init | `-L <profile>` |
| `switch-session` keybinding | explicit `--profile` in the command string | `-L <profile>` |
| `monitor-notifications` | explicit `--profile` arg | `-L <profile>` |
| `monitor-codex-title` | inherits `AGENT_OF_EMPIRES_PROFILE` (child of TUI) | `-L <profile>` |
| `record-pane` hook | inherits env from the managed server | bare -> `$TMUX` (in-server) |
| `aoe tmux status` (user `.tmux.conf`) | none | bare -> `$TMUX` (in-server) |

### D3: Bare fallback is safe because no OUTSIDE process lacks a profile

Every process that operates from OUTSIDE the managed server (the TUI, keybinding/monitor commands) always has a profile, so it always gets `-L <profile>`. Every process that has NO profile source runs INSIDE the managed server, where bare `tmux` resolves to `$TMUX` = exactly the right `-L <profile>` server it is already attached to. So the bare fallback can never accidentally hit the user's default socket from an external operation. The user's existing `.tmux.conf` `#(aoe tmux status)` snippet keeps working unchanged.

### D4: Env inheritance makes hand-launched-pane capture more reliable (bonus)

`main.rs` sets `AGENT_OF_EMPIRES_PROFILE` before starting the TUI, so the dedicated `-L <profile>` tmux server (started by the TUI) inherits it, and every pane -- including panes the user later splits and starts an agent in by hand -- inherits the correct profile. The `record-pane` hook then resolves the SAME profile as the TUI for both its store target and its socket. On today's shared default socket the server may pre-exist without that env, so hand-launched capture could land in the wrong store. Per-profile dedicated servers remove that failure mode.

### D5: Overlong-name fallback

A `-L` name becomes a unix domain socket path under `$TMUX_TMPDIR` (default `/tmp/tmux-<uid>/<name>`), and macOS caps socket paths near 104 bytes. If the derived profile name would push the socket path past a safe bound, fall back to a deterministic short hash-based name (e.g. `aoe-<hash>`). This is an edge case for pathologically long directory basenames.

### D6: Remove the config field outright (BREAKING)

Delete `TmuxConfig.tmux_socket_name` and its entire settings-TUI surface rather than deprecating it. Per project policy, backward compatibility is not required; existing configs with the field present simply ignore the now-unknown key (serde default).

## Risks / Trade-offs

- One-time orphaning of live sessions on upgrade -> mitigation: warn before installing; cold-start recovery (V key) re-creates sessions on the new per-profile socket. The user controls install timing.
- A helper that should target the managed server but somehow loses its profile would fall back to bare `$TMUX`. Inside the server that is correct; from outside there is no such helper in the matrix. Mitigation: the `tmux-test-isolation` guard test plus the entry-point matrix above; do not add new outside-the-server helpers without a profile.
- Test safety regression risk: the `#[cfg(test)]` branch MUST come before the bare-fallback branch. Mitigation: the seam unit test `test_tmux_command_is_private_under_test_even_without_optin` keeps asserting `-L aoe_test_<pid>` under test; the guard test keeps asserting no bare `tmux` outside the seam.
- Explicit profiles sharing a name across machines/dirs intentionally share a socket -- this is correct (same profile = same store = same server).

## Migration Plan

1. Ship the change; `tmux_socket_name` config key is dropped (silently ignored if still present).
2. On first launch of the new binary in a directory, AoE looks at `-L <profile>` and will not see sessions that were on the bare default socket -> they appear gone.
3. Recovery: cold-start recovery re-creates tracked panes on the new socket. No data migration of the store is needed (the store is already per-profile and unchanged).
4. Rollback: revert the change; sessions created on `-L <profile>` then become invisible to the reverted binary (mirror of step 2). Low risk given single-developer usage.
