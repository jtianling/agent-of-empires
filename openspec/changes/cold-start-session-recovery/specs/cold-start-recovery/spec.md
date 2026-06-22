## ADDED Requirements

### Requirement: Recoverable instance detection

The system SHALL classify an instance as **recoverable** when it has one or more
persisted `agent_slot` rows AND its tmux session does not currently exist. An
instance with no persisted slots, or whose tmux session is alive, SHALL NOT be
classified as recoverable.

#### Scenario: Slots persist but tmux session is dead

- **WHEN** AoE loads an instance that has `agent_slot` rows and `Session::exists()` returns false for it
- **THEN** the instance is classified as recoverable

#### Scenario: Live session is never recoverable

- **WHEN** an instance's tmux session exists
- **THEN** the instance is not classified as recoverable regardless of its `agent_slot` rows

#### Scenario: No persisted slots means not recoverable

- **WHEN** an instance has no `agent_slot` rows
- **THEN** the instance is not classified as recoverable even when its tmux session is dead

### Requirement: Recoverable instances are surfaced in the home view

The system SHALL display a visible recoverable marker on each recoverable instance in
the home list and SHALL show a status bar hint describing the recovery action while a
recoverable instance is focused.

#### Scenario: Marker shown for recoverable instance

- **WHEN** the home view renders a recoverable instance
- **THEN** a recoverable marker is shown for that instance

#### Scenario: Status bar advertises the recovery key

- **WHEN** a recoverable instance is focused in the home view
- **THEN** the status bar shows the recovery action hint

#### Scenario: Non-recoverable instances show no recovery hint

- **WHEN** a non-recoverable instance is focused
- **THEN** the status bar does not show the recovery action hint

### Requirement: Manual per-instance recovery action

The system SHALL provide a manual recovery action that operates on the single focused
recoverable instance. Cold start SHALL be manual and per-session: AoE SHALL NOT
auto-rebuild every recoverable session on startup. The recovery action SHALL be a
no-op when the focused instance is not recoverable.

#### Scenario: Recovery triggers only on user action

- **WHEN** AoE starts up with one or more recoverable instances
- **THEN** no session is rebuilt until the user invokes the recovery action on a focused recoverable instance

#### Scenario: Recovery action on a non-recoverable instance does nothing

- **WHEN** the user invokes the recovery action while a non-recoverable instance is focused
- **THEN** no session rebuild is attempted and no error is surfaced

### Requirement: Session rebuild from persisted slots

On recovery, the system SHALL rebuild the instance's tmux session restoring the
instance's configured worktree/sandbox context, recreate exactly one pane per
persisted slot in ascending slot order (slot 0 as the primary `@aoe_agent_pane`,
remaining slots as additional panes), and resume each pane from its
`agent_slot.native_session_id` using the per-pane resume-launch core. Each pane's
working directory SHALL be its `agent_slot.cwd`.

#### Scenario: Rebuild recreates one pane per slot and resumes each

- **WHEN** the user recovers an instance that has N persisted slots (1 <= N <= 4)
- **THEN** the tmux session is recreated with N agent panes and each pane's launch command resumes its agent from that slot's `native_session_id`

#### Scenario: Slot 0 becomes the primary agent pane

- **WHEN** an instance is recovered
- **THEN** the pane recreated for slot 0 is pinned as `@aoe_agent_pane`

#### Scenario: Each recovered pane uses its recorded working directory

- **WHEN** a slot is recovered
- **THEN** the recreated pane's working directory is that slot's recorded `cwd`

### Requirement: Per-pane degrade and isolation on recovery

A slot whose `native_session_id` is empty or invalid, or whose agent has no resume
support, SHALL be relaunched fresh for that pane only. A per-pane failure SHALL NOT
abort recovery of the remaining panes.

#### Scenario: Invalid id degrades that pane to fresh

- **WHEN** a recovered slot has an empty or invalid `native_session_id`
- **THEN** that pane is launched fresh without a resume flag while the other panes still resume

#### Scenario: One pane failure does not abort the rest

- **WHEN** recovery of one pane fails
- **THEN** the remaining panes are still rebuilt and the failure is recorded rather than aborting the whole recovery

### Requirement: Pane id write-back after recovery

After a successful rebuild, the system SHALL update each persisted slot's
`agent_slot.tmux_pane` to the newly created pane id and SHALL re-pin `@aoe_agent_pane`
to the slot-0 pane, so that the reconciler and the `R` resume-all flow continue to
operate on the rebuilt session.

#### Scenario: New pane ids are written back to the store

- **WHEN** recovery recreates the panes
- **THEN** each slot's `agent_slot.tmux_pane` is updated to the new pane id for that slot

#### Scenario: Reconcile keeps working after recovery

- **WHEN** an instance has been recovered
- **THEN** `@aoe_agent_pane` points at the rebuilt slot-0 pane so subsequent reconcile ticks keep the slots current
