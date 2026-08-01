## ADDED Requirements

### Requirement: Clean recovery inherits the single-launch rebuild

Clean recovery SHALL use the same placeholder session rebuild as resume recovery, so an agent that refuses to start on its existing conversation cannot destroy the session before the fresh, no-resume launch runs.

#### Scenario: Clean recovery of an agent that exits immediately still relaunches every slot
- **WHEN** a recoverable instance whose agent exits immediately is recovered in fresh mode
- **THEN** the rebuilt session SHALL exist
- **AND** every durable slot SHALL be relaunched without a resume flag
