# Capability Spec: Sound Notifications

**Capability**: `sounds`
**Created**: 2026-03-06
**Status**: Stable

## Overview

AoE can play sound effects when agent status transitions occur (e.g., when an agent
finishes a task and becomes Idle/Waiting). This provides audible feedback so users
don't need to watch the TUI constantly.

## Sound Events

| Event | Description |
|-------|-------------|
| `on_waiting` | Agent transitions to Waiting state (needs user input) |
| `on_idle` | Agent transitions to Idle state (task complete) |

## `SoundConfig`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable sound notifications globally |
| `volume` | `f32` | `1.0` | Playback volume (0.0 to 1.0) |
| `on_waiting` | `Option<String>` | None | Sound name or path for waiting event |
| `on_idle` | `Option<String>` | None | Sound name or path for idle event |

## Bundled Sounds

AoE ships with bundled sound files (in `bundled_sounds/`). Users can reference
these by name or provide paths to custom audio files.

Sounds can be listed and previewed via the CLI:
```
aoe sounds list           -- list available bundled sounds
aoe sounds preview <name> -- play a sound for preview
```

## Profile Override

`SoundConfigOverride` allows per-profile sound settings, following the standard
`Option<T>` override pattern. Useful for different notification behaviors per workspace.

## Functional Requirements

- **FR-001**: Sound playback MUST be non-blocking (does not pause the TUI event loop).
- **FR-002**: Sound MUST be disabled by default (`enabled = false`).
- **FR-003**: Volume MUST be configurable between 0.0 and 1.0.
- **FR-004**: Users MUST be able to reference custom audio files by path.
- **FR-005**: Bundled sounds MUST be accessible by short name without full path.
- **FR-006**: Sound settings MUST be overridable per profile.
- **FR-007**: Sound MUST play only on status transitions (not on every poll tick).

## Success Criteria

- **SC-001**: Users receive audible feedback when an agent needs input without watching the screen.
- **SC-002**: Sound playback does not introduce latency or freezing in the TUI.
- **SC-003**: Users can disable or adjust sounds without restarting AoE.
