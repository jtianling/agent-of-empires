# Capability Spec: Session Management

**Capability**: `session-management`
**Created**: 2026-03-06
**Status**: Stable

## Overview

A Session (internally `Instance`) is the core unit of AoE. Each session pairs a running AI agent
process with a tmux session and persists its metadata to disk. Users can create, start, stop,
restart, and delete sessions via both the TUI and CLI.

## Key Entities

### Instance

The primary data structure representing a session.

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` (16-char hex) | Unique session identifier (UUID v4, truncated) |
| `title` | `String` | Human-readable display name |
| `project_path` | `String` | Absolute path to the project directory |
| `group_path` | `String` | Slash-delimited group hierarchy (e.g. `work/clients`) |
| `tool` | `String` | Agent name (`claude`, `opencode`, etc.) |
| `command` | `String` | Custom command override (empty = use agent binary) |
| `extra_args` | `String` | Extra CLI arguments appended after the command |
| `yolo_mode` | `bool` | Whether to enable auto-approve / skip-permissions mode |
| `status` | `Status` | Current agent state (see Status enum) |
| `created_at` | `DateTime<Utc>` | Creation timestamp |
| `worktree_info` | `Option<WorktreeInfo>` | Git worktree details if applicable |
| `sandbox_info` | `Option<SandboxInfo>` | Container sandbox details if applicable |
| `terminal_info` | `Option<TerminalInfo>` | Paired terminal session details |

### Status Enum

```
Running   -- Agent is actively processing
Waiting   -- Agent is waiting for user input
Idle      -- Agent is ready but not doing anything
Starting  -- Grace period after session launch (3 seconds)
Error     -- tmux session not found or pane dead
Stopped   -- Explicitly stopped by the user
Deleting  -- Session is being removed
Unknown   -- Agent is running but status is unrecognizable (e.g. custom command)
```

### WorktreeInfo

Tracks git worktree metadata for sessions created on separate branches.

| Field | Type | Description |
|-------|------|-------------|
| `branch` | `String` | Git branch name |
| `main_repo_path` | `String` | Path to the main (bare) repository |
| `managed_by_aoe` | `bool` | Whether AoE created this worktree |
| `cleanup_on_delete` | `bool` | Whether to remove the worktree on session deletion (default: true) |

### SandboxInfo

Tracks container sandbox state for a session.

| Field | Type | Description |
|-------|------|-------------|
| `enabled` | `bool` | Whether sandboxing is active |
| `container_id` | `Option<String>` | Runtime container ID |
| `image` | `String` | Container image reference |
| `container_name` | `String` | Named container handle |
| `extra_env` | `Option<Vec<String>>` | Session-specific env vars (`KEY` or `KEY=VALUE`) |
| `custom_instruction` | `Option<String>` | Instruction text injected into agent's system prompt |

### TerminalInfo

Tracks the optional paired shell terminal session.

| Field | Type | Description |
|-------|------|-------------|
| `created` | `bool` | Whether the terminal tmux session exists |
| `created_at` | `Option<DateTime<Utc>>` | When the terminal was created |

## Session Lifecycle

```
     [User creates session]
             │
             ▼
        [Instance::new()]
        Status: Idle
             │
             ▼
        [Instance::start()]
        ┌─────────────────────────────┐
        │ 1. Run on_create hooks      │
        │ 2. Ensure container (sandbox│
        │    sessions only)           │
        │ 3. Build launch command     │
        │ 4. Create tmux session      │
        │ 5. Apply tmux options       │
        └─────────────────────────────┘
        Status: Starting (3s grace)
             │
             ▼
        Status: Running / Waiting / Idle
        (detected from pane content)
             │
        [User stops/restarts/deletes]
             │
        ┌────┴────────────┐
        │                 │
        ▼                 ▼
      stop()          restart()
  kill tmux + stop  kill + start again
   container
```

## Command Construction

The launch command is built in priority order:

1. If `command` field is set and differs from agent binary: use it as-is
2. Otherwise: use the agent's registered `binary`
3. Append `extra_args` if set
4. If `yolo_mode`: append agent's YOLO flag/envvar
5. If sandboxed with `custom_instruction` and agent supports `instruction_flag`: append flag
6. Wrap entire command with `bash -c 'stty susp undef; exec <cmd>'` to disable Ctrl-Z

For sandboxed sessions, the command is wrapped in the container runtime's `exec` invocation.

## Functional Requirements

- **FR-001**: Each session MUST have a unique 16-character hex ID derived from UUID v4.
- **FR-002**: Sessions MUST persist to disk (JSON) and survive application restarts.
- **FR-003**: Session status MUST be updated from live tmux pane content during polling.
- **FR-004**: Starting an already-running session MUST be idempotent (no-op if tmux session exists).
- **FR-005**: Restarting a session MUST kill the existing tmux session, wait 100ms, then start fresh.
- **FR-006**: Deleting a session MUST kill the tmux session, optionally remove the worktree and git branch, stop and remove the container (if sandboxed), and remove the session record from storage.
- **FR-007**: The launch command MUST be wrapped to disable Ctrl-Z suspension (SIGTSTP).
- **FR-008**: Sessions without a recognized agent binary MUST fall back to `bash`.
- **FR-009**: Status MUST remain `Error` for 30 seconds after detection before re-checking (to avoid thrashing).
- **FR-010**: Sessions with custom commands that return `Idle` status MUST show `Unknown` (not Idle) since idle detection is agent-specific.

## Success Criteria

- **SC-001**: Sessions created via TUI or CLI are indistinguishable in behavior.
- **SC-002**: Session state is fully recoverable after AoE is closed and reopened.
- **SC-003**: Status polling correctly reflects Running/Waiting/Idle states for all supported agents.
- **SC-004**: Ctrl-Z does not suspend agent processes inside tmux sessions.
