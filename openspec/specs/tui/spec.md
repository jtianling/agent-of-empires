# Capability Spec: Terminal User Interface (TUI)

**Capability**: `tui`
**Created**: 2026-03-06
**Status**: Stable

## Purpose

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
│                        │  [R] resume/recover           │
│                        │  [C] clean restart            │
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
| `R` | Resume a live session or recover a persisted session, then attach |
| `C` | Clean-restart agent panes in a live session, then attach |
| `r` | Same as `R`, but stay on the home list instead of attaching |
| `c` | Same as `C`, but stay on the home list instead of attaching |
| `%` | Add a managed agent pane to the selected session, then attach |
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
## Requirements
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
When the user returns from an attached AoE-managed tmux session to the home screen, AoE SHALL restore selection to the session the user actually detached from, even if they switched sessions inside tmux after the initial attach. The client name for per-client tracking SHALL be resolved from the terminal's tty name.

#### Scenario: Detach restores the originally attached session when no cycling occurred
- **WHEN** the user attaches to a session from the home screen
- **AND** the user later returns to the TUI without switching to another managed session first
- **THEN** the home screen SHALL select that same session after the TUI reloads

#### Scenario: Detach restores the cycled-to session
- **WHEN** the user attaches to a session from the home screen
- **AND** the user switches to another AoE-managed session with root-table `Ctrl+.` or `Ctrl+,`
- **AND** the user presses `Ctrl+b d` to return to the TUI
- **THEN** the home screen SHALL select the session the user detached from
- **AND** AoE SHALL NOT force selection back to the originally attached session

#### Scenario: Client name resolved from tty name
- **WHEN** the TUI resolves the attach client name for per-client tracking
- **THEN** the system SHALL use `get_tty_name()` to obtain the terminal's tty path
- **AND** the system SHALL NOT check the TMUX env var for client name resolution

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

### Requirement: R keybinding resumes or recovers the selected session
The TUI home screen SHALL support the `R` (Shift+R) keybinding as the state-aware action for returning to the selected session's persisted conversations. When the tmux session exists, `R` SHALL resume every tracked pane from its persisted `native_session_id` without changing the layout. When the selected instance is recoverable because its tmux session does not exist but durable slots remain, `R` SHALL rebuild and recover it from those slots.

#### Scenario: R on session with dead agent pane
- **WHEN** the user presses `R` on a selected session whose tmux session exists
- **AND** an agent pane is dead
- **THEN** the system SHALL respawn every tracked agent pane through resume mode
- **AND** the session status SHALL transition to `Starting`
- **AND** the session layout SHALL be preserved

#### Scenario: R on session with running agent pane
- **WHEN** the user presses `R` on a selected session whose tmux session exists
- **AND** an agent pane is alive
- **THEN** the system SHALL force-restart every tracked agent pane in resume mode
- **AND** the session status SHALL transition to `Starting`

#### Scenario: R on recoverable session
- **WHEN** the user presses `R` on a selected instance with durable slots whose tmux session does not exist
- **THEN** the system SHALL invoke cold recovery for that instance
- **AND** it SHALL rebuild the session and resume its persisted panes

#### Scenario: R on missing non-recoverable session
- **WHEN** the user presses `R` on a selected instance whose tmux session does not exist
- **AND** the instance has no durable slots
- **THEN** the system SHALL retain the existing normal-start fallback behavior

#### Scenario: R on session being deleted
- **WHEN** the user presses `R` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: R is shown contextually
- **WHEN** the selected instance is recoverable
- **THEN** the home status bar SHALL show `R` as the recover action
- **AND** the help overlay SHALL describe `R` as the resume/recover action

#### Scenario: R is shown for a live session
- **WHEN** the selected instance is not recoverable
- **THEN** the home status bar SHALL show `R` as the resume action

### Requirement: C keybinding restarts agent panes clean
The TUI home screen SHALL support the `C` (Shift+C) keybinding as the state-aware action for starting the selected session's agents over without their previous conversations. When the tmux session exists, `C` SHALL restart every tracked agent pane with a fresh command that carries no resume flag, preserving the session layout. When the selected instance is recoverable because its tmux session does not exist but durable slots remain, `C` SHALL rebuild the session from those slots and launch every recovered pane fresh.

#### Scenario: C triggers a clean restart on a live session
- **WHEN** the user presses `C` on a selected session whose tmux session exists
- **THEN** the system SHALL initiate a fresh restart of the session's tracked agent panes
- **AND** the session layout SHALL be preserved
- **AND** no persisted resume token SHALL be passed to the relaunched commands

