## Context

Currently, `main.rs` calls `ensure_profile_exists(&profile)` on every startup (line 42). This eagerly creates a profile directory under `~/.agent-of-empires/profiles/<name>/` before any session data exists. Combined with auto-profile resolution (which generates a unique name per working directory like `auto-myproject-a1b2`), this pollutes the profiles directory with empty folders.

The codebase already has lazy directory creation in `get_profile_dir()` (which creates directories when accessed) and `register_instance()` (which creates the `.instances` subdirectory). The eager creation is redundant.

## Goals / Non-Goals

**Goals:**
- Remove eager profile directory creation on startup
- Profile directories are only created when actually needed (session save, instance registration, config write)
- Clean up `ensure_profile_exists` which duplicates `get_profile_dir`'s behavior

**Non-Goals:**
- Changing auto-profile naming or resolution logic
- Changing how `get_profile_dir` works (it already creates lazily)
- Changing profile CLI commands behavior

## Decisions

**Decision 1: Remove `ensure_profile_exists` call from `main.rs`**

The call at `main.rs:42` is the only place that eagerly creates profile directories. Removing it means directories are created on first actual use via `get_profile_dir()`, `register_instance()`, or `Storage::new()`.

Alternative considered: Making `ensure_profile_exists` conditional (only create if sessions exist). Rejected because existing lazy creation in `get_profile_dir` already handles this -- the function is simply unnecessary.

**Decision 2: Remove `ensure_profile_exists` function entirely**

`get_profile_dir()` already creates the directory if it doesn't exist. `ensure_profile_exists` is a duplicate. Callers in tests can use `get_profile_dir()` or `create_profile()` instead.

**Decision 3: Keep `check_migration_hint` but adjust ordering**

`check_migration_hint` reads profile data but doesn't create directories (it uses `Storage::new` which goes through `get_profile_dir`). It can remain in the startup sequence without `ensure_profile_exists` preceding it.

## Risks / Trade-offs

- [Risk] Code that assumes the profile directory exists before calling non-directory-creating functions. -> Mitigation: `get_profile_dir()` already creates lazily, and `Storage::new` uses it. All write paths go through directory-creating functions.
- [Risk] Tests that rely on `ensure_profile_exists`. -> Mitigation: Update tests to use `get_profile_dir()` or `create_profile()` instead.
