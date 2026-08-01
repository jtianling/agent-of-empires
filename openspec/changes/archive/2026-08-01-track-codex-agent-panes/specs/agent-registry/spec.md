## ADDED Requirements

### Requirement: Codex is a tracked agent, without hooks

The `codex` registry entry SHALL NOT carry a hook configuration. Codex executes hooks inside a shared `--remote` app-server whose environment is frozen at daemon start, so a hook there cannot name the pane its session runs in, and any value it read would be another pane's. A hook configuration in the registry asserts that the agent's hooks run in the agent's own process; Codex cannot make that assertion.

Codex panes SHALL instead be bound to their conversations from Codex's own rollout files (see `pane-session-capture`), and Codex status SHALL come from pane-content detection.

AoE SHALL NOT write anything under `~/.codex/`.

#### Scenario: Codex panes can occupy durable slots
- **WHEN** an AoE-launched Codex agent runs in the primary pane of a managed session
- **AND** its rollout file exists
- **THEN** the reconciler SHALL be able to snapshot that pane into an `agent_slot`
- **AND** the slot's recorded agent SHALL be `codex`

#### Scenario: The user's Codex configuration is not written
- **WHEN** a Codex session is started, restarted, or reconciled
- **THEN** no file under `~/.codex/` SHALL be created or modified by AoE
