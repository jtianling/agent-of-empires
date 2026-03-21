# Capability Spec: Terminal User Interface (TUI)

**Capability**: `tui`
**Created**: 2026-03-06
**Status**: Stable

## Overview

The TUI is a ratatui-based terminal dashboard that provides a visual interface for managing
agent sessions. It launches when the user runs `aoe` without subcommands. All session lifecycle
operations available via CLI are also available in the TUI, plus additional views (diff, settings).

## Screens / Components

```
┌─ Home Screen ─────────────────────────────────────────┐
│  Session List          │  Session Detail / Preview    │
│  (groups + sessions)   │  (status, path, branch, etc) │
│                        │                              │
│  [n]ew  [d]elete  [?]  │  [Enter] attach              │
│  [s] settings          │  [D] diff view               │
│                        │  [r] restart                 │
└────────────────────────┴──────────────────────────────┘

┌─ Diff View ────────────────────────────────────────────┐
│  File List   │  Diff Content (unified diff)           │
│              │  (syntax-highlighted, scrollable)       │
│  [Enter] open in editor                               │
│  [Esc] back to home                                   │
└───────────────────────────────────────────────────────┘

┌─ Settings TUI ─────────────────────────────────────────┐
│  Tabs: General | Session | Sandbox | Worktree |        │
│        Hooks | Repo | Updates | Sound | Theme          │
│  Scope: [Tab] toggle Global / Profile                  │
│  [r] clear profile override, [Esc] save & close       │
└───────────────────────────────────────────────────────┘

┌─ Creation Dialog ──────────────────────────────────────┐
│  Title, Path, Tool, Branch, Sandbox options            │
│  [Enter] create, [Esc] cancel                          │
└───────────────────────────────────────────────────────┘
```

## Key Bindings (Home Screen)

| Key | Action |
|-----|--------|
| `n` | New session |
| `Enter` | Attach to selected agent session |
| `D` | Open diff view for selected session |
| `d` | Delete selected session |
| `r` | Restart selected session |
| `R` | Restart agent pane only |
| `s` | Open settings |
| `?` | Show help |
| `q` | Quit (only plain `q` with no modifiers; `Ctrl+Q` is ignored to prevent accidental quit after tmux detach) |
| `Ctrl+c` | Quit |
| `j` / `k` / arrows | Navigate sessions |
| `g` | Create group |
| `Tab` | Switch sort order |

## Session List

Sessions are displayed in a list with:
- Status indicator (color-coded: Running=green, Waiting=yellow, Idle=gray, Error=red)
- Session title
- Branch name (if worktree, when `show_branch_in_tui=true`)
- Project path (abbreviated)
- Last accessed time

Sessions can be organized into collapsible groups (slash-delimited group paths).

## Polling

Background tasks keep the TUI live:
- `StatusPoller`: updates session statuses from tmux pane content
- `CreationPoller`: monitors async session creation progress
- `DeletionPoller`: monitors async session deletion progress

## Settings TUI

The settings screen supports two scopes:
- **Global**: edits `~/.agent-of-empires/config.toml`
- **Profile**: edits the active profile's override config

Fields show visual indicators when a profile override is active. Pressing `r` clears
the profile override for the selected field. All config sections are represented as tabs.

A **Repo** tab shows and edits `.aoe/config.toml` from the currently selected session's
project directory. The Repo tab is disabled when no session with a project path is selected.

## Diff View

The diff view shows git changes for the selected session's project:
- Left pane: list of changed files
- Right pane: unified diff with syntax highlighting
- `Enter` on a file: opens the file in `$EDITOR`
- Compares against a configured default branch (or auto-detected)
- Configurable context lines

## Terminal Tab Title Integration

### Requirement: New session dialog inherits the selected group context
When the user opens the new session dialog from the home screen, AoE SHALL prefill the dialog's
Group field from the currently selected home-screen item so the user can create a related session
without retyping the group path.

#### Scenario: Selected group prefills the Group field
- **WHEN** the selected home-screen row is a group
- **AND** the user presses `n`
- **THEN** the new session dialog SHALL prefill the Group field with that group's full path
- **AND** the user MAY edit or clear the value before creating the session

