## Context

AoE currently exposes three related home actions:

- `R` resumes tracked panes in an existing tmux session.
- `r` restarts tracked panes fresh.
- `V` rebuilds a dead but recoverable session from durable `agent_slot` rows.

The resume and recovery actions express the same user intent but are routed separately. The lower/upper-case `r` distinction also makes the destructive fresh action easy to trigger accidentally.

Cold recovery has enough durable data to recreate agents, resume tokens, working directories, and slot ownership, but not enough to recreate pane geometry. `recover_from_slots` creates every non-primary pane through `split_window_right_capture_pane`, so a nested layout is flattened into horizontal columns. tmux exposes the current serialized layout through `#{window_layout}`; its leaves identify the pane ids that occupied each geometric cell.

The existing `cold-start-session-recovery` change remains active rather than archived into main specs. This change builds on its implementation and introduces a separately testable layout-recovery contract.

## Goals / Non-Goals

**Goals:**

- Give `Shift+R` one state-aware "return to this conversation" meaning.
- Move the explicit fresh/clean restart to `Shift+C`.
- Preserve nested horizontal and vertical pane geometry across cold recovery.
- Keep persisted slot identity, resume-token validation, launch context, and per-pane failure isolation intact.
- Fall back to the current recoverable behavior when no trustworthy layout snapshot can be applied.

**Non-Goals:**

- Automatically recover every session during AoE startup.
- Recover panes that do not have durable `agent_slot` records.
- Persist tmux window options, zoom state, active pane, multiple windows, or arbitrary user shell processes.
- Change the CLI `session restart` command.
- Preserve exact character dimensions when the terminal size after reboot differs; tmux may scale the saved split proportions to the new window size.

## Decisions

### Decision 1: Route `Shift+R` by recoverability at the home input boundary

When the selected instance is recoverable (persisted slots and no live tmux session), `Shift+R` emits `RecoverInstance`. Otherwise it emits the existing resume `RespawnAgentPane(..., RestartMode::Resume)` action, subject to the existing deleting-state guard. This keeps the two execution cores separate while presenting one user intent.

`Shift+V` is removed rather than retained as an alias so the help and status bar teach one path. The contextual status hint says `R Recover` for a recoverable selection and `R Resume` for a live selection.

Alternative considered: make the app-level resume handler detect a missing session. Rejected because the home model already owns recoverability and contextual hints, and routing there keeps actions explicit and independently testable.

### Decision 2: Bind clean restart to uppercase `C`

The lowercase `r` binding is removed and `KeyCode::Char('C')` emits the existing fresh restart action for a non-deleting selected session. The implementation reuses `RestartMode::Fresh`; only discoverability and input routing change.

Alternative considered: keep lowercase `r` as an alias. Rejected because it preserves the accidental-trigger ambiguity the change is intended to remove.

### Decision 3: Store layout snapshots separately from agent slots

Add a per-instance durable layout record rather than duplicating a layout string on every `agent_slot` row. The record contains:

- `instance_id` as its key,
- the serialized tmux `window_layout`,
- capture time.

The schema change goes through the repository migration system and the idempotent defensive schema path. Removing an instance's session records also removes its layout snapshot.

Alternative considered: add parent/orientation/ratio columns to each slot. Rejected because tmux already provides a complete layout-tree serialization, while a hand-maintained parallel topology can drift during arbitrary user splits and resizes.

### Decision 4: Refresh only a coherent layout snapshot

The existing live reconcile path queries the session's `#{window_layout}` and pane ids. It updates the stored layout only when every layout leaf can be mapped to the durable tracked slots and the pane sets match. A transient partial reconcile therefore cannot overwrite the last known-good snapshot.

This intentionally excludes untracked panes from recoverable layout snapshots. Cold recovery already recreates exactly the durable slots and cannot restore an untracked shell process faithfully.

Alternative considered: snapshot only on clean TUI exit. Rejected because machine shutdown and crashes cannot reliably run teardown.

### Decision 5: Validate pane ids and preserve slot order for layout assignment

tmux parses the numeric pane ids embedded in a custom layout but assigns its
leaf cells to panes in the window pane-list order. Recovery therefore creates
placeholder panes as a chain: slot 0 is the initial pane, slot 1 is split from
slot 0, slot 2 from slot 1, and so on. This keeps pane-list order aligned with
the saved layout's leaf traversal order so every durable slot returns to its
original spatial cell.

Before applying the stored layout, the parser still replaces each old pane id
using `agent_slot.tmux_pane -> new_pane_id` and recomputes the checksum. tmux
does not use those rewritten ids for cell assignment, but the operation
provides the load-bearing one-to-one pane-set validation: duplicate, missing,
or extra pane ids reject a stale layout instead of allowing tmux to reshape a
different pane set. Layout text is passed as a single command argument, not
interpolated into a shell.

Alternative considered: replay a sequence of horizontal and vertical split commands. Rejected because deriving a stable split sequence and ratios from rectangles is more error-prone than preserving tmux's own layout tree.

### Decision 6: Layout failure degrades without blocking conversation recovery

If the layout snapshot is missing, stale, malformed, incompatible with the recovered pane set, or rejected by tmux, recovery retains the current horizontal fallback and records/logs a layout warning. Pane resume continues independently. A layout problem must not turn recoverable conversations into an unrecoverable session.

The layout is applied after all placeholder panes exist and before auto-attach. Resume commands can be launched before or after applying geometry because pane ids are stable; applying before the final attach avoids visible reflow.

## Risks / Trade-offs

- [tmux layout serialization changes across versions] -> Parse defensively, validate the complete input, and fall back without blocking recovery.
- [Saved layout references panes no longer represented by slots] -> Store snapshots only for exact pane-set matches and validate again at recovery.
- [Terminal size differs after reboot] -> Preserve topology and relative geometry while allowing tmux to resize the layout.
- [Pane-list order diverges from layout leaf order] -> Chain recovery splits in durable slot order and assert each slot's coordinates, not only the overall geometry, in isolated-socket E2E coverage.
- [Checksum or pane-id validation is incorrect] -> Add focused parser/checksum unit tests using real nested tmux layout fixtures plus isolated-socket E2E coverage.
- [Reconcile adds another tmux query] -> Fold layout into an existing formatted query where practical and only write SQLite when the snapshot changes.
- [Keybinding change surprises existing users] -> Mark it breaking, update the contextual footer, help overlay, README, and generated documentation source where applicable.

## Migration Plan

1. Add an idempotent migration for the per-instance layout snapshot table and bump the schema version.
2. Begin opportunistically capturing coherent layouts from live sessions. Existing profiles start with no snapshots.
3. Switch TUI bindings and documentation in the same release.
4. Cold recovery uses layout restoration when available and retains the old horizontal fallback for pre-migration sessions.

Rollback is safe at the data level: older binaries ignore the new table. Keyboard behavior rolls back with the binary.

## Open Questions

- None blocking. Exact tmux layout fixtures and checksum behavior will be confirmed against the installed tmux implementation during development and isolated-socket tests.
