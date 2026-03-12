## Context

AoE currently does not manage the outer terminal title at all while the TUI is active. That keeps
the code simple, but in Alacritty it leaves the title blank and prevents attached AoE-managed tmux
sessions from surfacing the agent's runtime title in the outer window. The codebase already has two
important building blocks:

- `src/tui/status_poller.rs` keeps pane titles up to date for agents that do not manage their own
  title.
- `src/agents.rs` already distinguishes between agents that set their own title and agents that
  need AoE-managed pane titles.

tmux's model also matters here. The pane title and the outer client terminal title are separate.
`select-pane -T` or OSC title sequences update the pane title; tmux only propagates that to the
outer terminal when `set-titles` is enabled and `set-titles-string` references the active pane
title.

## Goals / Non-Goals

**Goals:**
- Give the AoE TUI a stable, non-blank outer terminal title.
- Restore the terminal title that existed before launching AoE when AoE exits.
- Make attached AoE-managed tmux sessions drive the outer terminal title from the active pane title.
- Preserve live passthrough for agents like Claude and Gemini that update their own titles while
  running.
- Reapply the AoE TUI title after returning from an attached tmux session.

**Non-Goals:**
- Reintroducing per-view or per-dialog TUI title changes.
- Adding a new user-facing configuration toggle for title management.
- Managing terminal titles for arbitrary non-AoE tmux sessions.

## Decisions

### 1. Restore a small TUI title lifecycle with one stable AoE title

AoE will set a single stable outer terminal title while the TUI is active, for example `Agent of
Empires`, and will not vary it by view, dialog, or status.

**Rationale:** The user wants the title to stop going blank, but does not want the earlier dynamic
per-view title feature that changed several times and expanded the failure surface.

**Alternative considered:** Reintroduce the earlier dynamic TUI title state machine. Rejected
because it couples title updates to many TUI states and is unrelated to the actual requirement.

### 2. Use terminal title stack push/pop for AoE launch and exit

AoE will push the current terminal title before setting its own stable title, and pop it during
normal teardown and panic cleanup.

**Rationale:** The user explicitly wants the pre-launch title back after exiting AoE. The title
stack is the smallest mechanism that matches that requirement without reading a terminal response
from stdin.

**Alternative considered:** Query and cache the current title with a terminal response sequence.
Rejected because it requires response parsing in raw mode and is more fragile than push/pop.

### 3. Let tmux own title propagation while attached to AoE-managed sessions

Before attaching to an AoE-managed agent or paired-terminal tmux session, AoE will configure that
session so tmux sets the outer terminal title from the active pane title. AoE should rely on
`set-titles on` and a `set-titles-string` based on `#T`, rather than writing outer-title OSC
sequences while tmux is attached.

**Rationale:** tmux already understands active pane titles. Using tmux's own propagation path
allows live passthrough for agents that emit their own OSC title changes and automatically picks up
AoE-managed pane titles for agents that do not.

**Alternative considered:** Have AoE poll the pane title and write OSC 0 itself while attached.
Rejected because AoE is not running while control is inside attached tmux, and tmux already solves
that problem.

### 4. Keep pane title ownership split by agent capability

AoE will keep the existing `sets_own_title` split:

- agents with `sets_own_title = true` are expected to update the pane title directly;
- agents with `sets_own_title = false` continue to receive AoE-managed pane titles such as the
  session title or waiting-state variant.

**Rationale:** This preserves the working behavior already present in `status_poller.rs` and turns
it into a complete outer-title pipeline once tmux title propagation is enabled.

### 5. Reapply the AoE title after returning from tmux attach

AoE will set the stable TUI title again after `with_raw_mode_disabled(...)` returns from an attach
operation, before continuing normal redraw flow.

**Rationale:** tmux will leave the terminal title at whatever the attached pane last set. Without an
explicit reapply step, detaching back into the AoE TUI would keep showing the agent title.

## Risks / Trade-offs

- [Risk] Title stack support differs across terminals. -> Mitigation: target Alacritty first, and
  keep the implementation isolated so unsupported terminals degrade without affecting session logic.
- [Risk] tmux title options could bleed into unrelated sessions if set globally. -> Mitigation:
  scope title propagation changes to AoE-managed target sessions instead of mutating global tmux
  defaults.
- [Risk] Panic cleanup may miss title restoration if the restore logic lives only in normal exit.
  -> Mitigation: call the same title restore helper from both the panic hook and standard teardown.
