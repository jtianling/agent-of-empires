## MODIFIED Requirements

### Requirement: CLI switch-session supports --index parameter
The `aoe tmux switch-session` command SHALL accept an `--index N` parameter (1-based) as an alternative to `--direction`. The index resolves against the global ordered session list (same order as TUI display), not scoped to the current group. Upon successful switch, the system SHALL set `@aoe_index` on the target session to its 1-based position in the ordered list.

#### Scenario: Switch by index
- **WHEN** `aoe tmux switch-session --index 3 --profile default` is called
- **THEN** the system SHALL switch to the 3rd session in the global display order

#### Scenario: Index out of range
- **WHEN** `aoe tmux switch-session --index 50` is called
- **AND** there are only 10 sessions
- **THEN** no switch SHALL occur
- **AND** the command SHALL exit successfully (no error)

#### Scenario: Index resolves at runtime
- **WHEN** sessions are created or deleted between the time bindings were set and the jump is triggered
- **THEN** the index SHALL resolve against the current session list at the time of the jump

#### Scenario: @aoe_index set on target after switch
- **WHEN** `aoe tmux switch-session --index 3` is called and the switch succeeds
- **THEN** the target session SHALL have `@aoe_index` set to its current 1-based position
