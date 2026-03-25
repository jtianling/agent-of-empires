## Context

Groups in AoE are identified by slash-delimited paths (e.g., `work/clients/acme`). Group identity is entirely path-based: the "name" is the last segment, and all relationships are derived from path prefixes. Sessions reference groups via `group_path: String`, and group metadata (collapsed state, default_directory) is stored in `groups.json` keyed by path.

Currently, the `r` key in the home view only opens a rename dialog for sessions. Groups have no rename capability, forcing users to delete and recreate groups to reorganize their hierarchy.

## Goals / Non-Goals

**Goals:**
- Allow renaming any group via a single-input dialog pre-filled with the full path
- Cascade path changes to all child groups, their metadata, and all affected sessions
- Handle conflicts when the target path already exists by offering a merge option
- Follow existing dialog patterns (DialogResult, centered_rect, theme colors)

**Non-Goals:**
- Batch rename across multiple groups simultaneously
- Undo/redo for rename operations
- Drag-and-drop group reorganization
- Separate "move" vs "rename" flows (editing any part of the path handles both)

## Decisions

### Decision 1: Single-input dialog rather than multi-field

The rename dialog uses a single text input showing the full slash-delimited path, rather than separate fields for parent path and name.

**Rationale**: Editing the full path in one field lets users both rename (change last segment) and move (change parent segments) in a single operation. This is simpler to implement and more flexible. The existing `RenameDialog` is multi-field but that complexity is justified for sessions which have name + agent + directory fields. Groups only have a path.

**Alternative considered**: Separate "name" and "parent" fields. Rejected because it adds UI complexity for marginal benefit and doesn't match how users think about paths.

### Decision 2: GroupRenameDialog as a new dialog type

Create `src/tui/dialogs/group_rename.rs` as a dedicated dialog rather than reusing the existing `RenameDialog`.

**Rationale**: The existing RenameDialog is tightly coupled to session fields (name, agent, directory). A group rename dialog has fundamentally different state (single path string) and behavior (conflict detection). A separate type is cleaner than trying to generalize the existing dialog.

### Decision 3: Reuse ConfirmDialog for merge confirmation

When the target path conflicts with an existing group, use the existing `ConfirmDialog` pattern with a pending state approach: store the pending rename in HomeView fields, show a ConfirmDialog, and complete/cancel on user response.

**Rationale**: The ConfirmDialog already handles Yes/No flows with proper rendering and key handling. Adding pending state fields (`pending_group_rename: Option<(String, String)>`) to HomeView follows the same pattern used elsewhere (e.g., pending delete confirmations).

### Decision 4: rename_group() on GroupTree handles all cascading

A single `rename_group(old_path, new_path)` method on `GroupTree` handles renaming the target group, updating all child group paths, migrating metadata (collapsed state, default_directory), and is called alongside session path updates from the operations layer.

**Rationale**: Keeping the tree mutation logic in GroupTree maintains separation of concerns. The TUI operations layer coordinates between GroupTree and session storage, but the tree knows how to update itself.

### Decision 5: Merge means union of children

When merging into an existing group, child groups and sessions from the source are moved under the target. If both source and target have a child with the same sub-path, those children recursively merge as well. The target group's metadata (collapsed, default_directory) takes priority over the source's.

**Rationale**: This is the least surprising behavior. Users expect "merge" to combine contents. Target metadata wins because the target group is the one that already exists in the desired location.

## Risks / Trade-offs

- **[Path validation]** Users could enter invalid paths (empty, trailing slashes, double slashes). Mitigation: validate the path before applying and show an error in the dialog if invalid.
- **[Large cascade]** Renaming a top-level group with many descendants requires updating many sessions. Mitigation: this is a simple string prefix replacement operation that completes synchronously; unlikely to be slow even for hundreds of sessions.
- **[Data consistency]** If the app crashes mid-rename, some sessions could have updated paths while others don't. Mitigation: perform all updates in memory first, then save once. The existing save pattern already does this.
