## MODIFIED Requirements

### Requirement: Graceful restart captures resume token from agent output
When the user presses `R`, the restart is handled by the multi-pane store-based flow specified in `multi-pane-resume-restart`. For each tracked agent pane that supports resume, the resume token SHALL be sourced from the persisted `agent_slot.native_session_id` and inserted into the agent command directly. The system SHALL NOT send exit keys to the pane nor scrape a resume token from pane output for the `R` keybinding. A pane with no persisted `native_session_id` or no `ResumeConfig` SHALL restart fresh (no resume token). The resume decision is per tracked pane based on its recorded `agent_slot.agent`, independent of the instance's configured `command`: a pane that ran a resumable agent and recorded a session id resumes even when `instance.command` is a custom command.

#### Scenario: R delegates to per-pane store-based resume
- **WHEN** the user presses R on a session whose agent has a `ResumeConfig`
- **AND** the pane has a persisted `agent_slot.native_session_id`
- **THEN** the system SHALL respawn the pane with `resume_flag` filled from `native_session_id`
- **AND** the system SHALL NOT send exit keys to the pane
- **AND** the system SHALL NOT capture or regex-scrape a resume token from pane output

#### Scenario: Pane without persisted session id restarts fresh
- **WHEN** the user presses R
- **AND** a tracked pane's agent has a `ResumeConfig` but no persisted `native_session_id`
- **THEN** the system SHALL respawn that pane with a fresh command (no resume token)

#### Scenario: Resume is decided per recorded pane agent, not the instance command
- **WHEN** the user presses R on a session whose `instance.command` is a custom command
- **AND** a tracked pane recorded a resumable agent (`agent_slot.agent` with a `ResumeConfig`) and a non-empty `native_session_id`
- **THEN** the system SHALL respawn that pane with `resume_flag` filled from its `native_session_id`

#### Scenario: Agent without ResumeConfig restarts fresh
- **WHEN** the user presses R on a pane whose agent has no `ResumeConfig` (resume is `None`)
- **THEN** the system SHALL respawn that pane with a fresh command (no resume token)

## REMOVED Requirements

### Requirement: Resume restart state machine is tick-driven
**Reason**: The `R` restart now respawns each tracked pane in a single kill-and-respawn step using the persisted `agent_slot.native_session_id`. There is no exit-key/wait/scrape cycle to advance across ticks, and `R` was the only trigger for the graceful state machine, so the tick-driven `pending_resume` state machine is no longer used.
**Migration**: Resume is driven by the persisted session id read from the store at restart time; no per-tick exit-key sending or output scraping is performed. Duplicate-press suppression during an in-flight restart is preserved by the multi-pane flow (`multi-pane-resume-restart`).
