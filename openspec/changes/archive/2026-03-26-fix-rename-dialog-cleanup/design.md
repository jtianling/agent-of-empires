## Context

The `group-rename-directory-field` change added a directory field to the GroupRenameDialog and extracted `PathGhostCompletion` as a shared component. Code review identified three cleanup issues.

## Goals / Non-Goals

**Goals:**
- Fix tilde expansion gap on directory submit
- Remove suppressed warnings and dead code

**Non-Goals:**
- No behavior changes beyond tilde expansion
- No new features

## Decisions

### Decision 1: Call expand_tilde() in directory_result()

In `GroupRenameDialog::directory_result()`, call `expand_tilde()` on the trimmed directory value before returning. This matches what `NewSessionDialog` does on its path field at submission time.

Import `expand_tilde` from `crate::tui::components::path_ghost`.

### Decision 2: Remove unused re-export and allow attribute

In `src/tui/components/mod.rs`, `longest_common_prefix` was re-exported from `text_input` but is no longer used externally after the `PathGhostCompletion` extraction (it's consumed internally via `super::text_input::longest_common_prefix`). Remove the re-export and the `#[allow(unused_imports)]` attribute.

### Decision 3: Remove candidates field from PathGhostCompletion

The `candidates: Vec<String>` field is populated in `compute()` but never read. Remove it along with its `#[allow(dead_code)]` annotation. If a future need arises for exposing candidates, it can be added back then.

## Risks / Trade-offs

No risks. All changes are mechanical cleanup with no behavioral impact beyond the tilde expansion fix.
