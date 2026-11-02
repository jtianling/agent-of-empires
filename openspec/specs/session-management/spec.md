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

### Requirement: Codex 原生会话名与协作身份一致

启用 xats 的 Codex pane 在启动、resume 和 restart 时 SHALL 将 Codex 原生会话名设为该 pane 的 `name@team`, 使用与 `/rename` 相同的原生命名接口.  managed pane SHALL 使用明确配置的协作身份; 普通 TUI pane SHALL 仅使用当前 pane 和启动代次验证通过的 thread 绑定及 launcher key 对应的已注册身份.  命名 SHALL 不改变 thread ID, 不添加模型回合, 不改写已有对话内容或 AoE 的 session 标题.

#### Scenario: 多 pane 恢复后命名

- **WHEN** 启用 xats 的 Codex 主 pane 和附加 pane 完成 resume 或 restart
- **THEN** 每个原生 Codex 会话使用自己的 `name@team`
- **AND** 邻 pane 的会话名和历史不因该 pane 命名而改变

#### Scenario: 普通 pane 等待注册

- **WHEN** 普通 TUI pane 启动时尚未取得当前启动的注册身份
- **THEN** 命名助手最多等待 120 秒, 不使用标题或目录猜测名称
- **AND** 命名前重新核对启动代次、thread 和身份, 对旧代、不匹配、歧义或接口错误保留原名并记录具体原因

#### Scenario: 用户随后手动改名

- **WHEN** 本次启动的自动命名已经成功, 用户随后执行 `/rename`
- **THEN** AoE 不再持续覆盖本次启动中的手动名称

#### Scenario: 没有协作身份

- **WHEN** pane 未启用 xats 或未注册协作身份
- **THEN** AoE 不猜测或创建 `name@team` 名称

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

### Requirement: 宿主机 agent 启动支持 NVM 后备路径

AoE SHALL 在宿主机 agent 的首次启动, resume 和 restart 中优先使用 login shell 当前 PATH 的可执行文件.  当目标 agent 不在该 PATH 中时, SHALL 使用 AoE 进程 PATH 中的有效入口作为后备; 如果进程 PATH 也找不到, SHALL 在 `NVM_DIR/versions/node` 中按 Node 版本从高到低查找, 未设置 `NVM_DIR` 时使用 `~/.nvm/versions/node`.  安装检测 SHALL 使用相同的后备查找规则.  后备目录 SHALL 追加到 PATH 末尾, 保留当前 Node 版本的优先级.  此机制 SHALL 保留自动生成的 resume 参数和 xats 启动流程, 不应用于容器或完整自定义命令覆盖.

#### Scenario: 切换 Node 版本后恢复 Codex

- **WHEN** 当前 Node 版本的 PATH 中没有 `codex`, 但另一个 NVM 版本中存在可执行的 `codex`
- **THEN** 安装检测 SHALL 将 Codex 识别为可用
- **AND** resume 和 restart SHALL 能使用后备入口启动, 并保留原有恢复参数
- **AND** 当前 PATH 中的 `node` SHALL 继续优先于后备目录中的 `node`

#### Scenario: 当前 PATH 已提供 agent

- **WHEN** login shell 的当前 PATH 中存在目标 agent 的可执行文件
- **THEN** 启动 SHALL 使用当前 PATH 的入口, 不追加后备目录

#### Scenario: 多个 NVM 版本提供同一 agent

- **WHEN** AoE 进程 PATH 中没有目标 agent, 且多个有效 NVM 版本提供该 agent
- **THEN** AoE SHALL 选择版本号最高且实际存在可执行入口的 Node 安装目录作为后备

### Requirement: Codex 宿主机恢复采用当前 provider

