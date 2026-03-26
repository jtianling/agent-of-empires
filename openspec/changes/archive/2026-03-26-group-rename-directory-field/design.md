## Context

The `GroupRenameDialog` currently contains a single text input field for editing the group path (rename/move). Groups also have a `default_directory` property (used by `NewSessionDialog` to pre-fill the project path), but there is no TUI-accessible way to edit it directly.

The `PathGhostCompletion` logic -- which provides filesystem directory autocomplete with ghost text -- is implemented as methods on `NewSessionDialog` in `src/tui/dialogs/new_session/path_input.rs`. This is tightly coupled to `NewSessionDialog`'s fields (`self.path`, `self.path_ghost`, `self.error_message`, `self.path_invalid_flash_until`), making it impossible to reuse in other dialogs without duplication.

The existing `GroupGhostCompletion` in `src/tui/components/text_input.rs` provides a good reference for a standalone ghost completion component: it has `compute()` and `accept()` methods that take `&Input` and return values rather than mutating dialog state.

## Goals / Non-Goals

**Goals:**
- Extract `PathGhostCompletion` into a standalone, reusable component following the `GroupGhostCompletion` pattern.
- Refactor `NewSessionDialog` to use the extracted component with zero behavior change.
- Add a "Directory" input field to `GroupRenameDialog` with filesystem path ghost completion.
- Enable Tab/Up/Down focus switching between the two fields.
- Return a `GroupRenameResult` struct from the dialog for future extensibility.

**Non-Goals:**
- Changing the `GroupTree` API or data model (existing `set_default_directory` is sufficient).
- Adding directory validation beyond what path completion already provides.
- Adding ghost completion to the Path (group name) field -- it stays as a plain text input.
- Supporting file (non-directory) completion -- only directories are completed, matching existing behavior.

## Decisions

### Decision 1: Extract PathGhostCompletion as a standalone struct with compute/accept pattern

**Choice**: Create `src/tui/components/path_ghost.rs` with a `PathGhostCompletion` struct that has `compute(input: &Input) -> Option<Self>` and `accept(self, input: &Input) -> Option<String>` methods, mirroring `GroupGhostCompletion`.

**Rationale**: The `GroupGhostCompletion` pattern is already established in this codebase. A standalone struct with compute/accept avoids coupling to any specific dialog's fields. The caller manages when to call `compute()` (after input changes) and `accept()` (on Right/End key), keeping the component pure.

**Alternative considered**: Keep methods on a trait that dialogs implement. Rejected because it adds complexity without benefit -- the struct-based approach is simpler and already proven.

### Decision 2: Move helper functions alongside the component

**Choice**: Move `expand_tilde`, `path_completion_base`, and `char_to_byte_idx` to `src/tui/components/path_ghost.rs`, making `expand_tilde` public (it is used by `NewSessionDialog::submit`) and keeping the others private.

**Rationale**: These functions exist solely to support path completion. Co-locating them with the component makes the module self-contained.

### Decision 3: GroupRenameResult struct for dialog return type

**Choice**: Define `GroupRenameResult { pub new_path: String, pub directory: Option<String> }` where `directory = None` means "clear the default directory" (empty field) and `directory = Some(path)` means "set it to this value".

**Rationale**: A struct is more extensible than a tuple, and the `Option<String>` encoding naturally maps to the existing `GroupTree::set_default_directory` / clear semantics. An empty directory field means the user intentionally wants no default directory for this group.

### Decision 4: Focus switching via Tab, Up, and Down arrows

**Choice**: Support all three keys for switching focus between the two fields, with wrapping (Tab from Directory goes to Path, Down from Directory goes to Path, etc.).

**Rationale**: Tab is the standard convention used by `NewSessionDialog`. Up/Down arrows provide an additional natural navigation pattern. Since neither field is a multi-line input, Up/Down have no conflicting use.

### Decision 5: Pre-fill directory from group default or launch_dir

**Choice**: Pre-fill the Directory field with `group_tree.get_default_directory(path)` if set, otherwise use `launch_dir`. Pass both values into `GroupRenameDialog::new()`.

**Rationale**: Consistent with `NewSessionDialog` behavior where the path field falls back to `launch_dir` when a group has no default directory. The user sees a meaningful starting value rather than an empty field.

## Risks / Trade-offs

- **[Risk] NewSessionDialog refactor introduces regressions** -- Mitigated by making the extraction a pure refactor step with no behavior changes, validated by existing tests. The `recompute_path_ghost` call sites in `NewSessionDialog` will delegate to `PathGhostCompletion::compute()` and store the result in the same `path_ghost` field.

- **[Risk] Breaking change in GroupRenameDialog return type** -- Mitigated by the compiler: changing from `DialogResult<String>` to `DialogResult<GroupRenameResult>` will cause compile errors at every match site, ensuring nothing is missed.

- **[Trade-off] Dialog height increases** -- The dialog grows from 9 to ~13 lines to accommodate the directory field and its label. This is acceptable as it remains well within typical terminal heights.

- **[Trade-off] Empty directory field clears default_directory** -- This means a user cannot "leave directory unchanged" by simply not editing it. However, the field is pre-filled with the current value, so the user would have to intentionally clear it. This is consistent with the group name field behavior where the existing value is the starting point.
