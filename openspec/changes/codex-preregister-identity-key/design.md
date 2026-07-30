# Design: codex-preregister-identity-key

## Context

`codex_xats_bootstrap_command` (`src/session/instance.rs`) generates the `sh -c` script a Codex Cross Agent Team pane runs: preflight checks, mint a UUID, `npx --no-install cross-agent-teams-mcp@latest pre-register-codex-pane --pane "$TMUX_PANE" --agent-id "$xats_agent_id"`, then `exec codex --remote ... -c xats.agent_id=...`. That script runs in the pane's own shell, where the environment AoE injected -- including the instance's write-once `XATS_IDENTITY_KEY` -- is intact. Everything after the `exec` runs conversations through a shared app-server whose environment was frozen at daemon start, which is why the key can be delivered here and nowhere later.

The daemon side of the contract (agreed with xats-main, jt-approved): `pre-register-codex-pane` grows an optional `--identity-key`; on arrival the daemon resolves the key to its previous `(team, name)`, and once it detects the Codex process on the pane's tty it pokes the pane to re-register under that identity. Registration then upserts into the same `(device, team, name)` row and the identity is recovered. The daemon's poke waits on an unexpired pre-registration row; the row's default TTL is 120s.

Constraints: the two projects must remain independently runnable (aoe without xats, xats without aoe, new aoe against old xats), and the key must never enter the Codex argv, which any process on the machine can read via `ps`.

## Goals / Non-Goals

**Goals:**
- Deliver the instance's identity key to the xats daemon over the one channel Codex can actually reach, at every launch shape the bootstrap covers (fresh, resume, fork, restart).
- Keep a Codex launch working against a daemon that predates `--identity-key`.
- Keep the pre-registration row alive long enough for a Codex cold start.

**Non-Goals:**
- The poke-back, identity resolution, and conflict handling: all daemon-side (xats repo).
- Any change to how the key is minted or persisted (write-once on the instance record, already correct).
- Identity delivery for adopted/hand-launched Codex panes AoE did not bootstrap.

## Decisions

### Decision 1: The key rides the pre-register call, quoted from the environment, never from Rust

The script references `"$XATS_IDENTITY_KEY"` the same way it already references `"$TMUX_PANE"`: expanded by the pane's shell at run time, inside double quotes. AoE's Rust code does not interpolate the key's value into the script text. This keeps the generated command identical whether or not a key exists (the script branches on `[ -n "$XATS_IDENTITY_KEY" ]`), and keeps the key out of every place the command string is logged, captured, or diffed. The `exec`ed Codex command line is untouched, so the argv-visibility red line holds by construction.

### Decision 2: Fallback is retry-without-flag, decided by the pre-register exit code

An older `cross-agent-teams-mcp` rejects the unknown flag with a non-zero exit. The script tries `--identity-key` first (when the variable is non-empty) and on failure retries the exact call without it; only the second failure is fatal, taking the existing explicit-diagnostics path. The retry is not conditional on parsing error output -- exit code only -- because the CLI's message text is not a contract. Cost: one extra `npx` invocation in the old-daemon case, on a path that already tolerates `npx` latency.

### Decision 3: TTL rides the same call, as a flat number

`--ttl-seconds 600` (the daemon's documented ceiling) on the pre-register call, unconditionally: the value describes how long a Codex cold start may take, which does not depend on whether an identity key exists. If the older-CLI fallback fires, the retry drops both new flags together -- the fallback exists to reproduce the pre-change call exactly, not to bisect which flag offended.

## Risks / Trade-offs

- [A daemon that knows `--identity-key` but fails for a real reason (bad key, conflict) triggers the fallback, silently discarding the key] -> Accepted: the fallback lands on today's behavior (registered, identity NULL), and the daemon-side conflict branch is the mechanism that reports genuine identity disputes. Losing recovery beats losing the launch.
- [`--ttl-seconds` ceiling could change daemon-side] -> The value is a named constant beside the other bootstrap constants; a daemon that caps it lower clamps server-side per the agreed contract.
- [Old xats + retry doubles `npx` startup latency on a failing first call] -> Bounded and transient: disappears once the user's `@latest` cache passes 0.7.8.

## Migration Plan

Ship order is xats first (0.7.8/0.8.x with `--identity-key`), AoE after; but the fallback makes the order a soft preference, not a requirement. No data migration: the key and its persistence already exist.

## Open Questions

(none -- the cross-project contract was settled with xats-main before this proposal)
