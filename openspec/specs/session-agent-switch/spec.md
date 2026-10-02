# session-agent-switch Specification

## Purpose
Switch any of a session's panes in place between claude and codex from the home view: each switched pane's conversation is discarded with a fresh restart of that pane only, the other panes are left as they are, and the pane keeps its xats identity key so the new agent recovers the same xats name and team.
## Requirements
### Requirement: Agent switch keybindings
The home view SHALL bind `A` (Shift+a) and `a` on a selected session to open an agent switch dialog.  Both keys SHALL open the same dialog; they differ only in what happens after the switch is confirmed: `A` SHALL fresh-restart the switched panes and attach to the session, `a` SHALL fresh-restart the switched panes in the background and keep the user on the home view.  The keys SHALL do nothing when no session is selected, when the session is deleting, or when a restart is already in flight for it.

#### Scenario: Shift+a opens the dialog
- **WHEN** a claude session is selected and the user presses `A`
- **THEN** the agent switch dialog opens for that session

#### Scenario: Lowercase a opens the dialog
- **WHEN** a codex session is selected and the user presses `a`
- **THEN** the agent switch dialog opens for that session

#### Scenario: Busy session is ignored
- **WHEN** the selected session is deleting or has a restart in flight and the user presses `a` or `A`
- **THEN** no dialog opens and nothing changes

#### Scenario: Group row is ignored
- **WHEN** a group row is selected and the user presses `a` or `A`
- **THEN** no dialog opens

### Requirement: Switchable agents
The switch SHALL only be offered between `claude` and `codex`, and only for panes running one of them.  A session with no such pane, or a sandboxed session, SHALL NOT open the dialog; the home view SHALL instead show an info dialog explaining why the agent cannot be switched.  A session whose tmux session is not running and that has more than one switchable pane SHALL NOT open the dialog either, since bringing it back would restart every pane; the info dialog SHALL ask the user to start the session first.

#### Scenario: Unsupported agent
- **WHEN** a session running `opencode` is selected and the user presses `a`
- **THEN** the switch dialog does not open and an info dialog explains that only claude and codex panes can switch agents

#### Scenario: Sandboxed session
- **WHEN** a sandboxed claude session is selected and the user presses `a`
- **THEN** the switch dialog does not open and an info dialog explains that sandboxed sessions cannot switch agents

#### Scenario: Stopped split session
- **WHEN** a session with a claude pane and a codex pane is not running and the user presses `a`
- **THEN** the switch dialog does not open and an info dialog asks the user to start the session first

### Requirement: Agent switch dialog
The dialog SHALL show one row per switchable pane of the session, each row offering the switchable agents with the pane's current agent marked `(now)`; a row set to its current agent keeps that pane, a row set to another agent switches it.  With more than one switchable pane, rows SHALL be named by where the panes sit when there are exactly two of them (`Left`/`Right`, or `Top`/`Bottom` when stacked), and `Pane N` (slot + 1) otherwise, and SHALL be listed in on-screen order; every row SHALL start set to its current agent.  A lone pane's row SHALL be labeled `Agent:` and SHALL start set to the other agent.  Up/Down SHALL move between rows; Left/Right/Tab/Space SHALL cycle the agent of the focused row.  Enter SHALL confirm every row set to a new agent; with no such row Enter SHALL behave like cancel.  Esc SHALL cancel.

#### Scenario: Confirm switch
- **WHEN** the dialog for a single-pane claude session is open and the user presses Enter without changing the selection
- **THEN** the session is switched to codex

#### Scenario: Switch only the right pane
- **WHEN** a session has a claude pane on the left and a codex pane on the right, the user presses Down, Right and then Enter
- **THEN** only the right pane is switched, to claude, and the left pane is left as it is

#### Scenario: Switch both panes
- **WHEN** a session has a claude pane on the left and a codex pane on the right, the user presses Right, Down, Right and then Enter
- **THEN** the left pane is switched to codex and the right pane to claude

#### Scenario: Nothing set to switch
- **WHEN** a split session's dialog is open and the user presses Enter without changing any row
- **THEN** the dialog closes and nothing changes or restarts

#### Scenario: Cancel
- **WHEN** the user presses Esc in the dialog
- **THEN** the dialog closes and the session is unchanged

#### Scenario: Choosing the current agent
- **WHEN** the user cycles a single-pane session's row back to the current agent and presses Enter
- **THEN** the dialog closes and the session is unchanged and not restarted

