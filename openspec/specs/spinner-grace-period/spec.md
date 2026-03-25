# Capability Spec: Spinner Grace Period

**Capability**: `spinner-grace-period`
**Created**: 2026-03-24
**Status**: Draft

## Purpose

Prevent status flicker when an agent's spinner briefly disappears between animation frames. By holding `Running` status for 500ms after the spinner is last seen, transient gaps in spinner detection do not cause visible status transitions.

## Requirements

### Requirement: Hold Running status for 500ms after spinner disappears
When the detection pipeline detects a transition from `Running` to a non-Running status, the system SHALL hold the `Running` status for 500ms. If the spinner reappears within this window, the status SHALL remain `Running` without any visible transition.

#### Scenario: Spinner disappears and reappears within 500ms
- **WHEN** an instance was detected as `Running` on the previous poll
- **AND** the current poll detects a non-Running status (Idle or Waiting)
- **AND** less than 500ms has elapsed since the last spinner detection
- **THEN** the reported status SHALL remain `Running`

#### Scenario: Spinner disappears for longer than 500ms
- **WHEN** an instance was detected as `Running` on the previous poll
- **AND** the current poll detects a non-Running status
- **AND** more than 500ms has elapsed since the last spinner detection
- **THEN** the reported status SHALL transition to the newly detected status

#### Scenario: Grace period resets on new spinner detection
- **WHEN** a spinner is detected (via title fast-path or content parsing)
- **THEN** the `last_spinner_seen` timestamp SHALL be updated to the current time
- **AND** any active grace period SHALL be extended from this new timestamp

### Requirement: Grace period only applies to Running-to-non-Running transitions
The grace period SHALL NOT apply when transitioning between non-Running states (e.g., Idle to Waiting, or Waiting to Idle). It SHALL only activate when the previous status was `Running`.

#### Scenario: Idle to Waiting transition is immediate
- **WHEN** an instance transitions from `Idle` to `Waiting`
- **THEN** the transition SHALL occur immediately without any grace period
