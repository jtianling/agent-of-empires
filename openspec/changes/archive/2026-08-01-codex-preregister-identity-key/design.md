# Design: codex-preregister-identity-key

## Context

`codex_xats_bootstrap_command` (`src/session/instance.rs`) generates the `sh -c` script a Codex Cross Agent Team pane runs: preflight checks, mint a UUID, `npx --no-install cross-agent-teams-mcp@latest pre-register-codex-pane --pane "$TMUX_PANE" --agent-id "$xats_agent_id"`, then `exec codex --remote ... -c xats.agent_id=...`. That script runs in the pane's own shell, where the environment AoE injected -- including the instance's write-once `XATS_IDENTITY_KEY` -- is intact. Everything after the `exec` runs conversations through a shared app-server whose environment was frozen at daemon start, which is why the key can be delivered here and nowhere later.

The daemon side of the contract (agreed with xats-main, jt-approved): `pre-register-codex-pane` grows an optional identity-key flag (shipped as `--identity-key-env`, naming the variable rather than carrying the value); on arrival the daemon resolves the key to its previous `(team, name)`, and once it detects the Codex process on the pane's tty it pokes the pane to re-register under that identity. Registration then upserts into the same `(device, team, name)` row and the identity is recovered. The daemon's poke waits on an unexpired pre-registration row; the row's default TTL is 120s.

Constraints: the two projects must remain independently runnable (aoe without xats, xats without aoe, new aoe against old xats), and the key must never enter the Codex argv, which any process on the machine can read via `ps`.

## Goals / Non-Goals

**Goals:**
- Deliver the instance's identity key to the xats daemon over the one channel Codex can actually reach, at every launch shape the bootstrap covers (fresh, resume, fork, restart).
- Keep a Codex launch working against a daemon that predates the new flags.
- Keep the pre-registration row alive long enough for a Codex cold start.

**Non-Goals:**
- The poke-back, identity resolution, and conflict handling: all daemon-side (xats repo).
- Any change to how the key is minted or persisted (write-once on the instance record, already correct).
- Identity delivery for adopted/hand-launched Codex panes AoE did not bootstrap.

## Decisions

### Decision 1: The key is named, never spelled -- `--identity-key-env`

The bootstrap passes `--identity-key-env XATS_IDENTITY_KEY` and the CLI reads the value from its own environment (which the `npx` child inherits from the pane's shell). The first draft expanded the value into the pre-register argv (`--identity-key "$XATS_IDENTITY_KEY"`); the xats CLI then shipped the env-naming flag instead, on the observation that the pre-register process's own argv is just as `ps`-visible as Codex's. So no process the bootstrap script starts carries the value on its argv, and AoE's Rust still interpolates nothing: the script carries only the variable's name. The script branches on `[ -n "${XATS_IDENTITY_KEY:-}" ]` because the CLI treats the flag with a missing/empty variable as a hard error, which for AoE would just mean a wasted retry.

Scope honesty, from review: the value still reaches the pane at all through AoE's pre-existing env-injection prefix (`XATS_IDENTITY_KEY='...' <shell> -lc ...`), and that full command string transits the `tmux new-session`/`respawn-pane` argv at launch. That mechanism predates this change and is shared with Claude panes; replacing it (e.g. with tmux's own per-session environment) is a separate change touching every agent. What this change does take on is the derived exposure it can reach cheaply: AoE's debug logs of launch commands now mask the key's value (`redact_identity_key`).

### Decision 4: The failure sentinel is script-local by assignment

`pre_register_failed=` is assigned empty before the first attempt: `sh` variables are seeded from the environment, so without the reset an inherited `pre_register_failed=1` would turn a successful first attempt into a spurious bare fallback -- which on the daemon side would overwrite the key- and TTL-carrying row with a bare one (found in review, reproduced, pinned by a regression test that launches with the variable inherited).

### Decision 2: Fallback is retry-without-flag, decided by the exit code, immune to inherited shell options

The script runs the first attempt with `|| pre_register_failed=1` and gates the retry on that flag variable, rather than consulting `$?` after the branch: `sh` imports `SHELLOPTS` from the environment, and an inherited `errexit` would abort the script on the first attempt's failure before any `$?` check ran (found in review, reproduced, and pinned by a regression test). Only the second failure is fatal, taking the existing explicit-diagnostics path. The retry is not conditional on parsing error output -- exit code only -- because the CLI's message text is not a contract.

One measured softening of the original rationale: the CLI's parser ignores flags it does not know, so an older CLI does not actually fail on the new flags -- it succeeds while ignoring them, which is behaviorally the pre-change state. The fallback therefore protects against a future stricter CLI rather than the current old one, at the cost of one extra `npx` invocation when it fires.

### Decision 3: TTL rides the same call, as a flat number, under the flag the CLI parses

`--ttl 600` (the daemon's documented ceiling) on the pre-register call, unconditionally: the value describes how long a Codex cold start may take, which does not depend on whether an identity key exists. The flag name is `--ttl` because that is what the CLI's parser reads -- and since that parser ignores unknown flags silently, a guessed spelling would "succeed" while leaving the daemon on its 120s default, which is precisely the window this change exists to extend (found in review; the execution-level tests now assert the argv the CLI actually receives). If the fallback fires, the retry drops both new flags together -- it exists to reproduce the pre-change call exactly, not to bisect which flag offended.

## Risks / Trade-offs

- [A daemon that knows the new flags but fails for a real reason (bad key, conflict) triggers the fallback, silently discarding the key] -> Accepted: the fallback lands on today's behavior (registered, identity NULL), and the daemon-side conflict branch is the mechanism that reports genuine identity disputes. Losing recovery beats losing the launch.
- [The `--ttl` ceiling could change daemon-side] -> The value is a named constant beside the other bootstrap constants; a daemon that caps it lower clamps server-side per the agreed contract.
- [Old xats + retry doubles `npx` startup latency on a failing first call] -> Bounded and transient: disappears once the user's `@latest` cache passes 0.7.8.

## Migration Plan

Ship order is xats first (0.7.8/0.8.x with `--identity-key-env`), AoE after; but the fallback makes the order a soft preference, not a requirement. No data migration: the key and its persistence already exist.

## Open Questions

(none -- the cross-project contract was settled with xats-main before this proposal)
