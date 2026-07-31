## Context

AoE has two ways to build a pane's launch command and only one of them is complete.

`Instance::build_pane_command(target_agent, resume_token, is_primary, slot_identity_key)` is the one the restart and cold-start recovery paths use. It knows the primary/non-primary distinction, sandboxing, YOLO, Cross Agent Team decoration for `claude` and `codex`, and identity-key injection.

`build_right_pane_command(instance, tool_name)` in `src/tui/app.rs` is the other. It predates all of that: binary, YOLO, container exec, and a `cd` for shell. It is what the new-session dialog's Right Pane runs through, and it produces a pane that is invisible to Cross Agent Team. `aoe session add-agent-pane` avoids that trap by calling `build_agent_command`, but that is `build_pane_command` with `is_primary = true`, so it produces the opposite defect: a second pane wearing the instance's own conversation and identity.

Capture has the mirror-image gap. Claude panes report themselves through their status hook, so an extra Claude pane already reaches a slot. Codex installs no hook (`hook_config: None`); its panes are bound by reading Codex's rollout files, and `reconcile_all` calls `codex_rollout::maybe_claim_for_pane` for the primary pane only. A second Codex pane therefore has no `pane_live` row, `reconcile_session` skips it (`read_pane_live` -> `None` -> `continue`), it never becomes an `agent_slot`, and both `Shift+C` (which fans out over `read_slots_for_instance`) and cold-start recovery (which rebuilds from the same slots) skip it.

## Goals / Non-Goals

**Goals:**

- One command builder for every agent pane AoE launches, so decoration cannot be present in one path and absent in another.
- An extra Codex pane reaches a durable slot, which is the precondition for restart and recovery covering it.
- An extra pane never presents the instance's identity or conversation.

**Non-Goals:**

- Minting an identity key at the moment an extra pane is launched. There is no slot record to own it yet.
- Any change to the xats daemon, to its candidate selection, or to the pre-registration protocol.
- Retroactive adoption of extra panes that are already running untracked. They are adopted the first time they produce a capture, like any other pane.
- Removing the ambiguity inherent in matching rollouts by time (see Risks).

## Decisions

### 1. Route agent right panes through `build_pane_command`, keep the shell branch local

`build_right_pane_command` keeps its `shell` branch and delegates everything else to `instance.build_pane_command(tool, None, false, None)`.

Shell cannot go through the shared builder: the `shell` registry entry's `binary` is the literal string `"shell"`, not an executable, so `build_base_pane_command` would produce a command that fails to launch. The shell branch already resolves `$SHELL` and prepends a `cd`. A shell pane also produces no capture and never occupies a slot, so nothing downstream wants it unified.

Alternative considered: teach `build_pane_command` about shell. Rejected -- it would push a pane kind that is never tracked into the builder that exists to describe tracked panes, for no gain.

### 2. `is_primary = false`, and no identity key at launch

Passing `is_primary = false` is what suppresses the instance's command override, pre-allocated session id, fork token, and instance identity key. Passing `slot_identity_key = None` leaves the pane keyless on its first launch.

This is not a gap being left open; it is the behavior the Cross Agent Team spec already defines for a pane whose slot does not exist yet ("Panes AoE never launched receive a key at their first relaunch"): the key is minted by `ensure_slot_identity_keys` the first time AoE relaunches that slot, and is stable from then on. The cost is one extra manual registration, and it is bounded.

Alternatives considered and rejected:

- **Mint at launch, stash the key against the pane id.** Requires somewhere to keep a pane-to-key mapping between the split and the reconciler's first sight of the pane: a new table plus migration, a tmux pane user option, or a map on the instance record. Each adds a storage lifetime that has to be garbage-collected, to save exactly one manual registration.
- **Pre-create the `agent_slot` row at launch with the key and an empty capture.** Slot assignment is the reconciler's job and `is_recoverable` keys off "has slots"; a slot with no capture would make an unstarted pane look recoverable.

### 3. Attempt the rollout claim per pane, with instance-level preconditions scoped to the primary

`maybe_claim_for_pane` gains an `is_primary` input. `inst.tool != "codex"` and `inst.has_command_override()` are checked only when it is set. Everything else in the function is already per-pane and stays: the pane must have no existing capture, its process tree must actually be invoking Codex (`process_tree_runs_codex`, matched on the command line because npm-installed Codex runs behind a `node` shim), and the rollout must be unclaimed.

Scoping those two guards is not a loosening. The instance's tool describes its own agent pane, and a command override describes the program AoE launches for the instance; neither says anything about what is running in a pane beside it. The positive evidence requirement is what keeps a shell pane or a dead Codex from being bound.

`reconcile_all` iterates the panes it already listed, in `list_session_panes` order (ascending pane index, i.e. creation order), calling the claim for each before `reconcile_session` snapshots them.

Alternative considered: order panes by process start time instead of pane index. Rejected as unnecessary structure -- the greedy claim is correct in either order for panes that started more than the slack apart, because `claimed_native_session_ids()` prevents double-claiming, and neither order resolves the sub-slack case.

### 4. `add_agent_pane` builds a non-primary command

`build_agent_command(None)` becomes `build_pane_command(&inst.tool, None, false, None)`. This is the same call the right pane makes, and it is what stops the CLI from putting the instance's identity key into a second live pane.

### 5. `pane_base_command` is made non-primary aware

`pane_base_command` returned the instance's command override for any pane running the instance's tool, primary or not. It feeds `codex_xats_bootstrap_command`, so a second Codex pane in an instance carrying an override was bootstrapped from that override -- while `build_base_pane_command` had already built the plain binary for the same pane, meaning the two disagreed and the bootstrap's `strip_prefix` silently produced an empty suffix.

This was going to be recorded as pre-existing debt, but it makes the spec's "an extra pane does not inherit the instance's launch context" false for exactly the case this change is about (a second Codex pane in a Codex session). It takes an `is_primary` argument now, which also brings it into agreement with `build_base_pane_command`. Nothing changes for a primary pane; a non-primary one stops relaunching the instance's own program.

## Risks / Trade-offs

- **Two Codex panes started within the rollout slack window (2s) can be bound to each other's conversation.** The claim matches "earliest unclaimed rollout created at or after this pane's process start", and the slack exists because a pane's shell starts before Codex stamps the rollout name. -> Not mitigated. The failure is a swapped resume target between two panes of the same session in the same directory; both conversations survive, and a user who notices can restart the pane. Adding a disambiguation mechanism means inventing a per-pane signal Codex does not currently emit.
- **An extra pane is keyless for one restart cycle**, so its first `Shift+C` mints a key that no identity holds yet and the user registers it once by hand. -> Accepted; this is the documented adopted-pane cost, and it converges.
- **A right pane in an instance with a command override now launches the tool's own binary rather than the override.** -> This is the intended correction (the override describes the instance's agent), but it is a visible behavior change for anyone who was relying on the old builder to apply YOLO and nothing else.
- **The four-slot cap now binds sooner** for Codex sessions, because extra Codex panes actually consume slots. -> Intended; the cap exists to bound tracking, and a pane occupying a slot is what makes it recoverable.
