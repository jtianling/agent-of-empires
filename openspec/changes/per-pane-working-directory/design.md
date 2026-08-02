## Context

AoE sessions can run several agent panes. Slot 0 is the instance's own agent; slots 1..3 are extra panes AoE launched (the new session dialog's right pane, the fork dialog's right pane, `aoe session add-agent-pane`) or panes it adopted from a capture.

Every layer below launch already treats a pane's working directory as per-pane state:

| Path | Directory it uses |
| --- | --- |
| `record_pane` capture | the pane's real `$PWD` |
| `upsert_agent_slot_capture` | `capture.cwd`, per slot |
| `resume_launch_pane` -> `respawn_pane_target` | the slot's `cwd` |
| `rebuild_recovery_panes` -> `split_window_right_capture_pane` | the slot's `cwd` |
| First launch of an extra pane | **the instance's `project_path`, for both panes** |

The last row is the whole defect. It appears twice: once as the split's `-c` argument, and once as the single `cwd` parameter of `record_launched_extra_pane`, which is written to the primary slot and the extra pane's slot alike.

The two occurrences fail differently, which is worth keeping separate while implementing. Fixing only the split makes the pane start in the right place and come back in the wrong one after a restart, with nothing on screen to connect the two events. Fixing only the record does nothing observable at all.

The dialogs sit on top of this. `NewSessionData` carries `right_pane_tool: Option<String>` and nothing else about the pane, and `HomeView` forwards it through `pending_right_pane_tool: Option<String>` to the attach flow in `app.rs`, which performs the split. There is no channel for a second value because there has never been a second value.

## Goals / Non-Goals

**Goals:**

- A managed pane other than the primary one can be given its own working directory when it is created, from every entry point that creates one.
- That directory survives restart and cold-start recovery, because it lands on the pane's durable slot rather than only on the tmux split. A `shell` pane that inherited the session's directory is the one exception, and is covered in the decisions below.
- Choosing it feels like choosing the session's path: same completion, same browsing, same handling of a directory that does not exist yet.
- A running session can gain a managed agent pane from the TUI, not only from the CLI.

**Non-Goals:**

- Changing what `project_path` means. It stays the session's anchor; only pane `cwd` becomes per-pane.
- Per-pane working directories inside sandboxed sessions. See the decision below.
- Changing `prefix + %` / `prefix + "` inside attached sessions.
- Raising the four-slot cap, or adding a TUI way to remove a managed pane.
- Per-pane extra args, command overrides, or YOLO. The dialogs stay at tool plus directory.

## Decisions

### An unset directory is resolved at split time, not captured at submit time

"Same as the session" is stored as absent and falls back to `project_path` at the moment of the split, rather than being copied out of the path field when the dialog is submitted.

A snapshot would be wrong whenever the session's directory is not yet known at submit time, which is exactly the worktree case: `build_instance` replaces the typed path with the resolved worktree path, so a snapshot taken in the dialog would point the right pane at the original repository while the left pane went to the worktree. The existing `right-pane` spec has a scenario for precisely this ("Right pane working directory matches left pane after worktree resolution"), and the late fallback is what keeps it passing. Group default directories behave the same way.

An explicitly typed directory is used verbatim. If a session is worktree-backed and the user names another directory for the second pane, that directory is not worktree-resolved, because it is not the session's repository.

### The field is hidden while sandboxing is on

Under sandboxing, `build_extra_pane_command` emits `docker exec -w <container_workdir> ...`. The agent's directory is decided inside the container, and the tmux split's `-c` is decorative. A path field that silently does nothing is worse than no field, and the honest alternatives both exceed this change: reinterpreting the value as a container path makes it mean something different from the field above it, and mounting an additional host directory is a sandboxing feature in its own right.

So the field is shown only when the session is not sandboxed. If per-pane directories inside containers are wanted later, that is a `sandbox` capability change with its own mount story.

### One create-directory confirmation covers every missing directory

`confirm_create_dir: Option<bool>` is currently a single-slot state that only ever describes the session path. With two path fields, a submit can find two missing directories.

The confirmation carries the list of directories that would be created and creates them together on confirm. The alternative, one prompt per field in sequence, doubles the keystrokes and introduces a partial state where the first directory has been created and the user then cancels the second.

### A reusable path field, and the existing session path field moves onto it

The session path field's behavior lives in `src/tui/dialogs/new_session/path_input.rs`, with every method bound to `self.path`, `self.path_ghost`, and `self.path_invalid_flash_until`. The workspace-repo editor already demonstrates what happens without extraction: it re-implements a subset (input plus ghost) and silently lacks segment jumps and the invalid flash.

This change needs two more path fields, one of them in a dialog with no path input at all. Three call sites is where the duplication stops being tolerable, so the machinery moves into a component alongside the existing `path_ghost.rs` and `dir_picker.rs`, and the session path field is moved onto it in the same change.

Moving the existing field is the deliberate part. Leaving it on its bespoke implementation would mean the new fields either reimplement segment jumps and the invalid flash to match, or visibly do not match. Since the premise of this change is that these panes are peers, a permanent behavior gap between their path fields contradicts it. The existing field is densely covered by `new_session/tests.rs`, which is what makes the move the safer half of this change rather than the riskier one.

