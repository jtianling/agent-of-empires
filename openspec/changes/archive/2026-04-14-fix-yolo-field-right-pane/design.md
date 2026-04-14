## Context

The new session dialog has a `has_yolo` boolean that controls whether the "Skip permission prompts" checkbox is visible. Currently this boolean is computed as `!is_terminal && !self.selected_tool_always_yolo()`, which only examines the left pane's tool. The right pane tool (`self.right_pane_tool_index`) is not consulted.

This logic exists in two places:
- `src/tui/dialogs/new_session/mod.rs` (line ~773) for field index calculation and key handling
- `src/tui/dialogs/new_session/render.rs` (line ~40) for layout constraint building and rendering

## Goals / Non-Goals

**Goals:**
- Show the YOLO checkbox when either pane has a code agent that supports opt-in YOLO mode
- Keep existing behavior for left-pane-only sessions unchanged

**Non-Goals:**
- Separate per-pane YOLO toggles (single checkbox applies to all eligible panes)
- Changes to how `yolo_mode` is applied at session creation time (already works correctly)

## Decisions

**Decision 1: Add `right_pane_needs_yolo()` helper method**

Add a method parallel to `selected_tool_always_yolo()` that checks the right pane tool. Returns `true` when `right_pane_tool_index > 0` (not "none"), the selected tool is not "shell", and the tool's YOLO mode is not `AlwaysYolo`.

Rationale: mirrors the existing left-pane check pattern; keeps the condition readable.

**Decision 2: OR the right pane check into `has_yolo`**

```rust
let has_yolo = (!is_terminal && !self.selected_tool_always_yolo())
    || self.right_pane_needs_yolo();
```

The left-pane condition stays as-is. The right-pane condition is additive (OR). This means: if either pane needs the YOLO checkbox, show it.

## Risks / Trade-offs

[Minimal risk] The field index calculation dynamically adjusts based on `has_yolo`. Since both `mod.rs` and `render.rs` compute field indices identically, they must both use the same updated condition. Mismatch would cause field focus to be off by one.
Mitigation: both files already duplicate the condition; update both.
