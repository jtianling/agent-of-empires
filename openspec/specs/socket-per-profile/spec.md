# socket-per-profile Specification

## Purpose
TBD - created by archiving change socket-per-profile. Update Purpose after archive.
## Requirements
### Requirement: Socket name is derived from the resolved profile

AoE SHALL derive its tmux socket name from the resolved profile name, using the profile string verbatim as the `-L <name>` value. Each profile (and therefore each launch directory, since `resolve_profile(None)` yields `auto-<dir>-<hash>`) SHALL run on its own tmux server. The main TUI SHALL resolve its profile once and initialize the process-global socket name from it before any tmux command runs.

#### Scenario: Each directory gets its own socket
- **WHEN** AoE is launched in directory A (profile `auto-A-xxxx`) and separately in directory B (profile `auto-B-yyyy`)
- **THEN** the AoE in A SHALL run its tmux commands on `-L auto-A-xxxx`
- **AND** the AoE in B SHALL run its tmux commands on `-L auto-B-yyyy`
- **AND** neither AoE SHALL see or disturb the other's sessions

#### Scenario: Explicit profile sets the socket
- **WHEN** AoE is launched with `--profile myprofile`
- **THEN** its tmux commands SHALL run on `-L myprofile`

#### Scenario: Socket name equals profile name verbatim
- **WHEN** the resolved profile is `auto-agent-of-empires-3f2a`
- **THEN** the socket name SHALL be exactly `auto-agent-of-empires-3f2a` (no added prefix or transform)

### Requirement: Helper processes resolve the same socket as the main TUI

AoE-spawned helper processes that issue tmux commands SHALL target the same server as the main TUI. A helper SHALL obtain the profile by being handed it (an explicit `--profile` argument, or the inherited `AGENT_OF_EMPIRES_PROFILE` environment variable) and SHALL NOT re-derive the socket from its own working directory.

#### Scenario: switch-session keybinding targets the managed server
- **WHEN** the session-cycle keybinding runs `aoe tmux switch-session --profile <p> ...`
- **THEN** that process SHALL run its tmux commands on `-L <p>`

#### Scenario: codex-title monitor inherits the profile
- **WHEN** the main TUI (which has set `AGENT_OF_EMPIRES_PROFILE`) spawns the codex-title monitor
- **THEN** the monitor SHALL resolve `-L <profile>` from the inherited environment, not from its cwd

### Requirement: Bare tmux fallback when no profile is resolvable

When no profile is resolvable (no explicit init, no `AGENT_OF_EMPIRES_PROFILE` env, and not under test), `tmux_command()` SHALL build a bare `tmux` command with no `-L` flag, so selection falls to the ambient `$TMUX` server. This keeps in-server helpers that have no profile (the user's `.tmux.conf` `#(aoe tmux status)`, the record-pane hook) correct without configuration, because inside the managed server `$TMUX` already points at the right `-L <profile>` server.

#### Scenario: Status command from the user's tmux config works unchanged
- **WHEN** the user's `.tmux.conf` runs `#(aoe tmux status)` inside a managed AoE session
- **THEN** the command SHALL resolve the current session via the ambient `$TMUX` server
- **AND** SHALL NOT force a `-L` socket that could point at a different server

#### Scenario: No outside process relies on the bare fallback
- **WHEN** a process operates on tmux from outside the managed server (the main TUI or a profile-carrying subcommand)
- **THEN** it SHALL always have a profile and run on `-L <profile>`, never falling back to bare tmux

### Requirement: Test-only private socket is preserved ahead of the bare fallback

Under `#[cfg(test)]`, `resolved_socket_name()` SHALL force a private `aoe_test_<pid>` socket, and this branch SHALL be evaluated BEFORE the bare-fallback branch, so the test suite can never produce a bare tmux command that touches a live socket.

#### Scenario: Tests stay isolated even without opt-in
- **WHEN** code under `#[cfg(test)]` calls `tmux_command()` without any explicit socket init
- **THEN** the built command SHALL include `-L aoe_test_<pid>`
- **AND** SHALL NOT be a bare tmux command

### Requirement: Overlong socket names fall back to a short hash

If the profile-derived socket name would make the resulting unix domain socket path exceed a safe length bound, AoE SHALL substitute a deterministic short hash-based socket name so the socket can still be created.

#### Scenario: Pathologically long directory name
- **WHEN** the resolved profile name is long enough that `$TMUX_TMPDIR/<name>` would exceed the platform socket-path limit
- **THEN** AoE SHALL use a deterministic shortened (hash-based) socket name instead
- **AND** all entry points for that profile SHALL agree on the same shortened name

