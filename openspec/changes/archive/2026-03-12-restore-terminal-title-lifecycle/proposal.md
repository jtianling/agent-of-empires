## Why

AoE currently leaves the outer terminal title unmanaged, which causes the Alacritty window title
to go blank when the TUI is active and prevents attached tmux agent sessions from driving the
outer title. Users need a predictable title lifecycle that preserves the pre-launch title, shows a
stable AoE title in the TUI, and mirrors the live agent title while attached to an AoE-managed
session.

## What Changes

- Restore explicit terminal title lifecycle management for the AoE TUI using a stable AoE title
  instead of per-view dynamic titles.
- Configure AoE-managed tmux agent and paired-terminal sessions so the outer terminal title follows
  the active pane title while attached.
- Preserve dynamic title passthrough for agents that already emit OSC title updates, while keeping
  AoE-managed pane titles for agents that do not.
- Restore the terminal title that existed before launching AoE when the TUI exits normally or
  through panic cleanup.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `terminal-tab-title`: AoE will again manage the outer terminal title lifecycle, but only with a
  stable AoE title in the TUI and active-pane passthrough in attached tmux sessions.

## Impact

- Affected code: `src/tui/`, `src/tmux/`, `src/agents.rs`, and title-related tests.
- Affected behavior: terminal title handling while the TUI is active, while attached to AoE-managed
  tmux sessions, and during detach/exit cleanup.
- Dependencies and systems: tmux title options (`set-titles`, `set-titles-string`) and terminal
  title escape sequences supported by Alacritty.
