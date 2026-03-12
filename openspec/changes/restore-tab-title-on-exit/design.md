## Context

AoE sets the terminal tab title via OSC 0 escape sequences during its TUI lifecycle. On exit (both normal and panic), it calls `clear_terminal_title` which writes an empty OSC 0 (`\x1b]0;\x07`). This leaves the terminal tab with a blank title instead of whatever it was before AoE launched.

The existing code in `src/tui/mod.rs` has three call sites for `clear_terminal_title`:
1. Line 111: panic hook
2. Line 149: normal exit cleanup
3. `src/tui/app.rs` line 453: when user disables `dynamic_tab_title` in settings

## Goals / Non-Goals

**Goals:**
- Save the terminal's current title before AoE modifies it
- Restore the saved title on exit (both normal and panic paths)
- Restore the title when user disables `dynamic_tab_title` in settings mid-session

**Non-Goals:**
- Supporting terminals that don't handle xterm title stack sequences (graceful degradation is acceptable)
- Querying the current title via OSC 21 (too complex for the benefit)

## Decisions

### 1. Use xterm title stack (CSI 22;2 t / CSI 23;2 t) for save/restore

**Decision**: Use the xterm title save/restore stack mechanism.

- Push: `\x1b[22;2t` (CSI 22;2 t) saves the current window title onto a stack
- Pop: `\x1b[23;2t` (CSI 23;2 t) restores the most recently pushed title

**Rationale**: This is the simplest approach with no need to read back terminal state. The terminal itself stores and restores the title. Supported by Alacritty, kitty, iTerm2, WezTerm, xterm, and most modern terminal emulators. For terminals that don't support it, the sequences are silently ignored -- no worse than current behavior.

**Alternative considered**: OSC 21 (query title) requires reading terminal response from stdin in raw mode with a timeout, introducing fragility and race conditions. Rejected.

### 2. Push on startup, pop on all exit paths

**Decision**: Call `push_terminal_title` once at TUI startup (in `src/tui/mod.rs::run`), before the first `set_terminal_title` call. Call `pop_terminal_title` on:
- Normal exit cleanup (replacing current `clear_terminal_title` call)
- Panic hook (replacing current `clear_terminal_title` call)
- Settings toggle to disable `dynamic_tab_title` (replacing current `clear_terminal_title` call in `app.rs`)

### 3. Guard push/pop behind `dynamic_tab_title` config

**Decision**: Only push the title on startup if `dynamic_tab_title` is enabled in config. If the user never enabled the feature, there's no title to restore.

Note: The push must happen before the App is constructed (since config is loaded during App::new). We read the config value from `AppConfig` directly in `run()`, similar to how the tmux mouse/titles state is handled.

## Risks / Trade-offs

- [Risk] Terminal doesn't support title stack sequences -> Title stays as last AoE-set value. No worse than current blank-title behavior.
- [Risk] tmux may intercept CSI 22/23;2 t -> tmux does pass through these sequences when `set-titles` is on, which we already enable. Tested in archived change investigation.
- [Risk] Multiple push without pop (e.g., crash before pop) -> Title stack accumulates entries. This is harmless; terminals handle it gracefully.