AoE SHALL 在宿主机 Codex 恢复时使用目标 pane 工作目录下当前生效的 provider,
而非历史线程保存的 provider.  远程恢复 SHALL 通过同一 app-server 的
`config/read` 解析 provider, 显式传入 `thread/resume` 并验证返回的 provider 和
线程 UUID.  本地恢复 SHALL 确保 daemon 已启动, 通过其 stdio proxy 读取实际
配置与历史, 并向原 Codex 命令追加 provider 覆盖参数, 不重启已运行的 daemon.
未显式配置 provider 时 SHALL 使用 Codex 默认
`openai`.  此规则 SHALL 同时覆盖 YOLO 和非 YOLO 恢复, 不改变非 YOLO 权限,
不应用于容器或完整自定义命令覆盖.

此规则 SHALL 覆盖普通本地, 原有 xats 和 managed xats 的主 pane 与附加 pane,
包括 `r` 和 `R` 触发的恢复.  远程恢复 SHALL 在目标旧 TUI 退出后进行,
避免旧订阅阻止 provider 切换.  managed xats SHALL 不为退订而先恢复旧 provider,
并 SHALL 将本次选定的 provider 写入生成的客户端 profile, 供后续远程 TUI 使用.

当历史 provider 不同且 Codex 要求历史归属一致时, AoE SHALL 仅在同一服务确认
原 thread 为 `notLoaded`, 并核对本机 Codex home、原 UUID、规范化目录、rollout
和受支持数据库中的原记录后迁移 provider.  AoE SHALL 先落盘私有备份, 再以
事务和旧值校验更新准确 thread 的单个 provider 字段, 保留 UUID、rollout 和
其他 thread.  迁移 SHALL 同时覆盖 Resume 与保留上下文的 Restart.

AoE SHALL 对同一 thread 的 provider 迁移加锁.  非本机服务、仍加载的 thread、
归档历史、目录或 rollout 不匹配及未知数据库版本 SHALL 不触发本地数据修改.
服务加载状态 SHALL 不被宣称为跨 app-server 的全机器排他保证.

#### Scenario: 历史 provider 已被移除

- **WHEN** 历史线程保存的 provider 已从配置移除, 当前配置提供另一个有效 provider
- **THEN** 恢复 SHALL 使用当前 provider, 保留原线程 UUID 和历史
- **AND** 项目目录的配置覆盖 SHALL 按 Codex 自身的解析规则生效

#### Scenario: 已加载但无订阅的远程线程切换 provider

- **WHEN** 目标旧 TUI 已退出, 线程空闲且没有其它客户端订阅
- **AND** 当前生效的 provider 与历史 provider 不同
- **THEN** AoE SHALL 仅在服务确认原 thread 已卸载后迁移历史 provider 并恢复原 UUID
- **AND** AoE SHALL 不先用已删除的历史 provider 加载线程

#### Scenario: 远程线程仍被其它客户端占用

- **WHEN** Codex 返回的 provider 与当前生效的 provider 不同
- **THEN** AoE SHALL 明确报错并保留 pane 与原线程记录, 不使用旧 provider 假报恢复成功
- **AND** AoE SHALL 不通过归档线程或重启共享服务绕过限制

#### Scenario: provider 迁移后恢复明确失败

- **WHEN** 历史 provider 已迁移, 但恢复请求失败
- **AND** 同一服务再次确认原 thread 为 `notLoaded`
- **THEN** AoE SHALL 按准确 UUID 和迁移后的 provider 做旧值校验, 回滚原 provider
- **AND** 回滚失败 SHALL 保留具体原因及备份路径

#### Scenario: provider 迁移后的恢复结果未知

- **WHEN** 恢复超时、断连或被取消, 且不能确认原 thread 仍未加载
- **THEN** AoE SHALL 保留当前 provider 与备份, 记录结果无法确认的具体原因
- **AND** AoE SHALL 不盲目回滚可能已在服务端加载成功的 thread

#### Scenario: 新建上下文的 Restart

- **WHEN** 用户选择不保留旧上下文的 Restart
- **THEN** AoE SHALL 直接使用当前生效的 provider 新建上下文, 不迁移旧历史记录

