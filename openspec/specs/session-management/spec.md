# Capability Spec: Session Management

**Capability**: `session-management`
**Created**: 2026-03-06
**Status**: Stable

## Purpose

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

## Requirements

### Requirement: Session attach configures tmux key bindings
When attaching to any AoE-managed tmux session, the attach operation SHALL configure tmux key
bindings for navigation: `Ctrl+b d` for detach/return (nested mode only), `Ctrl+b n/p` for
session cycling (all modes), and `Ctrl+b 1-9` for number jump via key tables (all modes). The
`attach()` method accepts a `profile` parameter to scope session cycling and number jump to the
current profile.

#### Scenario: Agent session attach sets bindings
- **WHEN** `Session::attach(profile)` is called
- **THEN** session cycling bindings (`n`/`p`) are configured scoped to the given profile
- **AND** number jump bindings (`1`-`9` with `aoe-1` through `aoe-9` key tables) are configured
- **AND** if TMUX env var is set and `switch-client` succeeds, the `d` binding is also configured

#### Scenario: Number jump bindings cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** keys `1`-`9` SHALL be unbound from the prefix table
- **AND** all `aoe-N` key table bindings SHALL be unbound

### Requirement: Agent pane ID is stored on session creation
When an AoE-managed tmux session is created, the system SHALL capture the initial pane's `#{pane_id}` and store it as the session-level tmux option `@aoe_agent_pane`.

#### Scenario: Pane ID stored atomically with session creation
- **WHEN** `Session::create_with_size()` creates a new tmux session
- **THEN** the session SHALL have a `@aoe_agent_pane` option set to the pane ID of the initial pane (e.g. `%42`)
- **AND** the option SHALL be set atomically in the same tmux command chain as session creation

### Requirement: Pane health checks target the stored agent pane
All pane health check functions (`is_pane_dead`, `is_pane_running_shell`, `get_pane_pid`) SHALL target the stored agent pane ID rather than the session's currently active pane. If no stored pane ID exists, the functions SHALL fall back to targeting the session name.

Additionally, the `Session` struct SHALL expose a `pane_count()` method that returns the number of panes in the session, and a `respawn_agent_pane(command)` method that respawns only the agent pane.

#### Scenario: Health check with user-created split panes
- **WHEN** a session has user-created split panes via tmux shortcuts
- **AND** the active pane is a user-created shell (not the agent pane)
- **AND** `is_pane_dead()` or `is_pane_running_shell()` is called
- **THEN** the check SHALL target the original agent pane, not the active pane
- **AND** the result SHALL reflect the agent pane's state

#### Scenario: Session survives detach from user-created pane
- **WHEN** a user creates a split pane inside an AoE-managed session
- **AND** the user detaches from the user-created pane (Ctrl+b d)
- **AND** the user re-enters the session from the AoE TUI
- **THEN** the session SHALL NOT be killed and recreated
- **AND** all user-created split panes SHALL be preserved

#### Scenario: Fallback for sessions without stored pane ID
- **WHEN** `is_pane_dead()` or `is_pane_running_shell()` is called on a session
- **AND** the session does not have a `@aoe_agent_pane` option (e.g. created before this change)
- **THEN** the functions SHALL fall back to the previous behavior of targeting the session name

#### Scenario: Agent pane health is correctly detected through splits
- **WHEN** the agent process exits or crashes in the original pane
- **AND** user-created split panes are still running shells
- **THEN** `is_pane_dead()` SHALL return true (or `is_pane_running_shell()` SHALL return true)
- **AND** AoE SHALL correctly detect the agent has exited

#### Scenario: Attach-time recovery prefers respawn for multi-pane sessions
- **WHEN** the agent pane is dead during attach
- **AND** the session has more than one pane
- **THEN** the system SHALL use `respawn-pane` instead of `kill-session`
- **AND** the session layout and user-created panes SHALL be preserved

#### Scenario: Attach-time recovery uses kill-session for single-pane sessions
- **WHEN** the agent pane is dead during attach
- **AND** the session has exactly one pane
- **THEN** the system SHALL use the existing `kill-session` + recreate flow

### Requirement: Session creation sets group default directory for new groups
When creating a session that causes a new group to be created, the system SHALL set the group's `default_directory` to the session's `project_path`. This applies only when the group did not exist before the session was created.

The session creation flow SHALL accept an optional right pane tool parameter. When provided, the system SHALL split the tmux session window horizontally after creation and launch the specified tool in the right pane, while maintaining correct `@aoe_agent_pane` tracking.

#### Scenario: Creating session with new group sets default directory
- **WHEN** `create_session()` is called with a `group_path` that does not exist in the group tree
- **AND** the session's `project_path` is `/home/user/project`
- **THEN** after the group is created, its `default_directory` SHALL be `/home/user/project`

#### Scenario: Creating session in existing group does not change default directory
- **WHEN** `create_session()` is called with a `group_path` that already exists in the group tree
- **THEN** the group's `default_directory` SHALL NOT be modified

#### Scenario: Creating session with right pane tool splits window
- **WHEN** `create_session()` is called with a `right_pane_tool` value that is not "none"
- **THEN** after the tmux session is created, the system SHALL split the window horizontally
- **AND** the right pane SHALL launch the specified tool
- **AND** `@aoe_agent_pane` SHALL still reference the original left pane

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