The shared `DirPicker` needs to know which field it was opened for. The existing `workspace_repo_dir_picker_active: bool` becomes an explicit target, which is the same information the boolean already encodes.

### A shell pane takes a slot only when it was given a directory

A shell pane has always been left slotless: it runs no agent, holds no identity and produces no capture, so a slot would only cost the session one of four. That rule predates this change and is right for a shell pane that inherited the session's directory, which recovery would place there anyway.

It stops being right the moment the pane is given a directory of its own. That directory is held nowhere else, so a slotless pane is outside the restart fan-out and absent from cold recovery entirely, and the choice survives exactly until the first restart. The fork dialog defaults its right pane to `shell`, so this is not a corner: it is the most common way the feature is used.

So the carve-out narrows from "shell" to "shell that inherited the session's directory". Alternatives considered:

- **Keep shell slotless and document it.** Honest, and it was the cheaper option, but it makes the feature absent exactly where it is most reached from.
- **Give every shell pane a slot.** Simplest rule, but it spends a quarter of the session's slot budget on a pane that carries nothing a restart needs.

This makes one previously unreachable path reachable. The agent registry's entry for `shell` has the literal string `shell` as its binary, which names no program: the launch path never uses it, because `build_extra_pane_command` special-cases shell into `build_extra_shell_pane_command`. The resume path did use it, and was unreachable only because no slot ever recorded `shell`. It now routes shell slots the same way the launch path does.

### `record_launched_extra_pane` grows two directories

The function currently takes one `cwd` and writes it to both the primary pane's record (when absent) and the launched pane's record. Those are two different facts that happened to be equal.

The launched pane's directory moves onto `LaunchedPane`, beside the pane id, agent, and identity key it already carries, because it describes that pane. The primary pane's directory stays a separate parameter, sourced from the instance, because that record describes the pane AoE did not just launch.

### The `%` keybinding, and why it points the opposite way from `prefix + %`

`%` on the home screen adds a managed agent pane to the selected running session and attaches. Two behaviors that look similar now coexist:

| Trigger | Where | Result | Directory |
| --- | --- | --- | --- |
| `prefix + %` | attached to a session | raw tmux pane, no agent, no slot, no key | forced to `@aoe_project_path` |
| `%` | home screen | managed agent pane: agent launched, slot recorded, key minted | chosen, defaults to the session's |

Their directory rules point in opposite directions, and both are right. A hand-made split has no interface through which to accept a directory, so inheriting the project path is the only useful default it can have; that is the entire purpose of the `pane-cwd-inherit` capability. A managed pane is created through a dialog, which is exactly such an interface. The distinction is not "attached versus not" but "was the pane given a chance to be configured".

This is written down because the two rules read as a contradiction when found separately, and the plausible-looking fix in either direction breaks the other.

Three narrower decisions on the keybinding:

- **Home screen only.** Reaching it while attached would mean a tmux binding, which brings the setup/cleanup/status-hint lifecycle in `AGENTS.md` and a collision with `prefix + %`. Detaching to the home list is a small enough price.
- **Attaches after creating.** Adding a pane is an act of wanting to use it. The no-attach case is already served by the CLI, and `%` cannot spawn a case-based variant the way `R`/`r` and `C`/`c` do, because it is already a shifted key.
- **Does not start a stopped session.** The CLI refuses this today and the TUI matches. Starting a whole session as a side effect of asking for one more pane is more than the key promises; the four-slot cap is likewise reported rather than worked around.

### Field-index arithmetic converges

The new session dialog computes its field indices three times over: `handle_key`, `render`, and `current_input_mut`, each walking the same conditional sequence with its own `fi` counter, plus a `path_field()` helper and hardcoded indices in `tests.rs`. Adding a conditional field means keeping four hand-maintained sequences aligned, and a mismatch shows up as focus landing on the wrong field, which no unit test currently catches.

The layout is computed once and consumed by all three, so a new conditional field is added in one place. This is a precondition for the change rather than an improvement bundled with it: with the arithmetic left duplicated, the most likely defect this change ships is not in any of its features.

## Risks / Trade-offs

- **Moving the session path field onto the shared component touches working, well-tested code.** Mitigated by moving it in its own step, before any new field exists, so the test suite is judging a pure refactor.
- **The field-index convergence is a refactor the user did not ask for.** Justified above as a precondition. It is sequenced first for the same reason as the item above.
- **A typed directory that does not exist at split time** currently fails as `tracing::warn!` with no pane and no visible explanation. Acceptable while the directory was always the session's own; not acceptable once the user typed it. The dialogs confirm creation up front, and a split that still fails is surfaced rather than logged.
- **`%` overlaps a key that already means something in AoE.** Accepted: the shared meaning is "split to the right", the difference is whether the pane is managed, and the table above is in the spec so the next reader does not have to rediscover it.
- **Sandboxed sessions get no per-pane directory.** Accepted and visible: the field is absent rather than present and inert.

## Migration Plan

No data migration. `agent_slot.cwd` already exists and is already per-slot; this change starts writing the correct value into it at launch instead of the primary pane's.

Existing sessions whose extra panes recorded the primary pane's directory self-correct on the extra pane's next capture, which writes the pane's real `$PWD`. No backfill is needed, and none would be better informed than the capture.

## Open Questions

None blocking.