#### Scenario: 配置解析或恢复请求失败

- **WHEN** 配置解析或恢复 RPC 返回错误
- **THEN** AoE SHALL 保留具体错误原因
- **AND** 临时本地配置服务 SHALL 在成功, 失败和超时后退出

### Requirement: xats 绑定丢失后恢复已验证的 Codex 对话

实时绑定查询返回 `pending`, `not_found` 或 `stale` 时, AoE SHALL 允许 `r`/`R`
使用与当前 slot 的 generation, 身份, pane 和线程一致的持久化绑定证明.
当前进程的 launch ID SHALL 与已验证证明一致, 或进程树中的 Codex SHALL 都明确
执行 `resume <原线程 UUID>`.  `awaiting_launch` SHALL 拒绝已退役的 launch ID.
查询前后的 pane 进程和启动信息 SHALL 一致.  恢复 SHALL 保留原线程和历史,
并继续执行原有 provider 和权限准备.

#### Scenario: 共享服务重启后重试同一对话

- **WHEN** xats 重启后当前 Codex 启动的绑定查询返回 `pending`
- **AND** slot 保留有效证明, 当前启动明确 resume 同一线程
- **THEN** AoE SHALL 允许恢复该线程, 不要求旧 TUI 先重新完成 xats 注册

#### Scenario: 缺少证明或启动不匹配

- **WHEN** 持久化证明缺失, 已过期, 属于另一 pane, 或当前启动既不匹配证明也未明确 resume 原线程
- **THEN** AoE SHALL 拒绝恢复, 不按目录或时间猜测线程
- **AND** fresh, fork 和已退役的启动 SHALL 不得作为同线程 resume 的证据

#### Scenario: 实时查询报告冲突或失败

- **WHEN** 实时查询返回不同线程, 歧义绑定, 接口错误或协议错误
- **THEN** AoE SHALL 保留具体失败原因并拒绝恢复, 不使用持久化证明绕过错误

### Requirement: Session attach configures tmux key bindings
When attaching to any AoE-managed tmux session, the attach operation SHALL configure tmux key bindings for navigation: root-table `Ctrl+,` and `Ctrl+.` for session cycling, `Ctrl+b 1-9` for number jump via key tables, `Ctrl+b b` for back toggle, `Ctrl+b h/j/k/l` for pane navigation, and `Ctrl+;` for pane cycling. The `attach()` method accepts a `profile` parameter to scope session cycling and number jump to the current profile.

The binding setup SHALL be performed by the caller (TUI or CLI) before entering the raw-mode-disabled window, not inside `Session::attach()`. `Session::attach()` SHALL only execute `tmux attach-session`. The attach SHALL always use `attach-session` (never `switch-client`).

#### Scenario: Agent session attach sets bindings before raw mode change
- **WHEN** `App::attach_session()` is called from the TUI
- **THEN** `setup_session_cycle_bindings(profile)` SHALL be called while the TUI alternate screen is still visible
- **AND** `Session::attach()` SHALL only execute `tmux attach-session`
- **AND** no tmux binding commands SHALL execute after `LeaveAlternateScreen`

#### Scenario: Attach always uses attach-session regardless of environment
- **WHEN** `Session::attach(profile)` is called
- **AND** the `TMUX` env var may or may not be set
- **THEN** the system SHALL always use `tmux attach-session` (never `switch-client`)

#### Scenario: Number jump bindings cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** keys `1`-`9` SHALL be unbound from the prefix table
- **AND** all `aoe-N` key table bindings SHALL be unbound

#### Scenario: CLI attach still sets up bindings
- **WHEN** `aoe session attach` is called from the CLI
- **THEN** `setup_session_cycle_bindings(profile)` SHALL be called before `tmux attach-session`

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

