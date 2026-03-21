## Why

`Ctrl+b n/p` cycles sessions within the current group, which is the right default. But when using
groups heavily to organize projects, there is no keyboard shortcut to quickly move across group
boundaries without using number jump. Adding `Ctrl+b N/P` (shift) fills this gap as a hidden
power-user feature.

## What Changes

- Add `Ctrl+b N` (next) and `Ctrl+b P` (prev) keybindings for cross-group session cycling
- Cross-group cycling traverses ALL sessions in global display order, ignoring group boundaries and collapse state
- Remove `Ctrl+b n/p switch` hint from tmux status bar (declutter, these are discoverable)
- Change `Ctrl+b 1-9 jump` status bar hint to `Ctrl+b 1-9 space jump` (accurate description)

## Capabilities

### New Capabilities

- `cross-group-cycling`: Tmux keybindings (Ctrl+b N/P) to cycle through all sessions across groups

### Modified Capabilities

- `groups`: Session cycling now has two scopes: within-group (n/p) and cross-group (N/P)

## Impact

- `src/tmux/utils.rs`: New keybinding setup, shell commands, cleanup
- `src/tmux/status_bar.rs`: Status bar text changes
- No breaking changes, no config changes, no data migration
