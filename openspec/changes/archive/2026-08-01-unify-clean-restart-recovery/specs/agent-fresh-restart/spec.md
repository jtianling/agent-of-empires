## ADDED Requirements

### Requirement: Fresh restart extends to recoverable sessions
When the selected instance has durable agent slots but no live tmux session, the fresh restart action SHALL rebuild the tmux session from those slots and launch every recovered pane with a command that does NOT carry a resume flag or a persisted resume token, while preserving each slot's agent, cwd, and launch context (YOLO flag, hook env). It SHALL reuse the existing cold recovery core rather than a parallel implementation.

#### Scenario: Clean recovery rebuilds and launches every slot fresh
- **WHEN** a fresh restart is invoked for a recoverable instance with multiple durable slots
- **THEN** the system SHALL recreate the tmux session and one pane per durable slot
- **AND** each pane SHALL launch its slot's agent in its slot's recorded cwd
- **AND** no launch command SHALL include a `--resume`/`resume` flag or a persisted token
- **AND** the saved pane topology SHALL be restored

#### Scenario: Clean recovery isolates per-pane failures
- **WHEN** a fresh restart is invoked for a recoverable instance
- **AND** one slot cannot be safely launched
- **THEN** the system SHALL launch the remaining slots
- **AND** SHALL record the failing slot's error without aborting the others

#### Scenario: Instance with no durable slots is not clean-recovered
- **WHEN** a fresh restart is invoked for an instance whose tmux session does not exist
- **AND** the instance has no durable slots
- **THEN** the system SHALL NOT attempt cold recovery
- **AND** SHALL fall back to the existing single-pane fresh restart behavior

### Requirement: Clean recovery applies the fresh identity transaction
Clean recovery SHALL perform the same identity handling as a live fresh restart: reallocate the pre-allocated session id, drop any pending fork, commit the new identity only when the primary slot is launched, roll back otherwise, and clear the instance's stale `resume_token` on success.

#### Scenario: Stale resume token is cleared after clean recovery
- **WHEN** a clean recovery completes and its primary slot has been launched
- **THEN** the instance SHALL NOT retain the resume token that pointed at the discarded conversation

#### Scenario: A later fork does not resurrect the discarded conversation
- **WHEN** an instance is clean-recovered
- **AND** the user later forks that session
- **THEN** the fork SHALL NOT be created from the conversation that the clean recovery discarded

#### Scenario: Failed clean recovery rolls the identity back
- **WHEN** a clean recovery is attempted
- **AND** the primary slot is never launched
- **THEN** the reallocated identity SHALL be rolled back
- **AND** the instance SHALL retain its previous identity state
