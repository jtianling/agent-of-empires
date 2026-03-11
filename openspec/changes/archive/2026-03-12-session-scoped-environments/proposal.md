## Why

Currently `aoe` is a globally unique application -- all invocations share the same "default" profile regardless of where or how they are launched. This means running `aoe` in `~/project-a` and `~/project-b` shows the same session list, and opening `aoe` in two different tmux sessions sees the same environment. Users working on multiple projects simultaneously need automatic isolation without manually switching profiles.

## What Changes

- **BREAKING**: `aoe` will automatically derive an "environment key" from the current context (tmux session name + working directory) to determine which environment to load
- When launched inside a tmux session, the tmux session name becomes part of the environment key, creating an isolated environment per tmux session
- When launched from a different directory (outside tmux), the directory becomes part of the environment key, creating per-directory isolation
- When launched from the same directory and not inside tmux, `aoe` reuses the same environment (preserving current behavior for the simple case)
- The environment key maps to an auto-created profile, keeping the existing profile infrastructure intact
- Manual `--profile` flag continues to work and takes precedence over auto-detection

## Capabilities

### New Capabilities
- `environment-scoping`: Automatic environment isolation based on tmux session and working directory context. Derives an environment key, maps it to a profile, and auto-creates profiles as needed.

### Modified Capabilities
- `profiles`: Profile selection changes from static default to dynamic auto-resolution. Auto-created profiles are managed transparently. The `default` profile becomes a fallback rather than the universal default.
- `cli`: The profile resolution logic changes -- when no `--profile` is specified, the environment key determines the profile instead of always using `default`.

## Impact

- `src/session/mod.rs`: New environment key derivation logic
- `src/tmux/`: Detection of parent tmux session (whether `aoe` was launched inside an existing tmux session)
- `src/cli/definition.rs`: Profile resolution fallback chain changes
- `src/main.rs`: Startup flow incorporates environment detection before profile selection
- `src/tui/app.rs`: Must use resolved profile from environment context
- Existing users' sessions remain in `default` profile and are accessible when the environment key resolves to `default` (same directory, no tmux)
