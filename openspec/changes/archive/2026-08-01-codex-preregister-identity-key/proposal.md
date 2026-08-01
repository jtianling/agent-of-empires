# Proposal: codex-preregister-identity-key

## Why

A Codex Cross Agent Team pane cannot recover its xats identity across an AoE restart. AoE keeps a write-once `XATS_IDENTITY_KEY` per instance and injects it into the Codex client's environment, but Codex executes tools inside a shared `--remote` app-server that never sees the client's environment (measured live), so the key never reaches the xats daemon and every Codex registration lands with `identity_key = NULL`. The xats side (0.7.7) already has the full identity-recovery machinery; the key just has no route that Codex can actually deliver it through.

The one moment the key IS readable is AoE's own xats bootstrap: it runs `pre-register-codex-pane` in the pane's shell, before `exec codex`, with the injected environment intact. The agreed cross-project design (jt-approved, coordinated with xats-main) routes the key through that call; the xats daemon then restores the identity and pokes the restarted Codex to re-register.

## What Changes

- The Codex xats bootstrap passes `--identity-key-env XATS_IDENTITY_KEY` to `pre-register-codex-pane` when the variable is non-empty; the CLI reads the value from its inherited environment and sends it to the daemon over HTTP, so the value appears on the argv of no process the bootstrap starts. (The value still reaches the pane through AoE's pre-existing env-injection prefix, which transits the tmux launch argv -- a mechanism shared with Claude panes and out of scope here; see the design doc.) AoE debug logs of launch commands mask the value.
- Compatibility fallback: if the pre-register call with the new flags fails, the bootstrap retries once without them, so a `cross-agent-teams-mcp` that rejects them cannot break a Codex launch.
- The bootstrap passes a lengthened pre-registration row TTL (`--ttl 600`), covering Codex cold starts that exceed the daemon's 120s default (the daemon's poke-back waits on an unexpired row).

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `cross-agent-team`: the "Codex xats pane bootstrap" requirement gains identity-key delivery, an old-daemon fallback, and a pre-registration TTL; the bootstrap-failure requirement's fallback semantics extend to the identity-key retry.

## Impact

- `src/session/instance.rs`: `codex_xats_bootstrap_command` (the generated shell script) and its unit tests.
- No new runtime dependency: `pre-register-codex-pane` is already invoked; only its arguments change.
- Cross-project contract: xats ships `--identity-key-env` support first (`cross-agent-teams-mcp` 0.7.8/0.8.x); AoE's fallback keeps older daemons working, so the two projects stay independently runnable in both directions.
