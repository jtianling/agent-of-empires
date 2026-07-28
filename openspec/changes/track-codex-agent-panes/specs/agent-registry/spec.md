## ADDED Requirements

### Requirement: A hook configuration declares the settings format it is written in

`AgentHookConfig` SHALL declare the format of the settings file it targets, alongside the path. The hook installer SHALL dispatch on that declaration.

The format cannot be left implicit now that it varies: Claude, Gemini, and Cursor keep JSON settings, and Codex keeps TOML. Inferring it from the file extension would make the registry entry the wrong place to read the answer, and would silently pick a parser for any future agent whose settings happen to end in a familiar suffix.

#### Scenario: Codex declares TOML
- **WHEN** the registry entry for `codex` is read
- **THEN** its hook configuration SHALL name `~/.codex/config.toml`
- **AND** SHALL declare that file's format as TOML

#### Scenario: Existing agents keep JSON
- **WHEN** the registry entries for `claude`, `gemini`, and `cursor` are read
- **THEN** each hook configuration SHALL declare its settings format as JSON

### Requirement: Codex is a tracked agent

The `codex` registry entry SHALL carry a hook configuration, so that Codex panes report themselves into `pane_live` the way every other tracked agent's panes do.

#### Scenario: Codex panes can occupy durable slots
- **WHEN** a Codex agent runs in a pane of a managed session
- **AND** it fires a hook event
- **THEN** the reconciler SHALL be able to snapshot that pane into an `agent_slot`
- **AND** the slot's recorded agent SHALL be `codex`
