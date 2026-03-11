## Context

Currently `aoe` uses a static profile system. The `--profile` flag (or `AGENT_OF_EMPIRES_PROFILE` env var) determines which workspace to load, defaulting to `"default"`. This means every invocation without an explicit profile shares the same session list, regardless of directory or tmux context.

Users running `aoe` in multiple projects or multiple tmux sessions see the same environment, leading to clutter and confusion. The profile system exists but requires manual switching, which breaks the "just run `aoe`" workflow.

## Goals / Non-Goals

**Goals:**
- Automatically isolate `aoe` environments based on tmux session and working directory
- Zero-config for new users -- `aoe` "just works" with sensible scoping
- Preserve explicit `--profile` as an override mechanism
- Maintain backwards compatibility for existing single-environment users

**Non-Goals:**
- Merging or syncing sessions across environments
- Per-directory config file auto-detection (`.aoe/` already handles repo config)
- Changing the internal profile storage format

## Decisions

### D1: Environment key derivation

The environment key is computed as:

```
if --profile is specified:
    use that profile directly (no auto-resolution)
else if inside a tmux session (TMUX env var set):
    key = hash(tmux_session_name)
else:
    key = hash(canonical_cwd)
```

**Rationale**: Inside tmux, the session name is the natural isolation boundary -- users organize work by tmux sessions. Outside tmux, the working directory is the only distinguishing context. This matches user mental models: "I'm in my project-a tmux session" or "I'm in the project-a directory".

**Alternative considered**: Combining both tmux session AND directory. Rejected because within a tmux session, users may `cd` around and still expect the same `aoe` environment. The tmux session is the stronger signal.

### D2: Environment key format

The key is a deterministic, human-readable profile name:

- For tmux: `auto-<sanitized_tmux_session_name>` (e.g., `auto-project-a`)
- For directory: `auto-<sanitized_dir_name>-<short_hash>` (e.g., `auto-project-a-3f8a`)

The short hash (first 4 chars of hex SHA-256 of canonical path) prevents collisions when different paths have the same directory name. The `auto-` prefix distinguishes auto-created profiles from user-created ones.

**Rationale**: Human-readable names make `aoe profile list` output understandable. The prefix prevents naming conflicts with manually created profiles.

**Alternative considered**: Using raw hashes as profile names. Rejected for poor UX in profile listings and debugging.

### D3: Auto-profile creation

When the resolved environment key maps to a profile that doesn't exist, `aoe` creates it automatically (empty sessions.json and groups.json). No user prompt.

**Rationale**: The whole point is zero-config. Prompting defeats the purpose.

### D4: Profile resolution function

A new `resolve_profile()` function in `src/session/mod.rs` encapsulates the logic:

```rust
pub fn resolve_profile(explicit: Option<String>) -> String {
    if let Some(p) = explicit {
        return p;
    }
    if let Ok(tmux) = std::env::var("TMUX") {
        // Parse tmux session name from TMUX env var
        // or use tmux display-message to get session name
        return format!("auto-{}", sanitize(session_name));
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let dir_name = cwd.file_name().unwrap_or_default();
    let hash = short_hash(&cwd);
    format!("auto-{}-{}", sanitize(dir_name), hash)
}
```

This is called in `main.rs` replacing the current `cli.profile.unwrap_or_default()`.

### D5: Getting tmux session name

When `TMUX` env var is set, we get the session name via `tmux display-message -p '#S'`. The `TMUX` var itself contains the socket path, not the session name, so we need the tmux command.

**Alternative considered**: Parsing `TMUX` env var directly. Rejected because the format is `socket_path,pid,session_index` which doesn't contain the session name.

### D6: Default profile backward compatibility

Existing users who always run `aoe` from the same directory outside tmux will get a new auto-generated profile instead of `default`. To handle migration:

- On first run with the new logic, if the `default` profile has sessions and the auto-resolved profile is empty, display a one-time hint: "Sessions from 'default' profile can be accessed with `aoe -p default`"
- No automatic migration of sessions between profiles

**Rationale**: Automatic migration is risky and complex. A simple hint is sufficient -- power users already understand profiles.

## Risks / Trade-offs

- **[Directory renames break continuity]** -> If a user renames their project directory, the auto-profile changes and they get a fresh environment. Mitigation: users can use `--profile` to pin a specific profile name, or the tmux session name provides stability.

- **[Profile proliferation]** -> Auto-creation may accumulate many profiles over time. Mitigation: future cleanup command (`aoe profile prune`) can remove empty auto-profiles. Out of scope for this change.

- **[tmux session name changes]** -> Renaming a tmux session changes the environment key. Mitigation: tmux sessions are typically long-lived and rarely renamed. Same as directory rename -- `--profile` overrides.

- **[TMUX env var inheritance]** -> Child processes inherit `TMUX` env var even if launched outside a tmux session context. Mitigation: We already handle this case in the codebase (checking if we're actually attached to the tmux server).
