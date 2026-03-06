# Capability Spec: Container Sandbox

**Capability**: `sandbox`
**Created**: 2026-03-06
**Status**: Stable

## Overview

The sandbox subsystem isolates AI agent sessions inside containers (Docker or Apple Container).
Each sandboxed session gets its own container with the project directory bind-mounted. Containers
are stopped (not removed) when the session stops, and restarted on re-attach. This allows agents
to install packages and modify the filesystem freely without affecting the host.

## Key Entities

### SandboxConfig (global/profile setting)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled_by_default` | `bool` | `false` | Auto-sandbox all new sessions |
| `default_image` | `String` | `ghcr.io/njbrake/aoe-sandbox:latest` | Container image |
| `extra_volumes` | `Vec<String>` | `[]` | Additional bind mounts (`host:container`) |
| `environment` | `Vec<String>` | `["TERM", "COLORTERM", ...]` | Env vars to pass through |
| `auto_cleanup` | `bool` | `true` | Remove container on session delete |
| `cpu_limit` | `Option<String>` | None | Docker CPU quota (e.g. `"2"`) |
| `memory_limit` | `Option<String>` | None | Docker memory limit (e.g. `"4g"`) |
| `port_mappings` | `Vec<String>` | `[]` | Port forwards (`host:container`) |
| `default_terminal_mode` | `DefaultTerminalMode` | `Host` | Terminal toggles to host or container shell |
| `volume_ignores` | `Vec<String>` | `[]` | Subdirs excluded from bind mount via anonymous volumes |
| `mount_ssh` | `bool` | `false` | Mount `~/.ssh` into container |
| `custom_instruction` | `Option<String>` | None | System prompt injected at agent launch |
| `container_runtime` | `ContainerRuntimeName` | `Docker` | `docker` or `apple_container` |

### Container Runtimes

```
Docker          -- Standard Docker daemon (Linux & macOS)
AppleContainer  -- Apple's native virtualization framework (macOS only)
```

Both runtimes implement the `ContainerRuntimeInterface` trait, providing:
- `ensure_image(image)` -- pull image if not present
- `create(config)` -- create container with given config
- `start()` / `stop()` -- lifecycle control
- `exec_command(opts, cmd)` -- build exec command string
- `is_running()` / `exists()` -- state queries

### DefaultTerminalMode

```
Host       -- The 't' key opens a host shell in the project directory
Container  -- The 't' key opens a shell inside the running container
```

## Container Lifecycle

```
Session Create (sandboxed):
  1. ensure_image() -- pull if not cached (always pulls for latest)
  2. build_container_config() -- volumes, env, limits, port mappings
  3. container.create(config) -- stores container_id in SandboxInfo
  4. (container starts automatically via exec)

Session Start (agent launch):
  1. is_running()? -- if yes, refresh agent configs and proceed
  2. exists()?     -- if yes, start existing container
  3. else          -- create fresh container
  4. exec_command() -- wrap agent command in `docker exec` or equivalent

Session Stop:
  1. kill tmux session
  2. container.stop() -- container paused, not removed (preserves state)

Session Delete:
  1. stop()
  2. if auto_cleanup: container.remove()
  3. remove session record
```

## Volume Mounting

The project directory is bind-mounted into `/workspace` (or the computed container workdir).

For bare-repo worktrees, the volume path is computed relative to the worktree structure to
ensure the agent sees the correct working directory.

`volume_ignores` paths (e.g. `["target", "node_modules", ".venv"]`) are excluded by creating
anonymous Docker volumes at those paths, preventing the host directories from being visible
inside the container.

## Auth Volume Sharing

Agent auth credentials (API keys, config) are shared between the host and container via
named volumes mapped to agent-specific config directories:
- `claude`: `/root/.claude`
- `cursor`: `/root/.cursor`

## Container Terminal

Sandboxed sessions support two terminal types (toggled with `t` key):
- **Host terminal**: a plain bash shell in the project directory on the host
- **Container terminal**: `docker exec -it <container> /bin/bash` inside the running container

## Functional Requirements

- **FR-001**: Container MUST be created before the agent session starts.
- **FR-002**: Stopping a session MUST stop (not remove) the container to preserve state.
- **FR-003**: Restarting a session MUST reuse the existing container if it still exists.
- **FR-004**: Image pulls MUST happen on every container creation to ensure latest image.
- **FR-005**: `volume_ignores` paths MUST be mounted as anonymous volumes (not excluded from the bind mount -- the bind mount covers the parent, anonymous volumes shadow the subdirs).
- **FR-006**: Custom instructions MUST only be applied to sandboxed sessions with supported agents.
- **FR-007**: The container runtime MUST be selectable per profile or global config.
- **FR-008**: Auth volumes (claude, cursor config dirs) MUST be automatically shared from host into container.
- **FR-009**: `on_create` and `on_launch` hooks MUST run inside the container for sandboxed sessions.
- **FR-010**: The `container_terminal` mode MUST exec into the running container, not start a new one.

## Success Criteria

- **SC-001**: A sandboxed Claude session can install npm packages without affecting the host.
- **SC-002**: Stopping and re-attaching a sandboxed session preserves container filesystem state.
- **SC-003**: Port mappings allow web apps running in the container to be accessed on the host.
- **SC-004**: `volume_ignores` prevents large build artifact directories from being bind-mounted into the container.
- **SC-005**: Auth credentials are available inside the container without manual copying.
