## MODIFIED Requirements

### Requirement: Agent launch command is reusable
The agent launch command construction (binary, extra_args, yolo flags, env vars, custom instruction, model flag) SHALL be extracted into a reusable method so both initial session creation and pane respawn can share the same command-building logic.

The model flag is the pane's observed model, appended after `extra_args`, and applies only to `claude` panes as specified in `claude-model-continuity`. It SHALL be produced by this same reusable method rather than by any per-keybinding or per-restart-path special case, so every caller of the builder gets identical composition.

#### Scenario: Respawn uses same command as initial start
- **WHEN** an agent pane is respawned
- **THEN** the respawn command SHALL be identical to what `start_with_size_opts()` would produce for the same instance configuration
- **AND** env vars, yolo flags, custom instructions, and the model flag SHALL all be applied

#### Scenario: Model flag comes from the shared builder
- **WHEN** any restart path builds a command for a `claude` pane that has an observed model
- **THEN** the model flag SHALL be produced by the shared command builder
- **AND** no restart path SHALL add or omit the model flag on its own
