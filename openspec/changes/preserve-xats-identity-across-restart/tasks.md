## 1. External Contract

- [x] 1.1 Fix the environment variable name with the xats side before any code is written, since changing it after minting begins invalidates every key already issued. Agreed name: `XATS_IDENTITY_KEY`, chosen to avoid colliding with the existing `XATS_TOKEN` bearer credential present in the same launcher shell.
- [ ] 1.2 Confirm the xats side implements the three-way registration binding rule (unknown key binds, same record is idempotent, key held by a record with no live process migrates, key held by a record with a different live process is rejected with the previous team and name named in the error) rather than a flat conflict, so a legitimate rename does not fail.
- [ ] 1.3 Confirm the xats side implements the per-tool reconnect shapes (`{identity_key, ui_pid}` for claude, `{identity_key, thread_id}` for codex with no process id) and orders the identity-key branch ahead of the existing startup-hint branches.

## 2. Durable Storage

- [x] 2.1 Add the `xats_identity_key` column to the durable slot record through the migration system and the idempotent schema definition, and add the primary pane's key to the instance record.
- [x] 2.2 Extend the idempotent schema-healing path to add the column to legacy databases, matching how the durable slot pane column is healed, and cover both the healing and the already-healed cases.
- [x] 2.3 Extend the slot store APIs to read and write a slot's identity key, with deterministic database tests.

## 3. Key Lifecycle

- [x] 3.1 Mint an identity key when AoE launches a Cross Agent Team pane that has none, persisting it on the instance record for the primary pane and on the slot record for an adopted pane.
- [x] 3.2 Reuse the slot's stored key on every subsequent launch, covering resume restart, clean restart, resume recovery, and clean recovery, without adding identity logic to any restart path.
- [x] 3.3 Make reconcile preserve a slot's existing identity key when it rewrites the row from a pane capture, since a capture carries no key.
- [x] 3.4 Mint a fresh key for new-from-selection and for fork, and add coverage proving neither inherits the source key. This is the only point at which two panes claiming one identity can be prevented, so treat the coverage as load-bearing rather than incidental.

## 4. Launch Injection

- [x] 4.1 Inject the key as `XATS_IDENTITY_KEY` into the claude launch command through the process environment, coexisting with the development-channels flag, the YOLO flag, and the auto-confirm flow.
- [x] 4.2 Inject the key as `XATS_IDENTITY_KEY` into the codex launch command through the process environment, keeping it distinct from the existing single-use pane pre-registration nonce, which continues to be generated per launch.
- [x] 4.3 Assert the key never appears in launch command arguments and is never logged.
- [x] 4.4 Confirm no key is minted or injected when Cross Agent Team is disabled.

## 5. Degradation and Adoption Limits

- [x] 5.1 Ensure a key that no longer resolves produces no session error, does not clear the stored key, and leaves the pane usable for manual registration.
- [x] 5.2 Ensure a hand-started pane adopted into a slot is left untouched while running, and receives a minted key at its first AoE relaunch.

## 6. Runtime Acceptance Coverage

- [x] 6.1 Add isolated-socket E2E coverage proving a Cross Agent Team pane's injected key value is identical before and after a clean restart.
- [x] 6.2 Add isolated-socket E2E coverage proving a clean recovery injects the key stored on each durable slot record.
- [x] 6.3 Add coverage proving a session created through new-from-selection carries a different key from its source. Covered at unit level rather than E2E: that path builds a fresh instance through the builder instead of cloning the source, so the property is structural and a full TUI dialog run would not test anything the unit test does not.
- [x] 6.4 Record that end-to-end identity continuity cannot be asserted until the xats side ships, and scope the AoE acceptance to key presence, stability, freshness on clone, and absence when disabled.

## 7. Verification

- [x] 7.1 Run `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo test --test e2e` with all tmux-touching tests confined to the project harness's private socket.
- [x] 7.2 Run `openspec validate preserve-xats-identity-across-restart` and confirm the implementation and task checklist match every scenario.