### Requirement: Switch commits before restart
Confirming a switch SHALL durably commit the new agent of every switched pane before any restart begins.  When a switched pane is the primary pane (slot 0), the instance's primary pane tool SHALL become the target agent; its command SHALL be reset to what a new session of the target agent would get (the agent's default binary when it has one, or the configured command override for that agent); its extra args SHALL be reset to the configured extra args for the target agent; its resume token, agent session id and fork token SHALL be cleared.  When only other slots are switched, the instance SHALL NOT change.  Each switched durable slot SHALL take its target agent, and its native session id, observed model and model fingerprint SHALL be cleared; its pane capture and Codex binding SHALL be dropped.  Every slot not switched SHALL be left unchanged.  When the session's tmux session is live and it has no durable slot yet, the primary pane SHALL first be recorded as slot 0 so the restart follows the slot's agent rather than the process still running in the pane.  The instance SHALL then be saved.  If committing any pane fails, no restart SHALL start and the error SHALL be shown on the session, naming any panes already switched.

#### Scenario: Switch claude to codex
- **WHEN** a single-pane claude session with a recorded slot 0 is switched to codex
- **THEN** the instance tool and slot 0 agent are `codex`, the instance command is `codex`, slot 0 has an empty native session id and empty model, and slot 0 keeps its xats identity key

#### Scenario: Switch codex to claude
- **WHEN** a codex session is switched to claude
- **THEN** the instance command is reset to empty (no override) or to the configured claude command override, and the codex binding of its slots is removed

#### Scenario: Other panes untouched
- **WHEN** the right pane (slot 1) of a session with claude in both slots is switched to codex
- **THEN** slot 1 becomes codex with an empty native session id, slot 0 keeps its agent and native session id, and the instance tool, command and agent session id are unchanged

#### Scenario: No slot yet
- **WHEN** a live claude session with no durable slots is switched to codex
- **THEN** slot 0 is recorded for the primary pane with agent `codex` before the restart runs

### Requirement: Fresh restart after switch
After the switch is committed, when the session's tmux session is running, only the switched panes SHALL be respawned with fresh mode, launching its slot's agent with a new conversation and no resume; every other pane SHALL keep running untouched.  If a switched slot is no longer tracked, nothing SHALL be restarted and the error SHALL be shown on the session.  When the session is not running (only possible with a single switchable pane), the session SHALL be restarted with fresh mode through the existing restart paths (cold recovery, or plain start).

#### Scenario: Background restart
- **WHEN** the user confirms a switch opened with `a` on a running session
- **THEN** only the switched panes are queued on the background restart worker with fresh mode and the user stays on the home view

#### Scenario: Attach restart
- **WHEN** the user confirms a switch opened with `A` on a running session
- **THEN** only the switched panes are fresh-restarted and the user is attached to the session

#### Scenario: Sibling pane keeps running
- **WHEN** the right pane of a running two-pane session is switched
- **THEN** the left pane's process is not restarted and keeps its conversation

### Requirement: xats identity survives the switch
The switch SHALL keep every switched slot's xats identity key, every other slot's key, and the instance's own identity key unchanged, so the new agent launches with the same identity key as the old one and recovers the old agent's xats name and team.

#### Scenario: Identity key unchanged
- **WHEN** a Cross Agent Team session with slot 0 identity key `K` is switched from claude to codex
- **THEN** slot 0 still holds `K` and the codex launch is bootstrapped with `K`

### Requirement: Identity preview before switching
For Cross Agent Team panes, opening the switch dialog SHALL ask the xats daemon (`POST /api/identity-key/lookup`) who holds the identity key of every switchable pane, and the dialog SHALL show, for every pane currently set to switch, the identity the new agent will come back as, prefixed with the pane's name.  A pane whose key no identity holds, a pane without a key, a daemon without the lookup endpoint (HTTP 404), and a failed lookup SHALL each be shown as a warning instead of an identity.  A 404 SHALL NOT be treated as an unheld key.  The lookup SHALL NOT block the switch.

#### Scenario: Held key
- **WHEN** the left pane is set to switch and the daemon reports its key is held by `mie-main@mie`
- **THEN** the dialog shows `Left: comes back as mie-main@mie`

#### Scenario: Unheld key
- **WHEN** the daemon answers `not_found` for the key of a pane set to switch
- **THEN** the dialog warns that no xats identity holds the key and the pane must register again

#### Scenario: Old daemon
- **WHEN** the daemon answers HTTP 404 for the lookup
- **THEN** the dialog warns that the daemon is too old to check the identity, and the switch can still be confirmed
