## Context

AoE manages tmux keybindings for session navigation across two runtime modes (nested and non-nested). Currently, four prefix-table keys handle session cycling:

- `Ctrl+b n` / `Ctrl+b p`: cycle within the current group
- `Ctrl+b N` / `Ctrl+b P`: cycle across all groups (global)

The CLI command `aoe tmux switch-session` supports both `--direction` (next/prev) and an optional `--global` flag to distinguish group-scoped vs. global cycling. All binding setup flows through `setup_session_cycle_bindings()` (both modes) and `apply_managed_session_bindings()` (nested override). Cleanup runs through `cleanup_session_cycle_bindings()` and `cleanup_nested_detach_binding()` (which includes a hardcoded hook command string listing every key to unbind).

## Goals / Non-Goals

**Goals:**
- Replace all four cycling keys (`n`, `p`, `N`, `P`) with two root-table chords: `Ctrl+,` (previous) and `Ctrl+.` (next).
- All cycling uses global (cross-group) ordering -- group-scoped cycling is removed entirely.
- Maintain correct behavior in both nested and non-nested tmux modes.
- Clean up the `--global` flag from the CLI since it becomes the only mode.
- Update existing specs and documentation to reflect the new keybindings.

**Non-Goals:**
- Changing the back-toggle key (`Ctrl+b b`) -- it stays as-is.
- Changing number-jump keybindings (`Ctrl+b 1-9`) -- unchanged.
- Adding any new visual indicator or status bar change for the cycling keys.

## Decisions

### 1. Use root-table bindings (`C-,` / `C-.`) instead of prefix-table

**Rationale**: Root-table keys fire immediately on keypress without requiring the prefix chord first. This makes session cycling a single-keystroke action, matching the ergonomics of `Ctrl+;` (pane cycling) and `Ctrl+q` (detach) which are already root-table. The `,` and `.` keys are mnemonic for `<` (prev) and `>` (next).

**Alternative considered**: Keeping prefix-table bindings with different keys (e.g., `Ctrl+b ,` / `Ctrl+b .`). Rejected because the prefix overhead is the primary friction being addressed.

### 2. Remove group-scoped cycling entirely

**Rationale**: Global cycling already traverses all sessions in display order, making group-scoped cycling redundant. Removing it simplifies the codebase (one code path instead of two) and frees up prefix keys. Users who want to stay within a group can use number-jump to skip directly.

**Alternative considered**: Keeping group-scoped cycling on different keys. Rejected because it adds complexity with little benefit -- the session list is already visually grouped in the TUI, and global cycling naturally passes through groups in order.

### 3. Remove the `--global` flag from `aoe tmux switch-session`

**Rationale**: With group-scoped cycling removed, every `--direction` call is global. The flag becomes meaningless. The `session_cycle_run_shell_cmds()` (non-global variant) and `session_cycle_run_shell_cmds_with_scope()` can be simplified to always use global ordering.

### 4. Root-table bindings need aoe_* session guard

**Rationale**: Root-table keys fire in every tmux session, not just AoE-managed ones. The binding commands must check `#{session_name}` and only act on `aoe_*` sessions, passing through the raw keystroke otherwise (via `tmux send-keys`). This matches the existing pattern used for `C-q` and `C-\;`.

### 5. Hook command string in `setup_nested_detach_binding` must be updated

The hardcoded hook command in `setup_nested_detach_binding()` contains an explicit list of keys to unbind when switching away from a managed session (currently includes `n`, `p`, `N`, `P`). This string must be updated to remove those four keys and add `C-,`/`C-.` root-table unbinds instead (`unbind-key -T root C-,` / `unbind-key -T root C-.`).

## Risks / Trade-offs

- **[Terminal compatibility]** `C-,` and `C-.` may not be recognized by all terminal emulators. Older terminals or some SSH clients may not send distinct escape sequences for these chords. -> Mitigation: These chords work in modern terminals (Alacritty, iTerm2, WezTerm, kitty, Windows Terminal) which are the primary AoE audience. tmux 3.1+ supports `C-,`/`C-.` in key tables. Document the terminal requirement.

- **[Breaking change]** Users accustomed to `Ctrl+b n/p/N/P` will need to relearn. -> Mitigation: The new keys are arguably more discoverable (single keystroke) and the welcome dialog or documentation can be updated. The status bar hint for detach remains; cycling has never been shown in the status bar.

- **[Conflict with tmux defaults]** `Ctrl+b ,` (prefix + comma) is tmux's default "rename window" binding. Since we bind `C-,` in the root table (not the prefix table), there is no conflict -- `C-,` is a different key event from prefix + `,`. However, if a user has customized their tmux config to bind `C-,` in root, our binding will overwrite it for aoe sessions. -> Mitigation: The aoe_* session guard ensures we only intercept in managed sessions and pass through otherwise.
