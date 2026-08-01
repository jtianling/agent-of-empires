## Why

The Codex xats bootstrap pre-registers the pane, and on failure retries once without the identity key. That retry was written when a missing key was cheap. It is not: the key is the only thing that lets the xats daemon recognize which identity a pane belongs to, so a pane that pre-registers without one is never sent a recovery prompt after a restart, and never re-registers. The pane looks fine and stays silently outside Cross Agent Team for the rest of its life.

So the retry converts a loud failure into a silent, permanent degradation -- the failure mode this project spent a day learning to recognize.

The retry also cannot succeed in the cases that now produce a failure:

- **The npx call itself fails.** Publishing a new xats version invalidates the `@latest` entry `npx --no-install` resolves against, so the first call fails at the npx layer. The retry runs the same `npx --no-install` with the same spec and fails identically.
- **The daemon refuses the write.** xats now rejects a pre-registration that would displace a live keyed row unless it carries the same key. An unkeyed retry is exactly the shape that rule rejects.

In both cases the retry turns one clear failure into two.

## What Changes

- A failed Codex pane pre-registration terminates the pane command with its diagnostic, instead of retrying without the identity key.

## Capabilities

### Modified Capabilities

- `cross-agent-team`: the Codex bootstrap's pre-registration failure is terminal rather than degrading to a keyless registration.

## Impact

- `src/session/instance.rs`: the pre-registration section of `codex_xats_bootstrap_command_for`.
- Behavior change for the failure path only. A pane whose pre-registration succeeds is unaffected.
- A pane that previously would have started keyless now does not start. That is the intent: it surfaces at launch instead of as a Cross Agent Team member that never answers.
