## ADDED Requirements

### Requirement: A pane being relaunched survives its own process kill

Relaunching a tracked pane SHALL hold that pane open across the process-tree kill it performs, regardless of how the pane was created, and SHALL then set `remain-on-exit` to the value the newly launched agent requires: held open for an agent, closing on exit for a plain shell.

The kill happens outside tmux because an agent's children can outlive the signal a tmux-internal respawn sends them, and tmux destroys a pane whose `remain-on-exit` is off as soon as its process goes -- which would leave the respawn with no pane to target.

The setting SHALL always be written rather than left to whatever the pane last carried, since a pane can be relaunched as an agent after having been created as a shell, or the reverse.

#### Scenario: The only slot of a shell-command instance comes back
- **WHEN** an instance whose command is a shell has a single slot recording an agent
- **AND** that instance is recovered from a cold start
- **THEN** the slot's pane SHALL still exist after the relaunch, running the agent the slot recorded
- **AND** the session SHALL NOT be destroyed by the relaunch

#### Scenario: A relaunched shell pane still closes when it exits
- **WHEN** a slot recording a plain shell is relaunched
- **THEN** its pane SHALL be left closing on exit rather than held open

### Requirement: Recovery reports slots that did not come back

After launching every slot and applying the saved layout, recovery SHALL verify that each durable slot has a live pane in the rebuilt session, and SHALL report each slot that does not as a per-pane failure.

The verification SHALL happen after a brief settle rather than immediately, because a pane can survive its own relaunch and disappear shortly afterwards. The report SHALL identify the slot by the agent and working directory it recorded, which is what the user recognizes, rather than only by a pane id that no longer exists.

Recovery SHALL NOT retry or repair a missing pane; reporting is the required behavior.

#### Scenario: A slot whose pane disappears is reported
- **WHEN** a recovered slot's pane is launched successfully and then disappears
- **THEN** recovery SHALL report that slot as failed
- **AND** the report SHALL name the agent and working directory the slot recorded

#### Scenario: Recovery with every pane present reports no failure
- **WHEN** every durable slot has a live pane after the rebuild
- **THEN** recovery SHALL report no per-pane failure

#### Scenario: A missing pane is not silently repaired
- **WHEN** a recovered slot has no live pane
- **THEN** recovery SHALL NOT relaunch or recreate it
