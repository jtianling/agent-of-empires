## 1. Group struct and storage

- [x] 1.1 Add `default_directory: Option<String>` field to `Group` struct in `src/session/groups.rs` with `#[serde(default)]`
- [x] 1.2 Update `Group::new()` to initialize `default_directory` as `None`
- [x] 1.3 Add `set_default_directory(&mut self, path: &str, directory: &str)` method to `GroupTree`
- [x] 1.4 Add `get_default_directory(&self, path: &str) -> Option<&str>` method to `GroupTree`
- [x] 1.5 Add `get_group_directories(&self) -> HashMap<String, String>` method to `GroupTree` for dialog use

## 2. Session creation flow

- [x] 2.1 In `src/tui/home/operations.rs` `create_session()`, check if group already exists before calling `create_group()`
- [x] 2.2 If the group is new, call `set_default_directory()` with the session's project path after `create_group()`

## 3. New session dialog

- [x] 3.1 Add `group_directories: HashMap<String, String>` field to `NewSessionDialog`
- [x] 3.2 Add `path_user_edited: bool` field to track manual path edits
- [x] 3.3 Update `NewSessionDialog::new()` to accept group directories and pre-fill path from group default when `default_group` has one
- [x] 3.4 Track when path field is manually edited by the user (set `path_user_edited = true`)
- [x] 3.5 When group field value changes and matches an existing group with a default directory, update path field if `path_user_edited` is false
- [x] 3.6 Update the caller in `src/tui/home/input.rs` to pass group directories from `GroupTree`

## 4. Tests

- [x] 4.1 Unit test: `Group` serialization/deserialization with `default_directory` field
- [x] 4.2 Unit test: `set_default_directory` and `get_default_directory` on `GroupTree`
- [x] 4.3 Unit test: `get_group_directories` returns correct mapping
- [x] 4.4 Unit test: backward compatibility -- loading groups without `default_directory` field