#### Scenario: Selected session prefills the Group field from its group
- **WHEN** the selected home-screen row is a session inside a group
- **AND** the user presses `n`
- **THEN** the new session dialog SHALL prefill the Group field with that session's `group_path`
- **AND** the user MAY edit or clear the value before creating the session

### Requirement: Returning from an attached session restores the actual detached session selection
When the user returns from an attached AoE-managed tmux session to the home screen, AoE SHALL
restore selection to the session the user actually detached from, even if they switched sessions
inside tmux after the initial attach.

#### Scenario: Detach restores the originally attached session when no cycling occurred
- **WHEN** the user attaches to a session from the home screen
- **AND** the user later returns to the TUI without switching to another managed session first
- **THEN** the home screen SHALL select that same session after the TUI reloads

#### Scenario: Detach restores the cycled-to session
- **WHEN** the user attaches to a session from the home screen
- **AND** the user switches to another AoE-managed session with `Ctrl+b n` or `Ctrl+b p`
- **AND** the user presses `Ctrl+b d` to return to the TUI
- **THEN** the home screen SHALL select the session the user detached from
- **AND** AoE SHALL NOT force selection back to the originally attached session

### Requirement: TUI integrates terminal tab title updates into event loop
The TUI event loop SHALL compute the current tab title state after processing events and before rendering, and update the terminal tab title when it changes. Title writes SHALL occur alongside the existing synchronized update sequence.

#### Scenario: Title update during normal event loop
- **WHEN** the event loop processes a state change (dialog open/close, view switch, creation start/finish)
- **THEN** the tab title SHALL be updated before the next draw call

#### Scenario: Title update with synchronized output
- **WHEN** the TUI writes a title update
- **THEN** it SHALL be written outside the synchronized update block (before `BeginSynchronizedUpdate`) to avoid interfering with frame rendering

### Requirement: Terminal teardown includes title reset
The terminal teardown sequence in `src/tui/mod.rs` SHALL include a title reset step alongside the existing `LeaveAlternateScreen` and `DisableMouseCapture` cleanup.

#### Scenario: Teardown sequence order
- **WHEN** the TUI exits and restores the terminal
- **THEN** the title reset SHALL execute as part of the teardown sequence, before `LeaveAlternateScreen`

### Requirement: R keybinding restarts agent pane only
The TUI home screen SHALL support the `R` (Shift+R) keybinding to restart only the AoE-managed agent pane of the selected session, without destroying the session or its layout.

#### Scenario: R on session with dead agent pane
- **WHEN** the user presses `R` on a selected session
- **AND** the agent pane is dead
- **THEN** the system SHALL respawn the agent pane with the original agent command
- **AND** the session status SHALL transition to `Starting`
- **AND** the session layout SHALL be preserved

#### Scenario: R on session with running agent pane
- **WHEN** the user presses `R` on a selected session
- **AND** the agent pane is alive
- **THEN** the system SHALL force-restart the agent pane (kill + respawn)
- **AND** the session status SHALL transition to `Starting`

#### Scenario: R on session that does not exist
- **WHEN** the user presses `R` on a selected session
- **AND** the tmux session does not exist
- **THEN** the system SHALL start the session normally (same as attach behavior)

#### Scenario: R on session being deleted
- **WHEN** the user presses `R` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: R is shown in help overlay
- **WHEN** the user opens the help overlay (`?`)
- **THEN** the help SHALL list `R` as "Restart agent" or similar description

### Requirement: Session list displays numeric indices
The TUI session list SHALL display a right-aligned numeric index (1-99) as a fixed-width prefix before the status icon for each visible session. Group headers SHALL show blank space in the index column to maintain alignment.

#### Scenario: Index display with single digits
- **WHEN** sessions 1-9 are visible
- **THEN** indices SHALL be displayed right-aligned in a 2-character-wide column (e.g., ` 1`, ` 2`)

#### Scenario: Index display with double digits
- **WHEN** more than 9 sessions are visible
- **THEN** single-digit indices SHALL be right-aligned (` 1`) and double-digit indices left-aligned (`10`)

#### Scenario: Index display for groups
- **WHEN** a group header is rendered
- **THEN** the index column SHALL be blank (spaces) to maintain alignment with session rows

