## 1. Restart-Mode Plumbing Through Recovery

- [x] 1.1 Add a restart-mode parameter to the cold recovery entry point and replace the hardcoded resume mode at its per-pane launch site, keeping per-pane failure isolation and slot pane write-back unchanged.
- [x] 1.2 Carry the restart mode on the recovery action and through the app-level recovery handler, leaving the resume branch behaviorally identical.
- [x] 1.3 Make the primary-slot launch performed during session rebuild agree with the requested mode so a clean recovery never starts the primary agent from a resumed command.

## 2. Clean Recovery Identity Handling

- [x] 2.1 Apply the existing fresh identity transaction to the recovery path: reallocate the pre-allocated session id, drop any pending fork, commit on primary-slot launch, and roll back otherwise.
- [x] 2.2 Clear the instance's stale resume token when a clean recovery succeeds, and add unit coverage proving a later fork is not created from the discarded conversation.
- [x] 2.3 Confirm durable slot rows keep their recorded native session ids after a clean recovery so recoverability is not lost before the reconcile chain refreshes them.

## 3. State-Aware TUI Routing and Discoverability

- [x] 3.1 Route `Shift+C` to clean restart for a live selection and clean recovery for a recoverable selection, preserving the deleting-state no-op, the in-flight guard, and the no-durable-slots fallback.
- [x] 3.2 Update the contextual home status hint so `C` reads as clean restart or clean recovery according to the selected instance state, and update the help overlay text.
- [x] 3.3 Update home input unit tests for `C` in the live, recoverable, non-recoverable-missing, and deleting states, and assert `Shift+R` routing is unchanged.

## 4. Runtime Acceptance Coverage

- [x] 4.1 Add isolated-socket E2E coverage proving `Shift+C` on a recoverable multi-pane instance rebuilds the session and launches every pane without a resume flag.
- [x] 4.2 Extend nested-layout recovery coverage to the fresh mode, asserting each slot returns to its original spatial cell while launching clean.
- [x] 4.3 Add coverage proving a slot that cannot be launched during clean recovery is reported without aborting its siblings.

## 5. Verification

- [x] 5.1 Run `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo test --test e2e` with all tmux-touching tests confined to the project harness's private socket.
- [x] 5.2 Run `openspec validate unify-clean-restart-recovery` and confirm the implementation and task checklist match every scenario.
