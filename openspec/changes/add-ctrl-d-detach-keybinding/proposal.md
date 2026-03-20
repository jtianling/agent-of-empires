## Why

Users must press `Ctrl+b d` (two-step prefix sequence) to return from a managed session to the AoE TUI. Adding `Ctrl+d` as a direct (non-prefix) keybinding in managed sessions makes this the most common navigation action faster and more intuitive, while keeping the existing `Ctrl+b d` path intact.

## What Changes

- Bind `Ctrl+d` in the tmux `root` key table so it triggers the same "return to AoE TUI" logic that `Ctrl+b d` currently uses, but only inside AoE-managed sessions (`aoe_*`).
- In non-managed sessions the binding must be absent or pass-through so normal `Ctrl+d` (EOF / shell logout / vim scroll) behavior is unaffected.
- The binding lifecycle (set on attach, refreshed on session-change hook, cleaned up on exit) mirrors the existing `d` prefix binding.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `nested-tmux-detach`: Add `Ctrl+d` as an additional root-table trigger that follows the same return-to-AoE semantics as the existing prefix `d` binding. Only active inside `aoe_*` sessions.

## Impact

- `src/tmux/utils.rs`: `apply_managed_session_bindings()` and `refresh_bindings()` must add/remove the root-table `C-d` binding alongside the existing prefix `d` binding.
- `src/tmux/utils.rs`: cleanup paths (`cleanup_nested_detach_binding`, the non-managed branch of the session-changed hook) must unbind root `C-d`.
- `src/tmux/status_bar.rs`: status bar hint text should mention `Ctrl+d` as an alternative.
- Help text or docs referencing the detach shortcut may need updating.
