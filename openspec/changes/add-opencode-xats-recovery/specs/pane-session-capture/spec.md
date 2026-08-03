## ADDED Requirements

### Requirement: OpenCode runtime records the exact pane session

The AoE OpenCode runtime SHALL capture the exact session returned by create or verified by resume, keyed by the actual inherited tmux pane.  It SHALL update volatile capture and the matching durable slot without querying the OpenCode session list.

#### Scenario: Fresh session is captured before attach
- **WHEN** a fresh OpenCode runtime creates session `ses_new`
- **THEN** the pane capture SHALL record `ses_new`
- **AND** the target durable slot SHALL converge to `ses_new` before the TUI is attached

#### Scenario: Resume capture preserves the old id
- **WHEN** an OpenCode runtime verifies durable session `ses_old`
- **THEN** the pane capture and durable slot SHALL both record `ses_old`

#### Scenario: Capture cannot claim a sibling pane
- **WHEN** the runtime's inherited pane id is positively owned by another process tree
- **THEN** capture SHALL reject the write
- **AND** SHALL not update any durable slot

#### Scenario: Slot materialization is bounded
- **WHEN** primary tmux creation has returned but its launch-time slot is not yet visible
- **THEN** the runtime MAY retry the exact `(instance_id, slot)` for a bounded interval
- **AND** SHALL wait until that slot is bound to the runtime's inherited `TMUX_PANE`
- **AND** SHALL fail explicitly if the slot never materializes
