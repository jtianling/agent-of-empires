## MODIFIED Requirements

### Requirement: Hook captures native session id keyed by tmux pane

The installed agent status hook SHALL, in addition to its existing status-file write, capture the agent's native session id into the SQLite store keyed by `$TMUX_PANE`. The native session id and working directory SHALL be read from the hook's stdin JSON (`.session_id`, `.cwd`), the working directory falling back to `$PWD`.

`$TMUX_PANE` SHALL be trusted only after a pane-ownership check: when the pane it names can be resolved and that pane's root process is not an ancestor of the capture process, no row SHALL be written. A hook that executes outside the pane it inherited `$TMUX_PANE` from -- Codex's shared app-server is the measured case -- would otherwise claim a pane belonging to a different session, and recovery acts on those rows. The check SHALL be positive-only: a pane whose ownership cannot be determined (no tmux server reachable, pane gone) is accepted.

#### Scenario: Session id captured from stdin
- **WHEN** an agent fires a hook event inside a tmux pane
- **AND** the hook stdin JSON contains `session_id`
- **THEN** the store SHALL hold a `pane_live` row for that pane's `$TMUX_PANE`
- **AND** the row's `native_session_id` SHALL equal the stdin `session_id`
- **AND** the row's `cwd` SHALL equal the agent's working directory

#### Scenario: A capture with no session id is not written
- **WHEN** a hook event's stdin carries no `session_id`
- **THEN** the hook SHALL NOT write a capture row
- **AND** the hook SHALL exit successfully without error

#### Scenario: A pane that belongs to another process is not claimed
- **WHEN** a capture runs with a `$TMUX_PANE` naming a resolvable pane
- **AND** that pane's root process is not an ancestor of the capture process
- **THEN** the hook SHALL NOT write a capture row
- **AND** the hook SHALL exit successfully without error

#### Scenario: A capture from inside its own pane is recorded
- **WHEN** a capture runs as a descendant of the pane's own process
- **THEN** the store SHALL hold a `pane_live` row for that pane

#### Scenario: Hand-launched agent without AOE_INSTANCE_ID is still captured
- **WHEN** a user manually runs an agent inside a shell pane (no `$AOE_INSTANCE_ID` in the environment)
- **AND** the pane has a `$TMUX_PANE` value
- **THEN** the hook SHALL still write the `pane_live` capture row
- **AND** the capture SHALL NOT depend on `$AOE_INSTANCE_ID`

#### Scenario: Capture no-ops outside tmux
- **WHEN** an agent fires a hook event but `$TMUX_PANE` is empty (not running inside tmux)
- **THEN** the hook SHALL NOT write a capture row
- **AND** the hook SHALL exit successfully without error

## ADDED Requirements

### Requirement: A Codex pane is bound to its conversation from Codex's rollout files

The reconciler SHALL bind an AoE-launched Codex pane to its conversation without hooks, by reading Codex's own session records under `$CODEX_HOME/sessions` (default `~/.codex/sessions`): one `rollout-<timestamp>-<thread-id>.jsonl` per conversation, whose first line carries the conversation's working directory.

For a Codex instance whose primary pane has no `pane_live` capture, the claim SHALL be the earliest rollout created at or after the pane's process started, whose working directory matches the instance's project path, and whose thread id no other pane or slot already holds. The claim SHALL write a `pane_live` row (`agent = codex`) for the pane, which the existing snapshot path turns into a durable slot.

A pane SHALL only be claimed for while a process in its tree is invoking Codex, matched on the command line rather than the process name (Codex installed through npm runs behind a `node` shim). A pane whose Codex has exited, or a shell pane that merely belongs to a codex-tool instance, SHALL NOT be bound to whatever conversation happened to start in the same directory.

An instance whose command is overridden SHALL NOT be claimed for: the pane runs the user's own program. A resumed pane's conversation predates its respawn and SHALL NOT re-match; its durable slot already carries the right conversation.

#### Scenario: A fresh Codex launch is bound to its rollout
- **WHEN** an AoE-launched Codex instance's primary pane is running
- **AND** a rollout created after the pane started names the instance's project path
- **THEN** the store SHALL hold a `pane_live` row for that pane
- **AND** the row's `native_session_id` SHALL be the rollout's thread id
- **AND** the row's `agent` SHALL be `codex`

#### Scenario: A conversation is never bound to two panes
- **WHEN** a rollout's thread id is already held by another pane or slot
- **THEN** that rollout SHALL NOT be claimed again

#### Scenario: An older conversation in the same directory is not claimed
- **WHEN** a rollout in the instance's project path predates the pane's process
- **THEN** it SHALL NOT be claimed for that pane

#### Scenario: A pane not running Codex is not claimed for
- **WHEN** a codex-tool instance's primary pane holds no process invoking Codex
- **THEN** no rollout SHALL be claimed for that pane