#### Scenario: C on a recoverable session triggers clean recovery
- **WHEN** the user presses `C` on a selected instance with durable slots whose tmux session does not exist
- **THEN** the system SHALL invoke cold recovery for that instance in fresh mode
- **AND** it SHALL rebuild the session and launch its persisted panes without any resume flag or token

#### Scenario: C on missing non-recoverable session
- **WHEN** the user presses `C` on a selected instance whose tmux session does not exist
- **AND** the instance has no durable slots
- **THEN** the system SHALL retain the existing single-pane fresh restart fallback

#### Scenario: C on session being deleted is a no-op
- **WHEN** the user presses `C` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: C is shown in help and status hints
- **WHEN** the user opens the help overlay or views the home status bar for a selected session
- **THEN** the TUI SHALL list `C` as the clean restart action
- **AND** the contextual status hint SHALL indicate clean recovery when the selected instance is recoverable

### Requirement: Lowercase r and c restart without attaching
The TUI home screen SHALL support lowercase `r` and `c` as no-attach counterparts of `R` and `C`. They SHALL resolve to the same state-aware action as their uppercase form -- cold recovery when the instance has durable slots and no live tmux session, an in-place respawn otherwise -- with `r` using resume mode and `c` using fresh mode. After the restart or recovery completes, the TUI SHALL remain on the home list instead of attaching to the session, so several sessions can be restarted in a row.

#### Scenario: r resumes a live session in place
- **WHEN** the user presses `r` on a selected session whose tmux session exists
- **THEN** the system SHALL restart every tracked agent pane in resume mode
- **AND** the TUI SHALL stay on the home list without attaching

#### Scenario: c clean-restarts a live session in place
- **WHEN** the user presses `c` on a selected session whose tmux session exists
- **THEN** the system SHALL restart every tracked agent pane in fresh mode
- **AND** the TUI SHALL stay on the home list without attaching

#### Scenario: r and c recover a cold session without attaching
- **WHEN** the user presses `r` or `c` on a selected instance with durable slots whose tmux session does not exist
- **THEN** the system SHALL invoke cold recovery for that instance in resume mode for `r` and fresh mode for `c`
- **AND** the TUI SHALL stay on the home list without attaching

#### Scenario: r and c on a missing non-recoverable session
- **WHEN** the user presses `r` or `c` on a selected instance whose tmux session does not exist
- **AND** the instance has no durable slots
- **THEN** the system SHALL start the session through the normal start path
- **AND** the TUI SHALL stay on the home list without attaching

#### Scenario: r and c on a session being deleted are no-ops
- **WHEN** the user presses `r` or `c` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: Modified lowercase keys do not restart
- **WHEN** the user presses `r` or `c` with any modifier held
- **THEN** the keybinding SHALL NOT trigger a restart or recovery

#### Scenario: r and c are shown in help and status hints
- **WHEN** the user opens the help overlay
- **THEN** the TUI SHALL list `r` and `c` as the stay-in-list restart actions
- **AND** the home status bar SHALL label the restart hints `R/r` and `C/c`

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

### Requirement: TUI panel title uses short form
The TUI home screen left-panel title SHALL display `AoE [{profile}]` instead of `Agent of Empires [{profile}]`, matching the terminal tab title convention and providing more space for the profile name.

#### Scenario: Panel title shows short form
- **WHEN** the TUI home screen is rendered
- **THEN** the left panel title SHALL display `" AoE [{profile}] "` where `{profile}` is the active profile name

#### Scenario: Profile name has more display room
- **WHEN** the terminal width is 80 columns
- **THEN** the shortened title SHALL allow approximately 15 more characters of profile name to be visible compared to the previous format

### Requirement: TUI dialogs that render paths use responsive widths with a 120-column cap

Dialogs that display or edit session paths or nested group paths (New Session, Edit Session, Edit Group, Fork Session, and the New Session sub-dialogs for Sandbox / Tool / Worktree configuration) SHALL compute their container width as `min(terminal_area_width - 4, 120)` rather than using a fixed width.

The cap of 120 columns keeps long lines readable on wide terminals; the `terminal_area_width - 4` floor allows the centered-rect clamp to degrade gracefully on narrow terminals without introducing overflow or panics.

Field layout, input behavior, and keybindings inside the dialogs are unchanged; only the outer container width is affected.

#### Scenario: Wide terminal shows dialogs at the 120-column cap