### Requirement: Pending jump visual indicator
When a pending jump is active, the TUI SHALL display a visual indicator showing the pending digit. The status bar SHALL show the pending state (e.g., `jump: 3_`). The session matching the pending single digit SHALL be visually highlighted.

#### Scenario: Pending state shown in status bar
- **WHEN** the user presses `3` to start a jump
- **THEN** the status bar SHALL show `3_` or similar pending indicator

#### Scenario: Pending state clears after jump or cancel
- **WHEN** the pending jump completes (Space or second digit) or is cancelled
- **THEN** the status bar SHALL return to normal

## Functional Requirements

- **FR-001**: The TUI MUST launch without arguments (`aoe` with no subcommand).
- **FR-002**: Session status MUST update in real-time via background polling.
- **FR-003**: Attaching to a session MUST detach from the TUI and attach the terminal to the tmux session.
- **FR-003a**: When AoE runs inside an existing tmux session, `Ctrl+b d` inside a managed session (`aoe_*`) MUST switch back to the previous session rather than fully detaching the tmux client. If no previous session exists, it SHALL fall back to normal detach. This binding MUST revert to default `detach-client` when the user switches to a non-AoE session, and the hook MUST be cleaned up when the TUI exits.
- **FR-003b**: When AoE runs inside an existing tmux session, the TUI MUST temporarily enable `mouse on` so that crossterm receives proper mouse events (scroll wheel, etc.) instead of tmux converting them to arrow key sequences. The original mouse setting MUST be restored when the TUI exits. Additionally, AoE-managed tmux sessions MUST always have session-level `mouse on` enabled regardless of the user's tmux configuration, so that mouse scroll works correctly when attached to agent sessions.
- **FR-004**: The session list MUST support collapsible group hierarchies.
- **FR-005**: The diff view MUST open files in `$EDITOR` (or a sensible default).
- **FR-006**: Settings MUST save immediately on field change (no explicit "save" button except Esc).
- **FR-007**: Profile override fields MUST be visually distinguished from global-only fields.
- **FR-008**: The Repo settings tab MUST be disabled when no session is selected.
- **FR-009**: The TUI MUST function correctly at terminal widths as narrow as 80 columns.
- **FR-010**: Session creation and deletion MUST show progress feedback during async operations.
- [x] - **FR-011**: The creation dialog's default project path MUST be the directory where the user launched `aoe`, not the process's current working directory at dialog open time. The launch directory SHALL be captured once at TUI startup and reused for all subsequent session creation dialogs.
- **FR-012**: The TUI MUST implement rendering optimizations to prevent visible flickering when running inside a `tmux` session.
- **FR-012a**: When the TUI renders a frame in a terminal that supports Synchronized Output, it SHALL use the terminal's synchronized update sequences to ensure the frame is displayed atomically.
- **FR-012b**: The TUI SHALL batch state changes and perform at most one `terminal.draw()` call per loop iteration to avoid redundant redraw operations.
- **FR-012c**: The TUI SHALL NOT call `terminal.clear()` during its normal event loop unless the terminal state is explicitly known to be corrupted or after returning from an external full-screen process.
- **FR-012d**: The TUI MUST throttle the frequency of redraws triggered by purely visual animations (like spinners) to prevent visual artifacts, with a maximum redraw rate of 10Hz (100ms interval) for such events.
- **FR-012e**: The TUI main loop MUST ensure that all internal state updates, cache refreshes, and terminal status checks are completed *before* initiating a draw operation to ensure the UI is rendered from a settled state.
- **FR-013**: The TUI SHALL optimize the session preview refresh rate and rendering to reduce the performance impact of background `tmux capture-pane` calls.
- **FR-013a**: The TUI SHALL throttle background refreshes of the preview content to a stable rate (e.g., 250ms interval) and only trigger TUI redraws when the content has actually changed.

## Success Criteria

- **SC-001**: Users can manage all session operations without leaving the TUI.
- **SC-002**: Status indicators update within one polling interval of the agent state changing.
- **SC-003**: The diff view accurately reflects uncommitted changes in the session's project.
- **SC-004**: Settings changes take effect immediately for the next session created.
