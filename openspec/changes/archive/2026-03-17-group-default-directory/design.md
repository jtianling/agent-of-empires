## Context

Groups currently store only `name`, `path`, and `collapsed` fields. When creating a new session, the path field is initialized with the launch directory (where `aoe` was started). Users who create multiple sessions in the same group must re-enter the same directory every time.

The `NewSessionDialog` accepts a `default_group: Option<String>` parameter and a `launch_dir`. When the dialog opens with a pre-selected group, or when the user types/selects a group, the path field should default to that group's saved directory.

## Goals / Non-Goals

**Goals:**
- Capture the project directory from the first session created in a new group as the group's default directory
- Pre-fill the path field when the user selects or types a group that has a default directory
- Only apply the default when the user hasn't manually edited the path field

**Non-Goals:**
- Allowing users to manually edit the group's default directory (can be added later)
- Changing the default directory after it's set (it's a one-time capture from the first session)
- Affecting existing sessions or their paths

## Decisions

### 1. Store `default_directory` on the `Group` struct

Add `default_directory: Option<String>` to `Group` with `#[serde(default)]`. This keeps it backward compatible -- existing `groups.json` files deserialize fine with the field absent (defaults to `None`).

**Alternative considered**: Store it in a separate config file per group. Rejected because it adds file management complexity for a single field.

### 2. Set `default_directory` only when the group is newly created

When `create_session()` in `operations.rs` calls `group_tree.create_group()` and the group didn't exist before, set `default_directory` to the session's project path. If the group already exists, don't change it.

**Alternative considered**: Always update to the latest session's path. Rejected because the user explicitly described "first session's directory" as the desired behavior.

### 3. Apply group default at dialog open and on group field change

Two trigger points:
1. **Dialog open**: If `default_group` is set and that group has a `default_directory`, initialize the path field with it instead of `launch_dir`.
2. **Group field change**: When the group input value changes and resolves to an existing group with a `default_directory`, update the path field -- but only if the user hasn't manually edited the path away from its initial value. Track this with a `path_user_edited` boolean.

This prevents overwriting a deliberately typed path while still being responsive to group selection changes.

**Alternative considered**: Only apply at dialog open time. Rejected because users often type the group name after opening the dialog.

### 4. Pass group tree info to the dialog

The `NewSessionDialog::new()` constructor already receives `existing_groups: Vec<String>`. Extend this to include the default directory by changing it to a `Vec<(String, Option<String>)>` (group path, default directory) or by passing a separate map. A `HashMap<String, String>` of group path to default directory is cleaner and avoids changing the existing `existing_groups` parameter type.

### 5. `GroupTree` API additions

Add to `GroupTree`:
- `set_default_directory(&mut self, path: &str, directory: &str)` - sets `default_directory` for a group
- `get_default_directory(&self, path: &str) -> Option<&str>` - reads `default_directory` for a group
- `get_group_directories(&self) -> HashMap<String, String>` - returns all group paths with their default directories (for passing to the dialog)

## Risks / Trade-offs

- [Risk] Users might expect the default directory to update when they change a session's path later. Mitigation: The feature is documented as "first session's directory" behavior. Manual editing can be added later.
- [Risk] Intermediate auto-created parent groups won't have a `default_directory`. Mitigation: They simply don't have one, and the path field falls back to `launch_dir`. This is expected.
