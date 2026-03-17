## Why

When working with multiple sessions in the same group, users almost always use the same project directory. Currently, the path field starts empty every time, forcing users to re-enter or re-browse the same directory for each new session in the group. Capturing the first session's directory as the group default eliminates this repetitive step.

## What Changes

- Add a `default_directory` field to the `Group` struct that stores the directory used by the first session created in that group.
- When a new group is created (via creating a session with a new group name), automatically set the group's `default_directory` to that session's project path.
- When opening the new session dialog with a group pre-selected (or when the user types/selects a group that already has a `default_directory`), pre-fill the path field with that directory.
- The pre-filled path is only a default - users can freely modify it.

## Capabilities

### New Capabilities

- `group-default-directory`: Automatically captures and applies a per-group default working directory based on the first session created in that group.

### Modified Capabilities

- `groups`: Group struct gains `default_directory` field; group creation flow sets it from first session's path.
- `session-management`: Session creation flow reads group's default directory to pre-fill the path field.

## Impact

- `src/session/groups.rs`: Add `default_directory: Option<String>` to `Group` struct.
- `src/session/storage.rs`: Serialize/deserialize the new field in `groups.json`.
- `src/tui/dialogs/new_session/mod.rs`: Pre-fill path field when group with default directory is selected.
- `src/tui/home/operations.rs`: Set `default_directory` on group when first session is created in a new group.
- Existing `groups.json` files will gain the new field on next save (defaults to `None`/null, backward compatible via serde defaults).
