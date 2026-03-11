## Why

Every time `aoe` runs, it eagerly creates a profile directory via `ensure_profile_exists()` in `main.rs`, even before any session exists. Combined with auto-profile resolution (which generates a unique profile name per working directory), this means running `aoe` from any new directory pollutes `~/.agent-of-empires/profiles/` with empty directories. This is unnecessary -- profile directories should only be created when actually needed (e.g., when a session is first saved).

## What Changes

- Remove the eager `ensure_profile_exists()` call from the startup sequence in `main.rs`
- Remove the `ensure_profile_exists` function (it duplicates `get_profile_dir`'s lazy creation)
- Rely on existing lazy directory creation in `get_profile_dir()` and `register_instance()` which already create directories on demand

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `profiles`: Remove eager profile directory creation on startup. Profile directories are created lazily when first accessed for writing (session save, instance registration, etc.) instead of unconditionally on every run.

## Impact

- `src/main.rs`: Remove `ensure_profile_exists` call
- `src/session/mod.rs`: Remove `ensure_profile_exists` function
- Tests referencing `ensure_profile_exists` need updating
- `check_migration_hint` may need adjustment since it currently runs after `ensure_profile_exists`
