## ADDED Requirements

### Requirement: AgentDef supports optional resume configuration
`AgentDef` SHALL include a `resume: Option<ResumeConfig>` field. Agents that support session resumption declare their exit sequence, output pattern, and resume CLI flag via this field. Agents that do not support resume set this to `None`.

#### Scenario: Claude declares resume support
- **WHEN** the Claude agent definition is loaded
- **THEN** it SHALL have a `ResumeConfig` with:
  - exit sequence: two Ctrl+C key groups (one per tick)
  - resume pattern matching `claude --resume` followed by a UUID
  - resume flag template `--resume {}`
  - timeout of 10 seconds

#### Scenario: Codex declares resume support
- **WHEN** the Codex agent definition is loaded
- **THEN** it SHALL have a `ResumeConfig` with:
  - exit sequence: two Ctrl+C key groups (one per tick)
  - resume pattern matching `codex resume` followed by a UUID
  - resume flag template `resume {}`
  - timeout of 10 seconds

#### Scenario: Agents without resume support
- **WHEN** agent definitions for opencode, vibe, gemini, shell, or cursor are loaded
- **THEN** their `resume` field SHALL be `None`

### Requirement: ResumeConfig structure
`ResumeConfig` SHALL contain: an exit key sequence (array of key groups sent one group per tick), a regex pattern for capturing the resume token (first capture group), a flag template with `{}` placeholder for the token, and a timeout in seconds.

#### Scenario: ResumeConfig fields are complete
- **WHEN** a `ResumeConfig` is defined for an agent
- **THEN** it SHALL have all four fields: `exit_sequence`, `resume_pattern`, `resume_flag`, `timeout_secs`
- **AND** `resume_pattern` SHALL contain exactly one capture group for the token
