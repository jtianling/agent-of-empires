## ADDED Requirements

### Requirement: Codex is a tracked agent

The `codex` registry entry SHALL carry a hook configuration, so that Codex panes report themselves into `pane_live` the way every other tracked agent's panes do.

The configuration SHALL target `~/.codex/hooks.json`, the dedicated hooks file Codex reads beside `config.toml`. AoE SHALL NOT write `~/.codex/config.toml`, which is a file the user owns and edits.

The configuration SHALL declare Codex's own event names. Codex has no `Notification` event; `PermissionRequest` is what stands for a pane waiting on the user.

#### Scenario: Codex panes can occupy durable slots
- **WHEN** a Codex agent runs in a pane of a managed session
- **AND** it fires a hook event
- **THEN** the reconciler SHALL be able to snapshot that pane into an `agent_slot`
- **AND** the slot's recorded agent SHALL be `codex`

#### Scenario: The user's Codex configuration is not written
- **WHEN** AoE installs Codex hooks
- **THEN** it SHALL write `~/.codex/hooks.json`
- **AND** `~/.codex/config.toml` SHALL be unchanged
