## Why

Groups cannot be renamed after creation. Users who want to reorganize their group hierarchy must delete groups and recreate them with different names, losing collapse state and default directory settings. This is especially painful for nested group trees where renaming a parent should cascade to all children.

## What Changes

- Pressing `r` on a selected group opens a rename dialog with the full group path pre-filled
- User can edit any part of the path, effectively enabling both rename and move in one operation
- If the target path already exists as a group, a merge confirmation dialog appears
- Declining merge cancels the operation; accepting merge combines the groups
- All child groups and sessions with matching `group_path` prefixes are updated cascadingly
- Group metadata (collapsed state, default_directory) is migrated to the new path

## Capabilities

### New Capabilities
- `group-rename`: Single-input dialog for renaming/moving groups with conflict detection, merge confirmation, and cascading path updates for children and sessions

### Modified Capabilities
- `groups`: Adding rename requirement (FR for group rename with cascading updates to children and sessions)

## Impact

- `src/tui/dialogs/` - New `group_rename.rs` dialog module
- `src/tui/home/input.rs` - Extended `r` key handler to support groups
- `src/tui/home/mod.rs` - New `group_rename_dialog` state field
- `src/tui/home/render.rs` - Render the new dialog
- `src/tui/home/operations.rs` - Group rename operation logic
- `src/session/groups.rs` - New `rename_group()` method on GroupTree
- `src/tui/home/tests.rs` - Update existing test that asserts `r` does nothing on groups
