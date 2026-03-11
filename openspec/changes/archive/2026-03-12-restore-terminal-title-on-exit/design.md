## Context

AoE sets the terminal tab title via OSC 0 escape sequences during its TUI lifecycle. On exit, it writes an empty OSC 0 (`\x1b]0;\x07`), which leaves terminals like Alacritty with a blank tab title. The expected behavior is to restore the original title that was present before AoE launched.

## Goals / Non-Goals

**Goals:**
- Save the terminal's current title before AoE modifies it
- Restore the saved title on exit (both normal and panic paths)

**Non-Goals:**
- Supporting terminals that don't use OSC sequences
- Saving/restoring titles across AoE sessions (persistence)

## Decisions

### 1. Title query mechanism: xterm title reporting (OSC 21) vs save/restore stack (XTPUSHCOLORS/XTPOPCOLORS)

**Decision**: Use the xterm title save/restore stack via CSI 22;2 t (push) and CSI 23;2 t (pop).

**Rationale**: OSC 21 (report title) requires reading back from stdin in raw mode with a timeout, which is fragile and adds complexity. The CSI 22;2 t / CSI 23;2 t mechanism is simpler - it tells the terminal to push the current title onto a stack, and pop restores it. This is supported by most modern terminals including Alacritty, kitty, iTerm2, WezTerm, and xterm. For terminals that don't support it, the behavior degrades gracefully to the current behavior (title left as-is after pop, which is no worse than clearing to empty).

**Alternative considered**: OSC 21 query - rejected due to complexity of reading terminal response with timeout in raw mode, and potential race conditions.

### 2. Fallback behavior for unsupported terminals

**Decision**: No explicit fallback. If CSI 23;2 t is not supported, the terminal simply ignores the sequence and the title remains as the last AoE-set value (or empty if cleared). This is acceptable and no worse than the current behavior.

## Risks / Trade-offs

- [Risk] Terminal doesn't support title stack → Title may not restore, but no worse than current behavior (clearing to empty)
- [Risk] tmux may intercept title stack sequences → Need to verify tmux passes through or handles CSI 22/23;2 t. If not, fall back to sending empty OSC 0 inside tmux (existing behavior).
