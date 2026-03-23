## Context

AoE manages tmux keybindings for session navigation (`b`, `n`, `p`, `N`, `P`), number jump (`1-9`), and pane navigation (`h`, `j`, `k`, `l`). All bindings follow a lifecycle: registered in `setup_session_cycle_bindings()`, optionally overridden in `apply_managed_session_bindings()` for nested mode, and cleaned up in `cleanup_session_cycle_bindings()`.

Currently there is no "last pane toggle" binding. Tmux natively supports this via `last-pane` (default `Ctrl+b ;`), but AoE's binding setup overrides the prefix table, so users lose access to default tmux bindings unless AoE explicitly registers them.

## Goals / Non-Goals

**Goals:**
- Bind `Ctrl+Tab` to `last-pane` in AoE-managed sessions.
- Follow the existing keybinding lifecycle so the binding is properly cleaned up.

**Non-Goals:**
- No status bar changes (pane switching is instant and needs no visual indicator).
- No CLI subcommand or AoE-specific logic -- this is a direct tmux `last-pane` passthrough.
- No nested-mode override needed -- `last-pane` is session-local and does not involve profile resolution.

## Decisions

**1. Use `last-pane` directly instead of routing through `aoe` CLI**

tmux's `last-pane` is exactly the behavior needed. Routing through an `aoe tmux` subcommand would add latency and complexity for zero benefit. The `h/j/k/l` pane bindings already use direct tmux commands as precedent.

**2. Bind in prefix table as `C-Tab`**

tmux bind-key syntax: `bind-key C-Tab last-pane`. This means the user presses `Ctrl+b` then `Ctrl+Tab`. This is consistent with how all other AoE bindings work in the prefix table.

Note: `C-Tab` requires tmux 3.1+ with `extended-keys` support. Most modern terminals and tmux versions support this. If the terminal does not support extended keys, the binding simply won't fire (no error, no side effect).

**3. No `apply_managed_session_bindings()` override needed**

Unlike session-switching bindings that need dynamic profile resolution via `@aoe_origin_profile`, `last-pane` is a pure tmux command with no AoE state. The binding set in `setup_session_cycle_bindings()` works identically in both nested and non-nested modes.

## Risks / Trade-offs

- **[Terminal compatibility]** Some older terminals may not send `Ctrl+Tab` as a distinct sequence. Mitigation: this is additive -- users who can't use it still have `Ctrl+b ;` and `Ctrl+b h/j/k/l` available.
- **[tmux version]** `C-Tab` as a key name requires tmux with extended-keys. Mitigation: fail silently on older tmux (bind-key will just not register).
