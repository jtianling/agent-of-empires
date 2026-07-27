## 1. Bound the Event Stream

- [x] 1.1 Append a `capture` event only when the pane's captured native session id differs from the one recorded on the slot, leaving `adopt` and the durable row refresh unchanged.
- [x] 1.2 Prune events on schema application by retention window and per-instance row cap, with deterministic database tests for each bound and for a store that is already within them.
- [x] 1.3 Reclaim freed space only after a prune that removed rows, so a normal open does no extra work.
- [x] 1.4 Remove an instance's event rows when its durable records are deleted, and cover that other instances' rows survive.

## 2. Contain an Unreadable Store

- [x] 2.1 Detect corruption by SQLite result code when applying the schema, move the file aside under a timestamped name, and create an empty database in its place.
- [x] 2.2 Leave non-corruption failures (permissions, locking, missing directory) surfacing as ordinary errors, with coverage proving no quarantine happens.
- [x] 2.3 Cover a corrupt file and a non-database file, asserting the original is preserved and never deleted.

## 3. Surface the Quarantine

- [x] 3.1 Warn the user when a database was quarantined, naming the preserved path, reusing the existing startup warning path rather than adding a new mechanism.

## 4. Runtime Acceptance Coverage

- [x] 4.1 Add isolated-socket E2E coverage proving AoE starts in a profile whose database is corrupt, rather than aborting with a database error.
- [x] 4.2 Assert the corrupt file is still present under its quarantined name after that startup.

## 5. Verification

- [x] 5.1 Run `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo test --test e2e` with all tmux-touching tests confined to the project harness's private socket.
- [x] 5.2 Run `openspec validate harden-agent-session-store --strict` and confirm the implementation and task checklist match every scenario.
