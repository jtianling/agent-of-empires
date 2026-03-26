# Capability Spec: Notification Keybindings

**Capability**: `notification-keybindings`
**Created**: 2026-03-26
**Status**: Draft

## Purpose

Allow users to switch directly to sessions displayed in the notification bar via dynamic tmux key bindings that update each poll cycle.

## Requirements

### Requirement: Dynamic key bindings for notification bar entries
The notification monitor SHALL manage tmux key bindings that allow users to switch directly to sessions displayed in the notification bar. Bindings SHALL be dynamically updated each poll cycle to reflect the current notification entries.

#### Scenario: Key bindings created for notification entries
- **WHEN** the notification bar displays entries with assigned indices (e.g., `[1] ◐ frontend`)
- **THEN** the monitor SHALL bind keys `1` through `6` in a custom tmux key table (`aoe_notify`)
- **AND** each binding SHALL switch to the corresponding session via `tmux switch-client -t <session_name>`

#### Scenario: Key binding triggers acknowledgment
- **WHEN** the user presses a notification key binding (e.g., `Ctrl+b` then `1`)
- **THEN** the binding SHALL write the target session's instance ID to an ack signal file (`/tmp/aoe-ack-signal`)
- **AND** switch the tmux client to the target session

#### Scenario: Monitor reads and clears ack signal
- **WHEN** the notification monitor begins a poll cycle
- **AND** the ack signal file exists
- **THEN** the monitor SHALL read the instance ID from the file
- **AND** delete the file
- **AND** mark the corresponding session as acknowledged in its state map

#### Scenario: Stale bindings cleaned up
- **WHEN** a notification entry is removed (session ended or status changed)
- **THEN** the monitor SHALL unbind the corresponding key
- **AND** when the monitor exits, it SHALL unbind all notification keys

#### Scenario: Maximum 6 key bindings
- **WHEN** there are more than 6 notification entries
- **THEN** only the first 6 SHALL have key bindings
- **AND** remaining entries SHALL still display in the notification bar without key bindings
