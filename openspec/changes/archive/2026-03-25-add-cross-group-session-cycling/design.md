## Context

`Ctrl+b n/p` session cycling is scoped by `group_path` via `ordered_scoped_profile_session_names()`.
This function filters sessions to only those matching the current session's `group_path`. Number jump
(`Ctrl+b 1-9`) already uses the unscoped `ordered_profile_session_names()` for global navigation.

The cross-group cycling feature adds `Ctrl+b N/P` (shift variants) that use the same unscoped session
list as number jump, but with prev/next wraparound instead of index-based jumping.

## Goals / Non-Goals

**Goals:**
- Add `Ctrl+b N/P` for cross-group session cycling in both nested and non-nested modes
- Follow the existing keybinding lifecycle (setup, cleanup, nested override)
- Remove `n/p switch` from status bar, fix `1-9 jump` to `1-9 space jump`

**Non-Goals:**
- Adding status bar hints for N/P (intentionally hidden)
- Changing the behavior of existing `n/p` within-group cycling
- Adding TUI-side cross-group navigation (this is tmux-only)

## Decisions

### Cross-group cycling ignores collapsed state

The `flatten_tree()` function skips sessions inside collapsed groups. For cross-group cycling, we
use `ordered_profile_session_names()` which calls `flatten_tree()`. However, collapsed groups should
not block navigation.

**Decision**: Create a variant that calls `flatten_tree` with all groups expanded, or bypass
`flatten_tree` and build the ordered list without respecting collapse state. The simplest approach
is to temporarily uncollapse all groups before flattening, or pass a flag to `flatten_tree`.

**Alternative considered**: Using a separate ordering function. Rejected because `flatten_tree`
already produces the correct display order; we just need it to ignore collapse.

### Reuse switch_aoe_session with a scope parameter

**Decision**: Add a `global: bool` parameter (or an enum) to `switch_aoe_session()` to control
whether scoping is applied. When `global = true`, use `ordered_profile_session_names()` (with
collapse ignored) instead of `ordered_scoped_profile_session_names()`.

**Alternative considered**: Separate `switch_aoe_session_global()` function. Rejected because the
logic is 95% identical; a parameter avoids duplication.

### Shell command generation

**Decision**: Add a `session_cycle_global_run_shell_cmds()` function (or extend the existing one)
that generates shell commands passing `--global` to `aoe tmux switch-session`. The CLI handler
forwards this flag to `switch_aoe_session()`.

## Risks / Trade-offs

- [Collapsed group handling] The `flatten_tree` function interleaves collapsed logic deeply.
  Passing a flag to ignore collapse is the safest approach. -> Mitigation: add an `ignore_collapse`
  parameter to `flatten_tree` or temporarily set all groups to `collapsed = false` on a cloned list.
