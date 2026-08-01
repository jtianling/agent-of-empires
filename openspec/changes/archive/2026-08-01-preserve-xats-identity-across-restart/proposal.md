## Why

A Cross Agent Team pane registers itself with xats under a team and a name that the user supplies inside the conversation. AoE never learns that identity. It only knows the feature is enabled.

That works as long as the conversation survives. On a resume restart or a resume recovery the agent's context comes back, the agent remembers that it is `tester` on team `aoe`, and it re-registers itself. Identity appears to be preserved, but nothing in AoE preserves it.

Every fresh path breaks that. A clean restart deliberately discards the context, so the relaunched agent has no memory of its identity and the user must type the registration line again for every pane. The same is true after a clean recovery, which is exactly the moment a user is least willing to re-register several agents by hand.

The gap is not that AoE forgets the identity. It is that AoE never held anything that could bridge a fresh launch.

## What Changes

- Mint a durable, opaque identity key for each Cross Agent Team pane AoE launches, and persist it so it survives process restart and machine reboot: on the instance record for the primary pane, alongside the other state describing that same agent, and on the durable slot record for an adopted pane.
- Accept that panes AoE never launched (agent adoption is observe-first) receive a key at their first AoE relaunch rather than at their first launch, and that reconcile must preserve a slot's key rather than blanking it from a capture.
- Inject the key into the launched pane as the `XATS_IDENTITY_KEY` environment variable, for both `claude` and `codex`, on every launch regardless of restart mode. The key is what makes a fresh launch recoverable; it is not a restart-path special case.
- Reuse the slot's existing key on every relaunch, restart, and recovery instead of minting a new one.
- Mint a fresh key, never copy one, when a session is cloned through new-from-selection or forked.
- Treat a key that no longer resolves as a normal state and fall back to today's manual registration flow.
- Keep the key out of argv, out of logs, and out of committed files.

The matching xats-side contract (accepting the key at registration under a three-way binding rule, resolving it during reconnect with per-tool shapes, and teaching the startup hint to use it) is an external dependency tracked in the design, not implemented here.

## Capabilities

### Modified Capabilities

- `cross-agent-team`: AoE mints, persists, injects, and preserves a per-pane identity key so a fresh launch can restore its xats identity, and mints a fresh key for cloned or forked sessions.
- `agent-session-store`: the durable per-slot record carries the identity key, and a launch-time pane-to-key association bridges launch and slot assignment.

## Impact

- `src/db/` and `src/migrations/` for the new column, the launch-time association table, and the legacy-database healing path.
- `src/db/reconcile.rs` for carrying the key into the durable slot record during slot assignment.
- `src/session/instance.rs` for minting at launch, injecting the environment variable into the claude and codex launch commands, and reusing the stored key on every relaunch and recovery path.
- `src/session/builder.rs` and the fork/clone paths for fresh-key allocation.
- `tests/` and `tests/e2e/` for key stability across restarts, fresh allocation on clone, and absence when Cross Agent Team is disabled.
- External dependency on the xats daemon for the identity resolution half. AoE-side behavior is independently verifiable; end-to-end identity continuity is not until that half ships.