### Requirement: Session Rename
When a session is renamed (title change), the system SHALL rename the underlying tmux session to match the new title. The rename operation MUST NOT interrupt running processes. The tmux session name MUST be generated from the NEW title and session ID using `Session::generate_name()`.

The system SHALL construct the tmux Session reference using the OLD title before mutating the instance, ensuring the `tmux rename-session` command targets the correct (existing) session.

This applies to both same-profile renames and cross-profile renames.

#### Scenario: Same-profile rename updates tmux session name
- **WHEN** a user renames a session from "OldTitle" to "NewTitle" within the same profile
- **THEN** the tmux session SHALL be renamed from `aoe_OldTitle_<id>` to `aoe_NewTitle_<id>`
- **AND** all processes in the session SHALL continue running uninterrupted
- **AND** the status poller SHALL find the session under the new name

#### Scenario: Cross-profile rename updates tmux session name
- **WHEN** a user renames a session and moves it to a different profile
- **THEN** the tmux session SHALL be renamed to reflect the new title
- **AND** all processes in the session SHALL continue running uninterrupted

#### Scenario: Rename when tmux session does not exist
- **WHEN** a user renames a session whose tmux session has already exited
- **THEN** the rename SHALL update only the stored instance title
- **AND** no tmux rename command SHALL be attempted

### Requirement: Shell left pane starts in project_path

When a session is created with the Shell tool as the left (main) pane, the shell process SHALL start with its working directory set to the session's `project_path`. The command SHALL include an explicit `cd` to `project_path` before launching the interactive shell, ensuring the directory is correct even if login shell profiles change the cwd.

#### Scenario: Shell session starts in specified directory
- **WHEN** a user creates a new session with tool set to "shell"
- **AND** the Path field is set to `/some/project/path`
- **THEN** the shell SHALL start with its working directory as `/some/project/path`
- **AND** the tmux pane command SHALL include `cd '/some/project/path' &&` before the `exec` of the shell binary

#### Scenario: Shell session with special characters in path
- **WHEN** a user creates a shell session with a path containing spaces or quotes
- **THEN** the path SHALL be properly shell-escaped in the `cd` command
- **AND** the shell SHALL start in the correct directory

#### Scenario: Consistency with right pane shell behavior
- **WHEN** a session is created with Shell on both the left pane and right pane
- **THEN** both panes SHALL use the same `cd {project_path} && ... exec {shell}` pattern
- **AND** both panes SHALL start in the same working directory

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
### Requirement: 人工选择原线程恢复缺失的绑定

系统 SHALL 提供仅限非托管 host xats Codex 的显式恢复入口,
要求完整 session ID、slot 和原 thread UUID, 不猜测历史归属.
系统 SHALL 验证当前 pane 启动、原线程 UUID、规范化目录和 notLoaded 状态,
拒绝与当前 argv、xats 或其他 slot 的已知线程冲突.
系统 SHALL 在验证前后核对启动与 xats 证据, 通过事务和 CAS 持久化恢复证明及审计记录.
该入口 SHALL 不重启进程或创建新对话, 用户随后执行普通 Resume.

存活的普通 host xats Codex 的 CLI restart SHALL 复用逐 pane Resume,
先验证主 pane、全部目标 pane 的归属和 Codex 恢复证明, 再保留原布局和原线程恢复.
缺少证明或 pane 归属冲突时 SHALL 拒绝, 不销毁 session 或静默 fresh.
单 pane 的恢复错误 SHALL 向调用者报告, 不以整个 restart 成功掩盖失败.

#### Scenario: 原绑定过期但历史仍在
- **WHEN** 用户明确指定缺失绑定的 slot 和可读的原线程
- **THEN** 系统保存准确绑定, 下一次 Resume 仍使用原线程

#### Scenario: 明确冲突或验证失败
- **WHEN** 原线程已加载、目录不匹配、其他 slot 已绑定该线程或控制面验证失败
- **THEN** 系统拒绝恢复, 不改动已有绑定或启动新对话
