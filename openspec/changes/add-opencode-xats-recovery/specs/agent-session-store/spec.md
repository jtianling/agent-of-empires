## ADDED Requirements

### Requirement: Durable slot stores OpenCode xats runtime generation

`agent_slot` SHALL store `xats_runtime_generation` as a non-negative safe integer for every managed pane.  Legacy rows SHALL default to 0.  For a Cross Agent Team OpenCode launch, the store SHALL atomically advance the target slot to a higher generation before reserve; unrelated slots SHALL remain unchanged.

#### Scenario: Generation survives AoE restart
- **WHEN** an OpenCode slot has persisted generation N
- **AND** AoE closes and reopens
- **THEN** the slot SHALL read generation N

#### Scenario: Generation advances per slot
- **WHEN** slot 1 begins a new OpenCode runtime
- **THEN** slot 1 generation SHALL increase by one
- **AND** slot 0 and slot 2 generations SHALL not change

#### Scenario: Concurrent new panes reserve distinct slots
- **WHEN** two host OpenCode panes are prepared concurrently for one instance
- **THEN** slot selection, config persistence and initial generation SHALL be one serialized transaction per pane
- **AND** each pane SHALL receive a distinct durable slot
- **AND** bind or rollback SHALL require the original slot, generation and identity token

#### Scenario: Closed extra pane slot is reclaimed
- **WHEN** every extra slot has a durable row but one bound pane is absent from the current live pane set
- **THEN** a new OpenCode pane reservation SHALL atomically replace that stale row
- **AND** SHALL advance its generation and clear the old pane capture
- **AND** SHALL NOT reclaim any unbound pending row or currently live pane row

#### Scenario: Fresh preparation clears only the target conversation
- **WHEN** a Cross Agent Team OpenCode slot is prepared for `Shift+C`
- **THEN** its generation SHALL advance and its durable native session id SHALL be cleared in one transaction
- **AND** its identity key and pane config SHALL remain unchanged

#### Scenario: Resume preparation preserves the target conversation
- **WHEN** an OpenCode slot is prepared for `Shift+R`
- **THEN** its generation SHALL advance while its durable native session id remains unchanged

### Requirement: Runtime generation schema healing is idempotent

Store schema application and migration v010 SHALL add the generation column to legacy `agent_slot` tables without recreating rows.  Repeated application SHALL be a no-op.

#### Scenario: Legacy store gains generation column
- **WHEN** a profile database predates `xats_runtime_generation`
- **THEN** migration SHALL add the column with value 0 for existing rows
- **AND** existing session, identity and pane config data SHALL remain intact

#### Scenario: Existing column is unchanged
- **WHEN** schema application runs against a store that already has the generation column
- **THEN** it SHALL succeed without adding a duplicate or resetting values
