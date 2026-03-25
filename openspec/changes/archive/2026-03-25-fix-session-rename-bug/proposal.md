## Why

Session rename is broken: `rename_selected()` mutates the instance title in memory before calling `tmux rename-session`, so the tmux Session object is constructed with the NEW name, which doesn't exist yet. The rename becomes a no-op, leaving the tmux session with its old name while AoE looks for the new one. The status poller then marks the session as Error, making it appear that the processes were killed.

## What Changes

- Fix the ordering bug in `rename_selected()`: construct the old tmux Session (with old title) before mutating the instance, then rename from old name to new name
- Fix the same bug in the cross-profile rename path (lines 280-294) where `orig_inst.title` is compared against `effective_title` after the instance was already mutated
- The `Session::rename()` method itself (`tmux rename-session`) is correct and needs no changes

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `session-management`: fix the rename operation so tmux session name stays in sync with the AoE instance title

## Impact

- `src/tui/home/operations.rs`: `rename_selected()` method -- reorder tmux rename to happen before or use pre-mutation session name
- No API changes, no data format changes, no new dependencies
