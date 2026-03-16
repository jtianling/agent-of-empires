# Capability Spec: Terminal Tab Title

**Capability**: `terminal-tab-title`
**Created**: 2026-03-12
**Status**: Stable

## Overview

AoE manages the outer terminal title with a stable AoE title while the TUI is active, restores the
pre-launch title when AoE exits, and configures AoE-managed tmux sessions so attached agent panes
can drive the outer terminal title dynamically.

## Requirements

### Requirement: Terminal tab title reflects TUI state
When the AoE TUI session itself is active, AoE SHALL manage the outer terminal title with a stable
AoE title that includes the current profile name, SHALL restore the pre-launch title when AoE
exits, and SHALL avoid per-view or per-dialog title churn while the TUI remains active.

#### Scenario: TUI launches
- **WHEN** the TUI launches
- **THEN** AoE SHALL save the current outer terminal title using the terminal title stack
- **AND** AoE SHALL set the outer terminal title to `AoE[<profile>]`

#### Scenario: TUI state changes while AoE session is active
- **WHEN** the user opens dialogs, settings, diff view, or returns to the home screen
- **THEN** AoE SHALL keep using the same stable AoE title with profile name
- **AND** AoE SHALL NOT derive different outer terminal titles from those view changes

#### Scenario: User detaches back to the TUI
- **WHEN** the user returns from an attached AoE-managed tmux session back into the AoE TUI
- **THEN** AoE SHALL set the outer terminal title back to the stable AoE title with profile name

#### Scenario: TUI exits or panics
- **WHEN** the TUI exits normally or through the panic cleanup path
- **THEN** AoE SHALL restore the title that was active before AoE launched

### Requirement: Attached AoE tmux sessions propagate the active pane title
When AoE attaches the user to an AoE-managed tmux session, the outer terminal title SHALL follow
the active pane title for the duration of that attachment. Codex CLI sessions SHALL additionally
surface a raised-hand waiting indicator in the pane title whenever Codex is waiting for user input
or approval, without changing title ownership for other session types.

#### Scenario: Agent session is attached
- **WHEN** AoE attaches to a managed agent or paired-terminal tmux session
- **THEN** AoE SHALL configure that tmux session to enable client terminal title updates
- **AND** tmux SHALL derive the outer terminal title from the active pane title

#### Scenario: Agent updates its own title dynamically
- **WHEN** an attached agent that manages its own title writes a new OSC title during runtime
- **THEN** the pane title SHALL update
- **AND** the outer terminal title SHALL update to match without requiring AoE to poll or redraw

#### Scenario: AoE manages pane titles for the agent
- **WHEN** an attached session uses an agent where AoE manages the pane title
- **THEN** AoE-managed pane title updates SHALL become visible in the outer terminal title while
  attached

#### Scenario: Codex CLI waits for user input
- **WHEN** a running Codex CLI session reaches a waiting-for-input or waiting-for-approval state
- **THEN** the tmux pane title SHALL become `✋ <session title>`
- **AND** the outer terminal title SHALL reflect that waiting title while attached

#### Scenario: Codex CLI resumes from waiting
- **WHEN** a Codex CLI session leaves the waiting state and returns to running or idle
- **THEN** the tmux pane title SHALL revert to `<session title>`
- **AND** the outer terminal title SHALL stop showing the raised-hand prefix while attached

#### Scenario: Non-Codex sessions keep their existing title behavior
- **WHEN** AoE manages or attaches to a session that does not use Codex CLI
- **THEN** this change SHALL NOT alter that session type's title ownership or waiting-title behavior
