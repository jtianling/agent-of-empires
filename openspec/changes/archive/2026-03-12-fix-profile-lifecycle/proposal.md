## Why

Profile lifecycle has several bugs: profiles are not cleaned up on exit when empty, some profiles are undeletable, and multiple aoe instances from different tmux sessions targeting the same directory create separate profiles instead of sharing one. The new session dialog also unnecessarily allows switching profiles, which is confusing since the session should always belong to the current profile.

## What Changes

- **BREAKING**: Profile resolution no longer uses tmux session name. Directory (canonical path) is the sole auto-profile identifier. All auto-profiles are directory-based: `auto-<dirname>-<hash>`.
- Fix empty profile cleanup on exit: add multi-instance tracking so profiles are only deleted when the last aoe instance in that profile exits with no sessions.
- Lock the profile field in the new session dialog: new sessions always use the current profile, no switching.
- Fix profile deletion: ensure all auto-profiles can be deleted, diagnose and fix cases where `delete_profile` silently fails.

## Capabilities

### New Capabilities

_None_

### Modified Capabilities

- `profiles`: Add multi-instance reference counting for safe cleanup; lock profile during new session creation; fix deletion edge cases.
- `environment-scoping`: Remove tmux-session-based profile resolution; always use directory-based derivation for auto-profiles. Same directory from different tmux sessions resolves to the same profile.

## Impact

- `src/session/mod.rs`: `resolve_profile()` simplified to remove tmux branch; new instance tracking (lock file or PID-based).
- `src/tui/dialogs/new_session/`: Remove profile selection field entirely.
- `src/main.rs`: Register/unregister instance on startup/exit; conditional cleanup.
- `openspec/specs/profiles/spec.md`: Updated requirements for multi-instance cleanup and locked profile in new session.
- `openspec/specs/environment-scoping/spec.md`: Remove tmux-based resolution scenarios.
