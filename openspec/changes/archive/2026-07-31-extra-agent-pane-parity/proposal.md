## Why

An extra agent pane that AoE itself launches -- the "Right Pane" of the new session dialog, or `aoe session add-agent-pane` -- is not the same kind of pane as the one beside it. It is built by a second, older command builder that knows nothing about Cross Agent Team, and for Codex it is never captured at all. The user sees two AoE-managed Codex panes; AoE sees one.

Measured on the reporting machine: session `aoe_aoe-codex_a7ba96a7` had panes `%76` and `%77`. `%76` ran `codex --remote ... -c xats.agent_id=...` and held slot 0. `%77` ran a bare `codex` with no `--remote`, no agent id, and no `XATS_IDENTITY_KEY`, had no `pane_live` row, no `agent_slot` row, and was silently skipped by `Shift+C` restart and by cold-start recovery. Nothing surfaced that half the session was untracked.

## What Changes

- Extra agent panes are launched through the same builder the restart and recovery paths use, so a Cross Agent Team session decorates them exactly as it decorates its primary pane: the Codex xats bootstrap (pre-registration, `--remote`, agent id) and the Claude channel flag.
- **BREAKING** (behavioral): `aoe session add-agent-pane` no longer builds its pane as if it were the instance's own agent. It stops copying the instance's command override, pre-allocated session id, fork token, and identity key into a second pane. Two panes presenting one identity key is the one failure the identity design cannot recover from, so this is a correctness fix rather than a preference.
- Codex conversation binding covers every pane of a managed session, not only the primary one. The instance-level preconditions that gate it today describe the primary pane alone and are applied only there; a non-primary pane is judged by what it is actually running.
- Once bound, an extra Codex pane reaches a durable slot through the existing snapshot path, which is what makes `Shift+C` and cold-start recovery include it. No new restart or recovery machinery.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `right-pane`: the right pane's launch command comes from the shared pane-command builder, so launch-context decoration that applies to an agent pane applies to it too.
- `multi-agent-session`: the add-agent-pane action launches a pane that is not the instance's own agent pane, and does not carry the instance's launch context or identity.
- `pane-session-capture`: Codex rollout binding applies to each pane of a managed session, with the instance-level preconditions scoped to the primary pane.

## Impact

- `src/tui/app.rs`: `build_right_pane_command` loses its agent branch to `Instance::build_pane_command`; the shell branch stays (the `shell` registry entry's binary is not an executable, and a shell pane is never captured into a slot).
- `src/cli/session.rs`: `add_agent_pane` builds a non-primary pane command.
- `src/db/reconcile.rs` and `src/db/codex_rollout.rs`: the claim is attempted per pane, with primary-only preconditions passed in.
- An extra pane carries no identity key at launch, because there is no slot record to own one yet. It is minted at that slot's first AoE relaunch, which is the behavior already specified for panes AoE did not launch, and costs one extra manual registration.
- Users with a Cross Agent Team session and a Codex right pane will see that pane pre-register with xats from the next launch onward. Existing untracked panes are not retroactively adopted; they are adopted the first time they produce a capture.
