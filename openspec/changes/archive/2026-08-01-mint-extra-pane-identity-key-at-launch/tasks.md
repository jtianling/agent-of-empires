## 1. Launch-Time Slot Record

- [x] 1.1 Add a store API that writes a durable slot record for a pane AoE just launched, carrying instance, slot, agent, pane id and identity key, with an empty native session id. Cover the empty-native-session-id row round-tripping through the store.
- [x] 1.2 Confirm the reconciler completes such a record from the first capture instead of replacing it: the native session id is filled in and the existing identity key survives. Add coverage if the existing key-preservation test does not already reach a row that started empty.
- [x] 1.3 Confirm slot assignment keeps a launch-time record's pane in its slot while that pane is live, and add coverage for a record whose native session id is still empty.
- [x] 1.4 Write the primary pane's record at that same moment when it has none, so the restart fan-out does not reach the extra pane while skipping the pane beside it, and leave an existing primary record alone. Cover that the primary is tracked.

## 2. Key Minting for an Extra Pane

- [x] 2.1 Mint an identity key when AoE launches an extra agent pane for a Cross Agent Team session, and inject it as `XATS_IDENTITY_KEY` through the existing pane-command environment path rather than a new one.
- [x] 2.2 Persist that key on the pane's launch-time slot record from task 1.1, so the existing "reuse the slot's key on every relaunch" behavior picks it up with no restart-path changes.
- [x] 2.3 Assert the minted key differs from the instance's own key. This is the only point at which two live panes behind one identity is preventable, so treat the coverage as load-bearing rather than incidental.
- [x] 2.4 Mint and inject nothing when Cross Agent Team is disabled, and nothing for a `shell` extra pane.
- [x] 2.5 Assert the key never appears in the agent process's own arguments. It does appear in the pane's start command as the environment assignment the shell consumes, which is the shared injection route and out of scope here.

## 3. Both Launch Entry Points

- [x] 3.1 Wire the new-session right pane launch path so the key it mints is persisted on the instance's stored state, not only on the working copy used to build the command.
- [x] 3.2 Wire the `aoe session add-agent-pane` launch path the same way, and confirm it still builds as a non-primary pane so it cannot pick up the instance's launch context.
- [x] 3.3 Add coverage that both entry points produce a pane whose environment carries a key and whose slot record holds the same value.

## 4. Slot 0 Resume Fallback

- [x] 4.1 Make the fan-out resume restart fall back to the instance's stored resume token when slot 0's record carries no native session id, scoped to slot 0 only.
- [x] 4.2 Cover the three cases: empty native session id resumes from the stored token, a recorded native session id takes precedence over it, and a fresh restart uses neither.
- [x] 4.3 Confirm no other slot consults the instance's resume token.

## 5. Runtime Acceptance Coverage

- [x] 5.1 Add isolated-socket E2E coverage proving a right pane created through the new-session flow carries `XATS_IDENTITY_KEY` at first launch, read from the pane process rather than from a shell inside the pane.
- [x] 5.2 Add isolated-socket E2E coverage proving the right pane's key is identical before and after a clean restart, and different from the left pane's.
- [x] 5.3 Add coverage proving the extra pane is inside the restart fan-out before any capture exists for it, which is the behavior the launch-time slot record buys.
- [x] 5.4 Record that end-to-end identity recovery cannot be asserted from AoE alone, and scope acceptance to key presence, stability, freshness, absence when disabled, and the resume fallback.

## 6. Review Remediation

- [x] 6.1 Make the launch write clear the conversation unconditionally, including when the row names the same pane id. A pane id is not a pane identity across a tmux server restart, and the capture itself is re-applied by the next reconcile from the volatile table the launch write never touches, so matching on the id buys a transient repair at the cost of inheriting a dead pane's conversation.
- [x] 6.6 Make the capture snapshot keep an identity key stored after its caller read one, so a launch that mints a key between that read and the write is not undone by writing the stale read back.
- [x] 6.7 Raise the launch-record failure through a dialog the user has to dismiss instead of the session's error field, which the next status poll overwrites with the value it read before the failure happened.
- [x] 6.2 Write the primary pane's launch-time record with insert-if-absent semantics instead of a read-then-write guard, closing the window in which a capture lands between the two.
- [x] 6.3 Return the launch-record failure to the caller instead of logging a warning, and surface it in both entry points. The pane stays up, because relaunching it would not repair the record.
- [x] 6.4 Narrow the E2E stub to write only the variable under test, so a failed run cannot leave the developer shell's whole environment in a temp file.
- [x] 6.5 Correct the argument-exposure requirement to state what actually happens: the key is an environment assignment the pane's shell consumes, absent from the agent's own arguments but readable from the pane's recorded start command. That exposure predates this change and belongs to the shared injection route, not to extra panes.

## 7. Verification

- [x] 7.1 Run `cargo fmt`, `cargo clippy`, and the test suites, with every tmux-touching test confined to the project harness's private socket. `cargo fmt --check`, `cargo clippy --all-targets` and `cargo check --all-targets` are clean. The suites were run filtered rather than in full, because this machine hosts live AoE tmux sessions: every test this change adds or touches passes (18 unit, 2 new E2E, 3 pre-existing `xats_identity` regressions, `pane_cwd` 2/2). Seven failures were observed and every one was reproduced on a clean-HEAD clone, so none is a regression from this change: three come from a test helper that does not clear `$TMUX`, three from a pre-existing multi-pane adoption race that caps slots below the expected count, and one is `no_bare_tmux_command_outside_seam`, which flags a bare tmux call in `src/cli/record_pane.rs` this change never touched. All seven belong to separate work.
- [x] 7.2 Run `openspec validate mint-extra-pane-identity-key-at-launch` and confirm the implementation and this checklist match every scenario.
