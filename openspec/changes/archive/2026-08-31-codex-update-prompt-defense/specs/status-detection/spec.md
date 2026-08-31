# Delta Spec: status-detection (codex-update-prompt-defense)

## ADDED Requirements

### Requirement: A tracked agent pane running a shell is a dead agent

The status poller SHALL report an instance as `Error` when a tracked pane's
recorded agent is not `shell` but the pane's live process is a plain shell (the
pane-died hook's fallback state), with a readable message that names the fallen
pane(s) and points at the restart keys. The recorded agent comes from the pane's
`agent_slot` row, or from the primary pane's tool when no slot row exists; the
live process comes from the `#{pane_current_command}` the batch pane query
already fetches, so detection adds no tmux round-trips.

Detection SHALL NOT fire for panes where a shell is the expected state:
instances whose tool is `shell`, shell slots, instances whose command override
names a shell, and instances within the `Starting` grace period (the launch
wrapper legitimately reports a shell until `exec` replaces it).

The error SHALL clear through the paths that already reset instance errors on
start/restart, not by the poller re-evaluating the pane into a healthy state.

#### Scenario: Fallen codex pane surfaces as an error
- **WHEN** a pane whose `agent_slot` row records `codex` is running a plain
  shell after the Starting grace period has passed
- **THEN** the poller SHALL set the instance status to `Error`
- **AND** the instance error message SHALL name the fallen pane and the restart
  keys

#### Scenario: Multiple fallen panes are reported together
- **WHEN** more than one tracked pane of an instance has fallen back to a shell
- **THEN** the instance SHALL be in `Error` with a message covering the fallen
  panes, not just the first one detected

#### Scenario: Shell tools and shell slots are exempt
- **WHEN** an instance's tool is `shell`, or a tracked slot records a shell
  agent, and its pane runs a shell
- **THEN** the poller SHALL NOT report a fallen agent for that pane

#### Scenario: Command overrides naming a shell are exempt
- **WHEN** an instance's command override resolves to a shell and its primary
  pane runs that shell
- **THEN** the poller SHALL NOT report a fallen agent for that pane

#### Scenario: The launch window does not false-positive
- **WHEN** an instance is `Starting` or within the start grace period and its
  pane still reports the launch wrapper's shell
- **THEN** the poller SHALL NOT report a fallen agent for that pane

#### Scenario: Restarting the instance clears the error
- **WHEN** a fallen instance is restarted or started through any existing
  restart path
- **THEN** the fallen-agent error SHALL be cleared by that path's existing
  error reset
