## MODIFIED Requirements

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

#### Scenario: OpenCode declares resume support
- **WHEN** the OpenCode agent definition is loaded
- **THEN** it SHALL have a `ResumeConfig` whose resume flag template is `--session {}`
- **AND** the token validation SHALL accept OpenCode `ses_...` ids

#### Scenario: Agents without resume support
- **WHEN** agent definitions for vibe, gemini, shell or cursor are loaded
- **THEN** their `resume` field SHALL be `None`

## ADDED Requirements

### Requirement: OpenCode supports direct host launch

The OpenCode registry entry SHALL permit host launch.  Primary and secondary panes SHALL use the same registered binary and launch capability instead of relying on a primary-only command override.

#### Scenario: Secondary OpenCode command builds
- **WHEN** AoE builds a non-sandboxed secondary pane command for OpenCode
- **THEN** command construction SHALL succeed through the registered host launch path