- **WHEN** the terminal width is 160 columns
- **AND** the user opens the New Session dialog, Edit Session dialog, Edit Group dialog, or Fork Session dialog
- **THEN** the dialog SHALL render at 120 columns wide
- **AND** group paths and filesystem paths up to roughly 110 characters SHALL display without truncation

#### Scenario: Medium terminal scales dialog to available width

- **WHEN** the terminal width is 100 columns
- **AND** the user opens any of the affected dialogs
- **THEN** the dialog SHALL render at 96 columns wide (terminal width minus 4)
- **AND** the dialog SHALL NOT overflow the terminal bounds

#### Scenario: Narrow terminal falls back to clamp behavior

- **WHEN** the terminal width is below 60 columns
- **AND** the user opens any of the affected dialogs
- **THEN** the dialog SHALL render within the terminal bounds using the existing `centered_rect` clamp
- **AND** the application SHALL NOT panic or produce an overflow layout

#### Scenario: Error wrap width tracks the responsive dialog width

- **WHEN** the New Session dialog renders an error message
- **THEN** the error-line wrap calculation SHALL use the same responsive width as the dialog container, not a stale fixed value

### Requirement: Status bar shows current sort order
The home-view status bar SHALL display the current session-list sort order and the key that cycles it, positioned right-aligned at the far right of the status bar so it does not push the left-aligned key hints. The indicator SHALL show the cycle key (`o`) and the current sort label (one of `Newest`, `Oldest`, `A-Z`, `Z-A`, `Manual`). When the current sort order is `Manual`, the status bar SHALL additionally show a `J/K` move hint, since manual reordering is only active in `Manual` sort. The status-bar area SHALL be split into a flexible left region (the existing key hints, which truncate first when the terminal is too narrow) and a fixed-width right region (the sort indicator) so the two regions never overlap.

#### Scenario: Sort order shown for a non-manual mode
- **WHEN** the session list sort order is `Newest`
- **THEN** the status bar SHALL show the cycle key `o` and the label `Newest`, right-aligned
- **AND** the status bar SHALL NOT show the `J/K` move hint

#### Scenario: Manual sort shows the move hint
- **WHEN** the session list sort order is `Manual`
- **THEN** the status bar SHALL show the cycle key `o` and the label `Manual`, right-aligned
- **AND** the status bar SHALL additionally show a `J/K` move hint

#### Scenario: Sort indicator updates when the order is cycled
- **WHEN** the user presses `o` to cycle the sort order
- **THEN** the status-bar sort label SHALL update to the newly selected order
- **AND** the `J/K` move hint SHALL appear only once the order becomes `Manual`

#### Scenario: Left hints truncate before overlapping the sort indicator
- **WHEN** the terminal is too narrow to fit both the left key hints and the right sort indicator
- **THEN** the left key hints SHALL truncate within their region
- **AND** the right sort indicator SHALL remain visible without overlapping the left hints

### Requirement: e keybinding opens the edit/rename dialog
The TUI home screen SHALL support the `e` keybinding to open the rename/edit dialog for the selected session, and the group-rename dialog when a group is selected.

#### Scenario: e opens the session rename dialog
- **WHEN** the user presses `e` on a selected session
- **THEN** the system SHALL open the session rename/edit dialog pre-filled with the session's current title, group, and profile

#### Scenario: e on session being deleted is a no-op
- **WHEN** the user presses `e` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: e is shown in help overlay
- **WHEN** the user opens the help overlay (`?`)
- **THEN** the help SHALL list `e` as "Edit/rename session" or similar description

### Requirement: Percent keybinding adds a managed agent pane

The TUI home screen SHALL support the `%` keybinding to add a managed agent pane to the selected session. `%` SHALL open a dialog offering the agent to launch, defaulting to the session's own tool, and the working directory to launch it in, defaulting to the session's own. On submit the system SHALL add the pane through the add-agent-pane action and then attach to the session.

`%` is chosen for its tmux meaning of "split to the right". AoE already binds `prefix + %` inside attached sessions to a split pinned to `@aoe_project_path`. The two are not the same action and their directory rules deliberately differ:

| Trigger | Result | Directory |
| --- | --- | --- |
| `prefix + %`, attached | raw tmux pane, no agent, no slot record, no identity key | forced to `@aoe_project_path` |
| `%`, home screen | managed agent or shell pane: tool launched, slot recorded, identity key minted when applicable | chosen in the dialog, defaulting to the session's |

