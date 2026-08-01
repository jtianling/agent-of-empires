## Why

AoE mints a xats identity key for the pane it launches from the instance record, but an extra agent pane launched beside it gets none: `build_extra_pane_command` builds with no slot key and not as the primary pane, so no `XATS_IDENTITY_KEY` reaches the process. Measured on a live dual-Codex session: the left pane's process carried the instance's key, the right pane's carried nothing.

The cost is not that the right pane recovers late. It is that it cannot recover at all, and that its emptiness is exploitable:

- The key it eventually receives is minted at its first relaunch, so it never matches the identity the pane registered under at first launch. Every restart hands the daemon a key that no identity holds, which reads as a new agent rather than a returning one.
- A pane with no key is exactly what the daemon's seat-matching treats as claimable. A pane that already holds a key is skipped. On the measured session the keyless right pane was handed a dead agent's key that AoE never issued, and the original holder lost it.

A pane that AoE itself launched has no reason to be keyless. AoE built its command and knows which slot it occupies at the moment it creates it.

## What Changes

- Mint an identity key when AoE launches an extra agent pane for a Cross Agent Team session, and inject it as `XATS_IDENTITY_KEY` the same way the primary pane's key is injected. This covers both entry points: the right pane of a new session, and `aoe session add-agent-pane`.
- Mint a fresh key for that pane. Never copy the primary pane's key: two live panes behind one identity is the state the recovery design cannot resolve.
- Record the pane's durable slot at launch rather than waiting for its first capture, so the key has somewhere to live and the pane is restartable before it has been claimed. The launch-time record carries the agent, the real pane id and the key, and no native session id, which arrives with the first capture.
- Preserve that key when the reconciler later writes the slot from a capture, so identity does not change at the moment the pane is adopted.
- Fall back to the instance's stored resume token during a fan-out restart when slot 0 carries no native session id. Without this, creating the slot record earlier silently narrows what `R` can resume in the window before the first capture.
- Narrow the existing "panes AoE never launched receive a key at their first relaunch" allowance to panes a user genuinely started by hand. It currently also describes AoE's own extra panes, which is the gap being closed.

Because the slot record now exists from the first second, the extra pane is inside the restart fan-out immediately instead of after its first capture. On Codex that window was measured between 14 seconds and 2 minutes 37 seconds, during which a restart silently skipped the pane.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `cross-agent-team`: an extra agent pane AoE launches carries a freshly minted identity key from its first launch, and the hand-started-pane allowance narrows to panes AoE did not launch.
- `agent-session-store`: a pane AoE launches has a durable slot record from launch, before any capture exists for it, and a capture completes that record without replacing its key.
- `agent-resume-restart`: a fan-out restart falls back to the instance's stored resume token when slot 0 has no native session id.

## Impact

- `src/session/instance.rs`: extra-pane command construction, key minting for a launched extra pane, and the fan-out resume-token fallback.
- `src/tui/app.rs`: the new-session right pane launch path.
- `src/cli/session.rs`: the `add-agent-pane` launch path.
- `src/db/`: writing a slot record at launch, and preserving its key when a capture later completes it.
- `tests/` and `tests/e2e/`: key presence at first launch, key stability across restart, freshness against the primary pane's key, and the resume fallback in the pre-capture window.
- No change to the xats side. The Codex pre-registration command shape is deliberately untouched: the daemon-side scoping work that would alter it is still undecided, and AoE was asked not to move first.
