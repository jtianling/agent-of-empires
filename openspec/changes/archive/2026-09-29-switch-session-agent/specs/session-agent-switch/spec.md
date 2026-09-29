## ADDED Requirements

### Requirement: Agent switch keybindings
The home view SHALL bind `A` (Shift+a) and `a` on a selected session to open an agent switch dialog.  Both keys SHALL open the same dialog; they differ only in what happens after the switch is confirmed: `A` SHALL fresh-restart the session and attach to it (same as `C`), `a` SHALL fresh-restart the session in the background and keep the user on the home view (same as `c`).  The keys SHALL do nothing when no session is selected, when the session is deleting, or when a restart is already in flight for it.

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
The switch SHALL only be offered between `claude` and `codex`.  A session whose agent is neither, or a sandboxed session, SHALL NOT open the dialog; the home view SHALL instead show an info dialog explaining why the agent cannot be switched.

#### Scenario: Unsupported agent
- **WHEN** a session running `opencode` is selected and the user presses `a`
- **THEN** the switch dialog does not open and an info dialog explains that only claude and codex sessions can switch agents

#### Scenario: Sandboxed session
- **WHEN** a sandboxed claude session is selected and the user presses `a`
- **THEN** the switch dialog does not open and an info dialog explains that sandboxed sessions cannot switch agents

### Requirement: Agent switch dialog
The dialog SHALL show the session's current agent and let the user pick the target agent among the switchable agents, preselecting the agent that is not the current one.  Left/Right/Tab/Space SHALL cycle the choice, Enter SHALL confirm, Esc SHALL cancel.  Confirming the current agent SHALL behave like cancel.

#### Scenario: Confirm switch
- **WHEN** the dialog for a claude session is open and the user presses Enter without changing the selection
- **THEN** the session is switched to codex

#### Scenario: Cancel
- **WHEN** the user presses Esc in the dialog
- **THEN** the dialog closes and the session is unchanged

#### Scenario: Choosing the current agent
- **WHEN** the user cycles back to the current agent and presses Enter
- **THEN** the dialog closes and the session is unchanged and not restarted

### Requirement: Switch commits before restart
Confirming a switch SHALL durably commit the new agent before any restart begins: the instance's primary pane tool SHALL become the target agent; its command SHALL be reset to what a new session of the target agent would get (the agent's default binary when it has one, or the configured command override for that agent); its extra args SHALL be reset to the configured extra args for the target agent; its resume token, agent session id and fork token SHALL be cleared.  Every durable slot of the session whose agent is `claude` or `codex` SHALL take the target agent, and its native session id, observed model and model fingerprint SHALL be cleared; the pane captures and Codex bindings of those slots SHALL be dropped.  Slots running other agents SHALL keep their agent.  When the session's tmux session is live and it has no durable slot yet, the primary pane SHALL first be recorded as slot 0 so the restart follows the slot's agent rather than the process still running in the pane.  The instance SHALL then be saved.  If committing fails, no restart SHALL start and the error SHALL be shown on the session.

#### Scenario: Switch claude to codex
- **WHEN** a single-pane claude session with a recorded slot 0 is switched to codex
- **THEN** the instance tool and slot 0 agent are `codex`, the instance command is `codex`, slot 0 has an empty native session id and empty model, and slot 0 keeps its xats identity key

#### Scenario: Switch codex to claude
- **WHEN** a codex session is switched to claude
- **THEN** the instance command is reset to empty (no override) or to the configured claude command override, and the codex binding of its slots is removed

#### Scenario: Shell pane untouched
- **WHEN** a session with a claude primary pane and a shell pane in slot 1 is switched to codex
- **THEN** slot 0 becomes codex and slot 1 stays shell

#### Scenario: No slot yet
- **WHEN** a live claude session with no durable slots is switched to codex
- **THEN** slot 0 is recorded for the primary pane with agent `codex` before the restart runs

### Requirement: Fresh restart after switch
After the switch is committed the session SHALL be restarted with fresh mode through the existing restart paths (in-place respawn, cold recovery, or plain start when nothing is running), so every pane launches its slot's agent with a new conversation and no resume.

#### Scenario: Background restart
- **WHEN** the user confirms a switch opened with `a`
- **THEN** the session is queued on the background restart worker with fresh mode and the user stays on the home view

#### Scenario: Attach restart
- **WHEN** the user confirms a switch opened with `A`
- **THEN** the session is fresh-restarted and the user is attached to it

### Requirement: xats identity survives the switch
The switch SHALL keep every slot's xats identity key and the instance's own identity key unchanged, so the new agent launches with the same identity key as the old one and recovers the old agent's xats name and team.

#### Scenario: Identity key unchanged
- **WHEN** a Cross Agent Team session with slot 0 identity key `K` is switched from claude to codex
- **THEN** slot 0 still holds `K` and the codex launch is bootstrapped with `K`
