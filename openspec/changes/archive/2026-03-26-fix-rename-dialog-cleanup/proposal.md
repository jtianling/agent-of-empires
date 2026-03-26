## Why

The recent `group-rename-directory-field` change introduced three minor issues identified during code review: missing tilde expansion on directory submit, a suppressed unused import warning, and a speculative dead-code field. These should be cleaned up before merge.

## What Changes

- Add `expand_tilde()` call on directory value in `GroupRenameDialog::directory_result()` so `~/...` paths are expanded to absolute paths before storage
- Remove `#[allow(unused_imports)]` from `src/tui/components/mod.rs` and clean up the unused `longest_common_prefix` re-export
- Remove the `candidates` field and `#[allow(dead_code)]` from `PathGhostCompletion` since it is never read

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

(none -- these are implementation-only fixes that don't change spec-level behavior)

## Impact

- `src/tui/dialogs/group_rename.rs` -- add tilde expansion import and call
- `src/tui/components/mod.rs` -- remove allow attribute and unused re-export
- `src/tui/components/path_ghost.rs` -- remove `candidates` field and dead_code allow
