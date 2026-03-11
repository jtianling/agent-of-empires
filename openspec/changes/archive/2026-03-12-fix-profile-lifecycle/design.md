## Context

AoE uses profiles to isolate sessions by context. Profile resolution currently prioritizes tmux session name, then falls back to directory-based naming. This causes several problems:

1. **Same directory, different profiles**: Two tmux sessions in the same directory produce different auto-profiles (e.g., `auto-dev-a1b2` vs `auto-main-c3d4`), splitting sessions that should be shared.
2. **Broken cleanup**: `cleanup_empty_profile()` unconditionally deletes empty profiles on exit, but if two aoe instances share a profile, one exiting can delete the profile while the other is still running.
3. **Undeletable profiles**: Some profiles fail to delete silently because `delete_profile()` errors are swallowed.
4. **Confusing new session dialog**: The profile field in the new session dialog allows switching, but sessions should always belong to the current profile.

## Goals / Non-Goals

**Goals:**
- Directory is the sole identifier for auto-profiles (tmux session name is irrelevant)
- Safe multi-instance cleanup using instance tracking
- Profile field removed from new session dialog
- Reliable profile deletion

**Non-Goals:**
- Changing how explicit `-p` / `AGENT_OF_EMPIRES_PROFILE` profiles work
- Migrating existing auto-profiles named after tmux sessions
- Adding a profile lock/mutex system for concurrent writes

## Decisions

### D1: Directory-only profile resolution

Remove the tmux-session-name branch from `resolve_profile()`. All auto-profiles derive from the canonical working directory: `auto-<sanitized_dirname>-<hash>`.

**Rationale**: The directory is the natural boundary for a project. tmux session names are arbitrary and users often rename them. Two terminals in the same project directory should see the same sessions.

**Alternative considered**: Keeping tmux as primary but adding a directory fallback merge. Rejected because it creates ambiguity about which profile "wins" and complicates the mental model.

### D2: PID-file-based instance tracking

Use a simple PID tracking scheme in the profile directory:

- On startup: write `<profile_dir>/.instances/<pid>` file
- On exit: remove the PID file
- Before cleanup: check if any other PID files exist AND their processes are still alive (kill -0)
- Only delete the profile if: (a) no sessions remain, (b) no other live instances

**Rationale**: PID files are simple, don't require external dependencies, and stale PIDs from crashed processes can be detected with `kill(pid, 0)`. Lock files or Unix sockets add complexity without meaningful benefit for this use case.

**Alternative considered**: Unix advisory locks (`flock`). More robust but harder to inspect/debug, and overkill for tracking instance count.

### D3: Remove profile field from new session dialog

The `NewSessionDialog` will no longer include `profile`, `available_profiles`, or `profile_index` fields. The profile is inherited from the parent `HomeView`'s current profile. `NewSessionData.profile` is set to the current profile unconditionally.

**Rationale**: Switching profiles during session creation is confusing. If a user wants a session in a different profile, they should switch profiles first, then create.

### D4: Fix profile deletion reliability

Ensure `delete_profile()` errors propagate to the caller. In the TUI profile picker, show error messages when deletion fails instead of silently continuing. Check for read-only directories or permission issues.

**Rationale**: Silent failures make profiles appear undeletable. Surfacing errors lets users understand and fix the issue.

## Risks / Trade-offs

- **[Breaking change]** Users who relied on tmux-session-based profiles will get new directory-based profiles. Their old `auto-<tmux-name>` profiles will remain but won't be auto-selected.
  -> Mitigation: Old profiles are not deleted. Users can manually copy sessions or rename profiles.

- **[Stale PID files]** If aoe crashes without cleanup, PID files remain.
  -> Mitigation: On startup, scan `.instances/` and remove files whose PIDs are no longer alive.

- **[Race condition]** Between checking instances and deleting the profile, another instance could start.
  -> Mitigation: Acceptable risk. The window is tiny and the consequence is just a missing auto-profile that gets recreated on next launch.
