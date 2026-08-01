## Context

AoE mints a xats identity key per pane so a fresh launch can recover the identity that pane had before it was restarted. The primary pane's key lives on the instance record; an adopted pane's key lives on its durable slot record and is minted the first time AoE relaunches that slot.

An extra agent pane falls through both. `build_extra_pane_command` builds it as a non-primary pane with no slot key, so `xats_identity_key_for_pane` returns nothing and no `XATS_IDENTITY_KEY` is injected. There is also nowhere to put a key if one were minted: the pane has no slot record yet, because slot records are written only by the reconciler, from a capture, and a capture arrives only after the pane's first exchange.

The two facts compound. The pane registers its identity with no key, so nothing binds the identity to a key. At the next restart the pane is finally in a slot and a key is minted, but that key is new and the identity it registered under holds none, so recovery never matches. The pane is permanently one round behind itself.

Two measured consequences, both from a live dual-Codex session on 2026-08-01:

- The right pane's process had no `XATS_IDENTITY_KEY` while the left pane's had the instance's key.
- The keyless right pane was handed a dead agent's identity key by the daemon's seat matching, which skips any caller that already holds a key. Holding a key is what makes a pane safe here; holding none is what makes it a target.

The daemon-side defects found alongside this (the seeding round, the startup hint, and the seat key) are owned by the xats side and are out of scope. The Codex pre-registration command shape is deliberately untouched: the scoping work that would change it is undecided there, and AoE was explicitly asked not to move first.

## Goals / Non-Goals

**Goals:**

- An extra agent pane AoE launches carries an identity key from its first launch, so its first registration is the one that binds the key.
- That key is stable across restart and recovery, and is never the primary pane's key.
- The key has a durable home that exists at launch, not one that appears whenever a capture happens to arrive.
- Nothing that works today stops working, in particular what `R` can resume before a capture exists.

**Non-Goals:**

- Any xats-side change.
- Any change to the Codex pre-registration arguments.
- Keys for `shell` panes, which run no agent and register no identity.
- Changing fork or new-from-selection semantics beyond the existing rule that a cloned session gets a fresh key.
- Closing the adoption-latency problem as a feature in its own right. Shrinking that window is a consequence of where the key has to live, not a separate goal.

## Decisions

### The key's home is the durable slot record, written at launch

The launch writes the slot record itself, carrying the agent, the pane id it just created, and the minted key, with no native session id. The reconciler later fills the native session id in from the first capture.

Alternatives considered:

- **tmux pane user options** (`@aoe_pane_tool`, `@aoe_pane_key`). Attractive because a pane option's lifetime is exactly the pane's, so nothing has to garbage-collect it. Rejected because it introduces a second surface that the restart fan-out has to consult in addition to the slot table, and because the reconciler would then have to take the key off the pane option at adoption or the identity would change at that moment. One surface with one adoption rule is simpler than two surfaces with a handover.
- **The volatile per-pane capture table.** Rejected outright, and not on taste: a pane row that exists before the agent's own capture makes `capture_is_superseded` compare a captured timestamp against a launch timestamp that is effectively the same instant. That comparison can never become true again, and Codex claiming for that pane would be blocked permanently. This would break the very panes the change is meant to help.

The slot table tolerates the launch-time row: the reconciler creates rows only when it holds a real capture, it skips panes with no capture rather than writing an empty row over them, it already preserves an existing slot's identity key against a capture that carries none, and slot assignment keeps a live pane in the slot its existing record names.

### The key is minted fresh, never copied

The instance's key describes the instance's own pane. An extra pane that presented the same key would be a second live holder of one identity, which the recovery design cannot resolve and which the daemon rejects as a conflict. Launch is the only moment where this is preventable, so the freshness assertion belongs in the tests rather than being left implicit.

### Slot 0 keeps a resume path in the pre-capture window

Creating slot records at launch changes which restart branch runs early in a session's life. Today, with no tracked panes, a restart takes the single-pane branch, which falls back to the instance's stored resume token. With a launch-time record the fan-out branch runs instead, and it reads only the slot's native session id, which is empty until the first capture.

The window is real: the instance's resume token is scraped from the primary pane's output when the instance enters an error state, which is exactly the "agent exited and printed a resume hint" case, and that can happen before any capture. Left alone, a restart that used to reattach the conversation would quietly start a fresh one.

The fallback is therefore part of this change, not a follow-up. It is scoped to slot 0 because the instance's resume token describes the instance's own pane and nothing else.

### The hand-started-pane allowance is a statement about panes AoE did not launch

The existing requirement that a pane may run keyless until its first relaunch exists because adoption is observe-first: AoE never built that pane's command and must not reach into it while it runs. An extra pane AoE launched is the opposite case. The specs add the launched-pane rule rather than rewriting that requirement, both because the requirement as written is already about hand-started panes and because it currently lives in an unarchived change's delta, where a cross-change modification would collide at archive time.

## Risks / Trade-offs

- **A slot record now exists for a pane that has never reported a conversation.** → The reconciler completes rather than replaces it, and a record with no native session id degrades a launch to fresh instead of failing it. The one behavior that genuinely changes, `R` in the pre-capture window, is covered by the slot 0 fallback above.

- **An instance becomes "recoverable" earlier**, because recoverability is driven by whether any slot record exists. A session that dies before its first capture will now offer cold-start recovery where it previously offered none. → This is closer to the truth than the old behavior: AoE knows what it launched and can relaunch it. Worth stating because it is a user-visible change that no requirement in this change describes.

- **The layout snapshot is taken earlier**, because it is gated on the live pane set matching the durable slot set, which now converges sooner. → No behavior depends on the snapshot being late; noted so it is not mistaken for a regression.

- **The key is minted before the pane has an identity to bind it to.** → This is the existing two-round semantics and is already specified: a key no identity holds yet must not fail the launch, and the registration that follows binds it.

- **End-to-end identity continuity still cannot be asserted from AoE alone.** The daemon must accept the key at registration and resolve it at reconnect. → Acceptance for this change is scoped to what AoE controls: key presence at first launch, stability across restart, freshness against the primary key, absence when disabled, and the resume fallback.

## Open Questions

- Whether the extra pane should also be recorded well enough to be relaunched after a cold start that rebuilds panes from slot records. The recovery path creates new panes with new ids and reads the key from the slot record, so it should follow, but this change does not add recovery coverage for a pane whose slot record has no native session id.
