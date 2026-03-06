# Capability Spec: Lifecycle Hooks

**Capability**: `hooks`
**Created**: 2026-03-06
**Status**: Stable

## Overview

Hooks are shell commands that run automatically at specific points in the session lifecycle.
They allow users to automate setup tasks (installing dependencies, setting env vars, running
migrations) without manual intervention. Hooks are configurable at global, profile, and repo
levels with a clear override hierarchy.

## Hook Types

### `on_create`

Runs **once** when a session is first created.

- **Failure semantics**: Fatal. Hook failure aborts session creation. Cleanup occurs.
- **Use cases**: Install dependencies (`npm install`), set up environment, run database migrations.
- **Execution**: Synchronous, blocking session creation.

### `on_launch`

Runs **every time** a session starts (including after restart).

- **Failure semantics**: Non-fatal. Failures are logged as warnings; the session starts normally.
- **Use cases**: Start background services, set environment variables, warm caches.
- **Execution**: Asynchronous for the initial creation path (runs in background poller).
  Synchronous on subsequent launches.

## Override Hierarchy

```
Repo (.aoe/config.toml)     ← most specific, wins entirely
    ↑ if not set
Profile config
    ↑ if not set
Global config               ← least specific, fallback
```

Override is **per-field**: `on_create` and `on_launch` are resolved independently.
If a repo defines `on_create` but not `on_launch`, `on_launch` falls back to the profile
or global value.

## Trust Model

| Hook Source | Trust Requirement |
|-------------|------------------|
| Global config | Implicitly trusted (user-authored in app's own config dir) |
| Profile config | Implicitly trusted |
| Repo config (`.aoe/config.toml`) | Requires explicit trust approval via TUI dialog |

Repo hooks show a trust dialog the first time they are encountered. The user must approve before
hooks execute. Trust state is stored per-project (keyed by SHA-256 hash of the config file).

## Execution Environment

Hooks ALWAYS follow the session's sandbox setting:

| Session Type | Hook Execution Location |
|-------------|------------------------|
| Non-sandboxed | Host machine, in `project_path` directory |
| Sandboxed | Inside the container, in the container working directory |

There is no per-hook override for execution location.

## `HooksConfig` Structure

```toml
[hooks]
on_create = ["npm install", "cp .env.example .env"]
on_launch = ["./scripts/start-services.sh"]
```

## Duplicate Execution Prevention

When a session is first created, `on_launch` hooks run in the background during creation.
When the user then attaches to that session, the system skips `on_launch` hooks to prevent
double execution. Subsequent re-attaches/restarts run hooks normally.

## Functional Requirements

- **FR-001**: `on_create` hooks MUST run exactly once, during session creation.
- **FR-002**: `on_launch` hooks MUST run on every session start except immediately after creation (to prevent duplicate execution).
- **FR-003**: `on_create` hook failures MUST abort session creation and prevent the tmux session from being created.
- **FR-004**: `on_launch` hook failures MUST be non-fatal and logged as warnings.
- **FR-005**: Global and profile hooks MUST NOT require trust approval.
- **FR-006**: Repo hooks MUST require user trust approval before executing.
- **FR-007**: Hook resolution MUST be per-field: `on_create` and `on_launch` resolved independently with the override hierarchy.
- **FR-008**: Sandboxed sessions MUST execute hooks inside the container, not on the host.
- **FR-009**: Hook commands MUST be passed to the shell as-is (no escaping by AoE); commands are run via the system shell.
- **FR-010**: An empty hook list (`[]`) at any level means "no hooks" for that level (does not fall through to parent).

## Success Criteria

- **SC-001**: `on_create` hooks execute exactly once per session lifetime.
- **SC-002**: `on_launch` hooks execute on every restart but not twice on creation.
- **SC-003**: A failing `on_create` hook prevents the session from appearing in the session list.
- **SC-004**: Repo hooks show a trust dialog on first use and remember the decision.
- **SC-005**: Hooks inside sandboxed sessions have access to the container filesystem, not the host.