A `shell` pane added through `%` SHALL always receive a durable slot, including when it inherits the session's directory. It is a managed pane because AoE launched it through a dialog that assigns its tool and directory, and restart and cold-start recovery SHALL include it. The shell slot holds no identity key and no native conversation id.

A hand-made split has no interface through which to name a directory, so inheriting the session's is the only useful behavior available to it. A managed pane is created through a dialog, which is such an interface. The distinction is whether AoE created the pane as part of its managed lifecycle, not whether the user was attached.

`%` SHALL remain a home-screen keybinding only. It SHALL NOT be added to the tmux key tables, so `prefix + %` keeps its existing meaning inside attached sessions.

#### Scenario: Percent adds a pane and attaches
- **WHEN** the user presses `%` on a selected running session and submits the dialog
- **THEN** the system SHALL add a managed agent pane to that session
- **AND** the TUI SHALL attach to the session

#### Scenario: The dialog defaults to the session's tool and directory
- **WHEN** the user presses `%` on a selected running session
- **THEN** the dialog SHALL preselect the session's own tool
- **AND** the working directory SHALL default to the session's own

#### Scenario: The dialog offers a different agent and directory
- **WHEN** the user presses `%` and chooses an agent other than the session's tool and a different working directory
- **THEN** the added pane SHALL run that agent in that directory

#### Scenario: Percent adds a durable shell pane in the session directory
- **WHEN** the user presses `%`, chooses shell, and keeps the default session directory
- **THEN** the added shell pane SHALL receive a durable slot carrying the session directory
- **AND** restart and cold-start recovery SHALL include it

#### Scenario: Percent on a session that is not running
- **WHEN** the user presses `%` on a selected session whose tmux session does not exist
- **THEN** the system SHALL surface that the session is not running
- **AND** SHALL NOT start the session or create a pane

#### Scenario: Percent at the four-slot cap
- **WHEN** the user presses `%` on a selected session that already has four panes
- **THEN** the system SHALL surface that the cap is reached
- **AND** SHALL NOT create a pane

#### Scenario: Percent on a group row or a session being deleted is a no-op
- **WHEN** the user presses `%` with a group header selected, or on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: Cancelling the dialog creates nothing
- **WHEN** the user presses `%` and cancels the dialog
- **THEN** no pane SHALL be created
- **AND** the TUI SHALL return to the home list without attaching

#### Scenario: Percent is shown in help
- **WHEN** the user opens the help overlay
- **THEN** the TUI SHALL list `%` as the action that adds an agent pane to the selected session

### Requirement: New Session 按 session 与 pane 分区展示

New Session 对话框 SHALL 先展示 session 元数据, 再展示 primary pane 配置, 然后通过可见分割线展示可选 secondary pane 配置。  字段顺序 MUST 为 Title、Group、primary Tool、primary Path、primary YOLO Mode 与 Cross Agent Team、primary Worktree、Right Pane Agent, 以及选择 right pane Agent 后出现的 secondary Path、secondary YOLO Mode 与 Cross Agent Team、secondary Worktree。

对话框 SHALL NOT 展示 Sandbox 字段或 Sandbox 配置入口。

#### Scenario: 默认布局只显示 primary pane 和 right pane selector
- **WHEN** 用户打开 New Session
- **THEN** Title 与 Group SHALL 位于 pane 配置之前
- **AND** primary pane 字段 SHALL 按规定顺序显示
- **AND** primary pane 与 Right Pane Agent 之间 SHALL 有可见分割线
- **AND** Right Pane Agent 默认为 `none`
- **AND** secondary pane 配置 SHALL 被折叠

#### Scenario: 选择 right pane Agent 展开完整配置
- **WHEN** 用户把 Right Pane Agent 从 `none` 改为一个 Tool
- **THEN** secondary Path、适用的 YOLO Mode 与 Cross Agent Team、secondary Worktree SHALL 显示在 selector 下方
- **AND** primary pane 字段 SHALL 保持原值和原顺序

#### Scenario: New Session 没有 Sandbox 入口
- **WHEN** 用户浏览 New Session 的全部可见字段和配置 overlay
- **THEN** Sandbox checkbox SHALL 不存在
- **AND** Sandbox 配置 overlay SHALL 无法从该对话框打开

## Functional Requirements

- **FR-001**: The TUI MUST launch without arguments (`aoe` with no subcommand).
- **FR-002**: Session status MUST update in real-time via background polling.
- **FR-003**: Attaching to a session MUST detach from the TUI and attach the terminal to the tmux session.
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
