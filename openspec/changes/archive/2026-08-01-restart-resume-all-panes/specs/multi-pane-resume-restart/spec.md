## ADDED Requirements

### Requirement: R restart fans out to all tracked agent panes
When the user presses `R` on an instance, the system SHALL restart every tracked agent pane of that instance, not only the `@aoe_agent_pane`. The set of tracked panes SHALL be read from the agent session store via `read_slots_for_instance(instance_id)`, which returns up to 4 `agent_slot` rows (slot 0 is the primary `@aoe_agent_pane`).

#### Scenario: Multi-pane session resumes every tracked pane
- **WHEN** the user presses `R` on an instance with 3 tracked agent panes recorded in `agent_slot`
- **THEN** the system SHALL restart all 3 panes
- **AND** each pane SHALL be respawned in its own `agent_slot.tmux_pane` target
- **AND** each pane SHALL run in its own `agent_slot.cwd`

#### Scenario: Single tracked pane behaves like the prior single-pane restart
- **WHEN** the user presses `R` on an instance with exactly 1 tracked agent pane (slot 0)
- **THEN** the system SHALL restart that one pane
- **AND** the session layout and any user-created (untracked) panes SHALL be preserved

#### Scenario: No tracked panes falls back to primary agent pane restart
- **WHEN** the user presses `R` on an instance with no `agent_slot` rows
- **THEN** the system SHALL restart the primary `@aoe_agent_pane` using the existing single-pane behavior

### Requirement: Each tracked pane resumes from its persisted native session id
For each tracked pane whose agent has a `ResumeConfig` and a non-empty `agent_slot.native_session_id`, the system SHALL build the resume command by substituting that `native_session_id` into the agent's `resume_flag`, and respawn the pane with that command. The system SHALL NOT send exit keys or scrape a resume token from pane output for these panes; the persisted session id is used directly.

#### Scenario: Claude pane resumes with persisted id
- **WHEN** a tracked pane runs `claude` with `agent_slot.native_session_id = 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11`
- **AND** the user presses `R`
- **THEN** the respawn command for that pane SHALL include `--resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11` after the `claude` binary
- **AND** the system SHALL NOT send exit keys to that pane before respawn
- **AND** the system SHALL NOT scrape a resume token from that pane's output

#### Scenario: Codex pane resumes with persisted id
- **WHEN** a tracked pane runs `codex` with `agent_slot.native_session_id = 019d1af9-a899-7df1-8f7d-a244126e5ded`
- **AND** the user presses `R`
- **THEN** the respawn command for that pane SHALL include `resume 019d1af9-a899-7df1-8f7d-a244126e5ded` after the `codex` binary

#### Scenario: Process tree killed before respawn
- **WHEN** a tracked pane is resumed
- **THEN** the system SHALL kill only that pane's process tree before respawning it
- **AND** processes in other tracked panes SHALL be terminated only by their own per-pane restart

### Requirement: tmux pane operations target an explicit pane
The system SHALL provide pane-parameterized variants of respawn, process-tree kill, and send-keys that operate on an explicit `tmux_pane` target rather than resolving the session-scoped `@aoe_agent_pane`. The existing `@aoe_agent_pane`-scoped behavior SHALL remain valid for the primary pane (slot 0).

#### Scenario: Respawn targets the specified pane
- **WHEN** the system respawns a tracked pane with target `%37`
- **THEN** the respawn SHALL use `tmux respawn-pane -k -t %37`
- **AND** no other pane SHALL be respawned by that call

#### Scenario: Process kill targets the specified pane
- **WHEN** the system kills the process tree for a tracked pane with target `%37`
- **THEN** only the process tree rooted at pane `%37` SHALL be terminated

### Requirement: Per-pane failure isolation
A pane that cannot be resumed SHALL degrade to a fresh restart of that pane only, and SHALL NOT prevent the other tracked panes from restarting. A pane degrades to fresh restart when its agent has no `ResumeConfig`, when it has no usable `native_session_id`, or when the resume respawn fails.

#### Scenario: Pane without resume support restarts fresh
- **WHEN** the user presses `R`
- **AND** one tracked pane runs an agent with no `ResumeConfig` (e.g. gemini, shell)
- **AND** another tracked pane runs `claude` with a persisted `native_session_id`
- **THEN** the no-resume pane SHALL be respawned with a fresh command (no resume flag)
- **AND** the `claude` pane SHALL be respawned with its `--resume <native_session_id>` command
- **AND** neither pane's restart SHALL be blocked by the other

#### Scenario: Pane with empty native session id restarts fresh
- **WHEN** a tracked pane's agent has a `ResumeConfig` but `agent_slot.native_session_id` is empty
- **AND** the user presses `R`
- **THEN** that pane SHALL be respawned with a fresh command (no resume flag)

#### Scenario: Failed resume respawn does not abort sibling panes
- **WHEN** resuming one tracked pane returns an error from tmux respawn
- **THEN** the system SHALL record the error for that pane
- **AND** SHALL continue restarting the remaining tracked panes

### Requirement: Aggregated restarting status during multi-pane restart
While a multi-pane restart is in flight, the instance status SHALL be `Restarting`, and after all tracked panes have been respawned the status SHALL transition to `Starting`.

#### Scenario: Status reflects in-flight multi-pane restart
- **WHEN** a multi-pane restart begins
- **THEN** the instance status SHALL be `Restarting`
- **AND** when every tracked pane has been respawned, the status SHALL transition to `Starting`

#### Scenario: Duplicate R press during multi-pane restart is ignored
- **WHEN** a multi-pane restart is already in flight for an instance
- **AND** the user presses `R` again on the same instance
- **THEN** the system SHALL ignore the second press
