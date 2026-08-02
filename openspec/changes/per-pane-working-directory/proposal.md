## Why

An AoE-managed session can run two agent panes, and the two agents frequently belong in different directories: an implementer in the repo, a reviewer in a sibling checkout, a tester in a scratch tree. Today the right pane is pinned to the left pane's directory with no way to say otherwise.

The pin is not a persistence limitation. The durable layer already models a working directory per pane: every `agent_slot` row carries its own `cwd`, `resume_launch_pane` respawns each pane into the directory its slot recorded, and `rebuild_recovery_panes` splits each recovery pane at its slot's `cwd`. The dimension exists everywhere except at first launch, where two call sites collapse both panes onto one value:

- `src/tui/app.rs` passes `&inst.project_path` as the split directory for the right pane.
- `src/session/instance.rs` passes `&self.project_path` into `record_launched_extra_pane`, where the single `cwd` parameter is written to both the primary slot and the extra pane's slot.

The second call site is the more damaging half, and it is invisible until a restart. Even if the split landed somewhere else, the extra pane's slot would record the left pane's directory, so the first `R` would pull the pane back to the wrong place and nothing would explain why.

A neighbouring gap surfaced while scoping this. Once a session is running, the TUI offers no way to add a managed agent pane at all. The only entry point is `aoe session add-agent-pane <identifier>`, which pins the same directory and additionally hardcodes the session's own tool, even though `build_extra_pane_command` has always accepted any tool. The `multi-agent-session` spec requires "an explicit action to add an agent pane" without requiring a keybinding, so the CLI-only state satisfies the letter of the spec while leaving the TUI half unbuilt.

Both gaps come from the same assumption: that the second pane is an accessory to the first rather than a peer. Panes that AoE launches, tracks in slots, mints identity keys for, and restarts as equals should be configurable as equals.

## What Changes

- Let a managed pane other than the primary one start in a working directory of its own, chosen when the pane is created. An unset directory means "wherever the session ends up", resolved at split time rather than snapshotted, so worktree resolution and group default directories keep working.
- Record that directory on the pane's durable slot, so restart and cold-start recovery return the pane to it. This requires splitting the single `cwd` argument of `record_launched_extra_pane` into the primary pane's directory and the launched pane's own.
- Add a "Right Pane Path" field to the new session dialog, shown when a right pane tool is selected. Its editing behavior matches the session path field: ghost completion, `Ctrl+P` directory browsing, and the create-directory confirmation.
- Add the same field to the fork dialog, which today has no path input at all.
- Add a `%` keybinding on the TUI home screen that adds a managed agent pane to the selected running session, through a small dialog offering the pane's tool and working directory. AoE attaches to the session once the pane is up.
- Accept `--path` and `--tool` on `aoe session add-agent-pane`, so the CLI entry point can express what the TUI one can.
- Extract the path-field machinery (input, ghost completion, directory-picker routing, invalid-path flash) into a reusable component, and move the existing session path field onto it. Three path fields across two dialogs with independently drifting behavior is the alternative.
- Merge the create-directory confirmation so one prompt covers every missing directory a submit would need, instead of one prompt per field.

The right pane's directory is configurable; the session's `project_path` is not, and does not become "the left pane's directory". It remains the session's anchor: the deduplication key, the worktree base, the group default directory source, the sandbox volume root, and the path Claude session resolution reads. A pane's `cwd` is only a working directory.

The change deliberately leaves `bind-key %` inside attached sessions alone. That binding pins a hand-made split to `@aoe_project_path` and is the only sensible behavior for a raw shell pane, which has nowhere to accept a directory. See `design.md` for why the two rules point in opposite directions and are both correct.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `right-pane`: the right pane's working directory is chosen at creation rather than fixed to the session's `project_path`, and the new session dialog gains a path field for it.
- `multi-agent-session`: the add-agent-pane action selects the pane's tool and working directory, and is reachable from the TUI as well as the CLI.
- `agent-session-store`: a launch-time slot record carries the launched pane's own working directory rather than the instance's.
- `tui`: a `%` keybinding adds a managed agent pane to the selected running session.

## Impact

- `src/tui/components/`: new reusable path field component; the existing session path field moves onto it.
- `src/tui/dialogs/new_session/`: right pane path field, merged create-directory confirmation, and convergence of the field-index arithmetic that is currently computed in three places.
- `src/tui/dialogs/fork_session.rs`: its first path input, plus directory-picker hosting.
- `src/tui/dialogs/`: new add-pane dialog for the `%` action.
- `src/tui/home/`: the `%` keybinding, the pending right-pane channel widening from a tool name to a tool and a directory, and the cap and not-running checks the CLI already performs.
- `src/session/instance.rs`, `src/db/reconcile.rs`: the launched pane's own `cwd` on its slot record.
- `src/cli/session.rs`, `src/cli/definition.rs`: `--path` and `--tool` on `add-agent-pane`, plus `cargo xtask gen-docs`.
- `tests/` and `tests/e2e/`: the directory reaching the pane at first launch, surviving a restart, and the dialog and keybinding paths.
- No change to `pane-cwd-inherit`. The `prefix + %` and `prefix + "` bindings inside attached sessions keep pinning hand-made splits to `@aoe_project_path`.
