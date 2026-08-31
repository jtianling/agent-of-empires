# Delta Spec: agent-registry (codex-update-prompt-defense)

## ADDED Requirements

### Requirement: Codex launch commands suppress the startup update check

Every codex launch command AoE builds SHALL carry the per-invocation config
override `--config check_for_update_on_startup=false`, so a managed codex pane
never blocks on the interactive startup update menu. The suppression SHALL be
declared in the codex registry entry (agent-specific launch knowledge lives in
the registry) and applied by the shared base command builder, so every launch
path that builds a codex command inherits it: primary launch, extra panes,
resume fan-out, fresh restart, cold-start recovery, and sandboxed launches.

The override SHALL ride the command line only. Nothing under `~/.codex/` is
created or modified (existing registry constraint), and a user command override
SHALL keep replacing the built command verbatim, without the suppression flag
appended to it.

#### Scenario: Built codex commands carry the suppression override
- **WHEN** AoE builds a codex launch command on any launch path (primary pane,
  extra pane, resume fan-out, fresh restart, or cold-start recovery)
- **THEN** the command SHALL include `--config check_for_update_on_startup=false`

#### Scenario: The override survives the Cross Agent Team bootstrap wrap
- **WHEN** a codex pane launches with Cross Agent Team enabled and the codex
  bootstrap wraps the launch command
- **THEN** the final codex invocation inside the wrap SHALL still include
  `--config check_for_update_on_startup=false`

#### Scenario: A user command override is not decorated
- **WHEN** an instance has a command override and its command is built
- **THEN** the override SHALL run verbatim, without the suppression flag appended

#### Scenario: Other agents are unaffected
- **WHEN** AoE builds a launch command for a non-codex agent
- **THEN** the command SHALL NOT include `check_for_update_on_startup`
