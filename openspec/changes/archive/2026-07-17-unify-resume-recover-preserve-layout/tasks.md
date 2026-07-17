## 1. Durable Layout Storage

- [x] 1.1 Add the next versioned migration and idempotent schema definition for one layout snapshot per instance, including capture time and cascade/cleanup behavior.
- [x] 1.2 Add store APIs to read, upsert only changed snapshots, and delete layout records with instance session records; cover lifecycle behavior with deterministic database tests.

## 2. tmux Layout Capture and Remapping

- [x] 2.1 Extend the tmux/reconcile query seam to capture a session's serialized `window_layout` together with the pane ids needed for coherence validation.
- [x] 2.2 Persist a new snapshot only when the live layout pane set maps one-to-one to the instance's durable slot pane set, retaining the last valid snapshot for partial observations.
- [x] 2.3 Implement a defensive tmux layout parser/remapper that replaces old slot pane ids with newly created pane ids, validates one-to-one coverage, and recomputes the layout checksum.
- [x] 2.4 Add focused unit tests for horizontal, vertical, and nested layout fixtures plus malformed, duplicate, missing, and mismatched pane-id cases.

## 3. Layout-Preserving Cold Recovery

- [x] 3.1 Add a session-scoped tmux helper that applies a remapped serialized layout without shell interpolation or global option changes.
- [x] 3.2 Update cold recovery to load the last coherent snapshot, remap it through the old-slot to new-pane mapping, and apply it after all recovery panes exist and before auto-attach.
- [x] 3.3 Preserve the current fallback layout and continue all per-pane resume attempts when the snapshot is absent, invalid, stale, or rejected by tmux; surface a layout-specific warning.

## 4. Unified TUI Actions

- [x] 4.1 Route `Shift+R` to live resume or cold recovery according to focused-instance recoverability while preserving deleting and missing-non-recoverable behavior.
- [x] 4.2 Replace lowercase `r` fresh restart with `Shift+C`, retaining the existing `RestartMode::Fresh` execution core and safety guards.
- [x] 4.3 Remove the `Shift+V` recovery binding and update contextual status hints, help overlay, README, and applicable generated CLI documentation sources.
- [x] 4.4 Update home input/render unit tests for `R` live resume, `R` recoverable recovery, `C` clean restart, and removed `r`/`V` behavior.

## 5. Runtime Acceptance Coverage

- [x] 5.1 Add isolated-socket E2E coverage proving `Shift+R` resumes a live tracked session and recovers a dead recoverable session, while `Shift+C` launches fresh commands without resume flags.
- [x] 5.2 Add isolated-socket E2E coverage that creates a left pane plus vertically split right column, persists its layout, destroys only the exact test session, recovers it, and verifies the nested geometry and slot-to-agent mapping.
- [x] 5.3 Add recovery E2E coverage for a missing or invalid layout snapshot to prove fallback recovery still resumes every eligible pane.

## 6. Verification

- [x] 6.1 Run `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo test --test e2e` with all tmux-touching tests confined to the project harness's private socket.
- [x] 6.2 Run `openspec verify unify-resume-recover-preserve-layout` and confirm the implementation and task checklist match every scenario.
