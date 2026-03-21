## Context

AoE manages sessions organized in a flat/grouped list. Current switching relies on `Ctrl+b n`/`p` (group-scoped cycling) or returning to the TUI and navigating with j/k + Enter. With many sessions, direct jumping by number is significantly faster.

The existing session ordering infrastructure (`flatten_tree`, `ordered_profile_session_names`) provides a stable, sort-order-aware session list that both TUI rendering and tmux cycling already share. The number jump feature extends this with index-based access.

## Goals / Non-Goals

**Goals:**
- Enable direct jump to any of the first 99 sessions by number, from both TUI and tmux
- Consistent interaction model: digit(s) + Space to confirm single digit, second digit auto-confirms
- Numbers visible in the TUI list for discoverability
- Keybinding lifecycle follows existing patterns (setup on attach, cleanup on detach/exit)

**Non-Goals:**
- Sessions beyond #99 (use n/p cycling)
- Customizable confirm key (hardcoded to Space)
- Per-group numbering (global numbering only)

## Decisions

### 1. Global numbering over group-scoped numbering

Numbers are assigned globally across all visible sessions in display order, skipping group headers and collapsed group contents. This differs from `n`/`p` which are group-scoped, but the semantics are different: `n`/`p` is relative navigation, `1-99` is absolute addressing. Global numbering matches what the user sees in the TUI -- press the number you see, go there.

Alternative considered: group-scoped numbering (each group restarts from 1). Rejected because it creates ambiguity in the TUI display and would require showing which group's numbering is active in tmux.

### 2. Space-only confirmation for single digits

Only Space confirms a single-digit jump. Enter does not confirm. This avoids accidental jumps in environments where Enter has side effects. Two-digit jumps auto-confirm on the second digit since 99 is the max.

### 3. Tmux key tables for two-phase input

Use tmux's `switch-client -T <table>` mechanism to handle the two-phase digit input:

```
Ctrl+b → prefix table
  1 → switch-client -T aoe-1
  2 → switch-client -T aoe-2
  ...
  9 → switch-client -T aoe-9

aoe-N table:
  Space → run-shell "aoe tmux switch-session --index N"
  0-9   → run-shell "aoe tmux switch-session --index N<digit>"
  (any other key → falls through, cancels)
```

This creates 9 prefix bindings + 9 key tables x 11 bindings each = 108 bindings total. All programmatically generated in a loop.

Alternative considered: `command-prompt -1` for second digit. Rejected because it shows a visible prompt bar that doesn't match the rest of the UX.

### 4. New CLI subcommand parameter: `--index N`

Add `--index N` to the existing `aoe tmux switch-session` command. This reuses the existing `ordered_profile_session_names()` infrastructure but resolves by index (1-based) instead of direction (next/prev). The index is global (not group-scoped).

### 5. TUI pending jump state

Add `PendingJump` state to `HomeView` with first digit and timestamp. While pending:
- Show visual indicator (e.g., the first digit highlighted, or "N_" in status bar)
- Accept second digit (0-9) to form two-digit number and jump immediately
- Accept Space to confirm single digit and jump
- Any other key (including Esc, Enter) cancels the pending state
- No timeout -- user must explicitly confirm or cancel

### 6. Number display in TUI list

Numbers are rendered as a fixed-width prefix before the status icon, right-aligned:

```
 1 ● api-server
 2 ◐ worker
 3 ○ docs
 ...
 9 ● frontend
10 ● dashboard
```

Only sessions get numbers. Group headers have no number prefix (blank space to maintain alignment). Sessions inside collapsed groups are not numbered. Numbers recalculate on every render based on current visible session order.

## Risks / Trade-offs

- **`Ctrl+b 1-9` overrides tmux window switching**: AoE sessions are typically single-window, so this rarely matters. Bindings are only active inside `aoe_*` sessions and cleaned up on exit. Same approach as existing n/p/h/j/k/l overrides. → Acceptable trade-off.
- **108 tmux bindings is a lot**: All generated programmatically in a loop. Performance impact is negligible since `tmux bind-key` is fast. Cleanup is also in a loop. → Acceptable.
- **Index stability during session changes**: If a session is created/deleted while attached, the index mapping changes but the tmux bindings point to the old mapping. The `aoe tmux switch-session --index N` command resolves the index at runtime from current storage, so this is self-correcting. → No issue.
