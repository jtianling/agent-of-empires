## MODIFIED Requirements

### Requirement: All agents MUST have a yolo mode configured
All agents MUST have a `yolo` mode configured, except for non-agent tools (e.g., terminal) where `yolo: None` is permitted.

#### Scenario: Agent tools have YOLO support
- **WHEN** iterating over agent entries in the registry (excluding terminal)
- **THEN** each entry has `yolo.is_some() == true`

#### Scenario: Terminal tool has no YOLO
- **WHEN** querying the terminal entry's YOLO mode
- **THEN** it returns `None`

## ADDED Requirements

### Requirement: Terminal entry in registry
The agent registry SHALL include a `terminal` entry with `name: "terminal"`, positioned after `gemini` and before `cursor` in the `AGENTS` array.

#### Scenario: Terminal is registered
- **WHEN** looking up agent by name "terminal"
- **THEN** an `AgentDef` is returned with `name == "terminal"`

#### Scenario: Registry order includes terminal
- **WHEN** listing all agent names in registry order
- **THEN** the list is `["claude", "opencode", "vibe", "codex", "gemini", "terminal", "cursor"]`

#### Scenario: Settings index accounts for terminal
- **WHEN** converting "terminal" to a settings index
- **THEN** the result is `6` (gemini=5, terminal=6, cursor=7)
