# Capability Spec: Acknowledged Waiting

**Capability**: `acknowledged-waiting`
**Created**: 2026-03-24
**Status**: Draft

## Purpose

Distinguish between sessions that need user attention (unacknowledged Waiting) and sessions whose output the user has already seen (acknowledged Waiting mapped to Idle). This reduces visual noise by not highlighting sessions the user has already reviewed.

## Requirements

### Requirement: Track user acknowledgment of session output
Each instance SHALL maintain an `acknowledged` flag (transient, not persisted) that tracks whether the user has viewed the session's current output. This flag SHALL be used by both the TUI and the notification monitor to distinguish between "needs attention" (Waiting) and "already seen" (Idle).

#### Scenario: User attaches to session marks as acknowledged
- **WHEN** the user attaches to (switches to) a session in the TUI
- **THEN** the instance's `acknowledged` flag SHALL be set to `true`

#### Scenario: Notification key binding marks as acknowledged
- **WHEN** the user presses a notification bar key binding to switch to a session
- **THEN** the monitor SHALL mark that session as acknowledged in its state map
- **AND** the notification bar SHALL reflect the updated status on the next cycle

#### Scenario: New activity resets acknowledgment
- **WHEN** the `window_activity` timestamp changes for an instance
- **AND** the instance was previously acknowledged
- **THEN** the `acknowledged` flag SHALL be reset to `false`

#### Scenario: New instances start unacknowledged
- **WHEN** a new instance is created or the monitor/TUI restarts
- **THEN** the instance's `acknowledged` flag SHALL default to `false`

### Requirement: Acknowledged Waiting maps to Idle
When content-based detection returns `Waiting` and the instance is acknowledged, the final reported status SHALL be `Idle` instead of `Waiting`. This applies to both the TUI status poller and the notification monitor.

#### Scenario: Unacknowledged Waiting stays Waiting
- **WHEN** content detection returns `Waiting`
- **AND** the instance's `acknowledged` flag is `false`
- **THEN** the reported status SHALL be `Waiting`

#### Scenario: Acknowledged Waiting becomes Idle
- **WHEN** content detection returns `Waiting`
- **AND** the instance's `acknowledged` flag is `true`
- **THEN** the reported status SHALL be `Idle`

#### Scenario: Running status ignores acknowledgment
- **WHEN** content detection returns `Running`
- **THEN** the reported status SHALL be `Running` regardless of the `acknowledged` flag

### Requirement: Acknowledgment applies only to content-based and hook-based Waiting
The acknowledged mapping SHALL apply to both content-based detection and hook-based detection when they return `Waiting`. Title fast-path always returns Running so acknowledgment is not applicable.

#### Scenario: Hook-based Waiting with acknowledgment
- **WHEN** the hook status file reports `Waiting`
- **AND** the instance's `acknowledged` flag is `true`
- **THEN** the reported status SHALL be `Idle`
