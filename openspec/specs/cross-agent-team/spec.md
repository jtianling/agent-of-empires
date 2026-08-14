# cross-agent-team Specification

## Purpose
TBD - created by archiving change cross-agent-team-launch. Update Purpose after archive.
## Requirements
### Requirement: Cross Agent Team launch option in New Session

New Session SHALL 为 primary pane 和已选择的 secondary pane 分别展示 Cross Agent Team checkbox。  checkbox SHALL 与同一 pane 的 YOLO Mode 位于同一行, 两者 MUST 可以独立切换。

每个 checkbox 的可见性 SHALL 只取决于同一 pane 的 Tool。  Tool 为 `claude` 或 `codex` 时显示, 其他 Tool 时隐藏。  primary 与 secondary 的值 SHALL 相互独立, 初始值分别取自 active profile 解析后的 `cross_agent_team_default`。

#### Scenario: Primary Claude 显示独立开关
- **WHEN** primary Tool 为 `claude`
- **THEN** primary Cross Agent Team checkbox SHALL 显示在 primary YOLO Mode 右侧
- **AND**切换它 SHALL 不改变 secondary pane

#### Scenario: Secondary Codex 显示独立开关
- **WHEN** Right Pane Agent 为 `codex`
- **THEN** secondary Cross Agent Team checkbox SHALL 显示在 secondary YOLO Mode 右侧
- **AND**切换它 SHALL 不改变 primary pane

#### Scenario: 不支持的 Tool 只隐藏自己的开关
- **WHEN**一个 pane 的 Tool 不是 `claude` 或 `codex`
- **THEN**该 pane 的 Cross Agent Team checkbox SHALL 不显示
- **AND**另一个 pane 的可见性 SHALL 不受影响

#### Scenario: 两个 pane 独立应用默认值
- **WHEN** `cross_agent_team_default` 为 true
- **AND** primary 与 secondary 都使用支持的 Tool
- **THEN**两个 pane 的 checkbox SHALL 分别初始化为选中
- **AND**用户 SHALL 可以只关闭其中一个

### Requirement: Development-channels flag on launch

When Cross Agent Team is enabled for a `claude`, non-sandboxed session, AoE SHALL
append `--dangerously-load-development-channels <channel>` to the launched `claude`
command, where `<channel>` is the configured channel string.

AoE SHALL NOT inject the `CROSS_AGENT_TEAMS_MCP_TOKEN` environment variable; the
launched pane inherits it from the environment AoE runs in.

The flag MUST coexist with the YOLO Mode flag (`--dangerously-skip-permissions`)
when both options are enabled.

#### Scenario: Flag appended when enabled

- **WHEN** a claude session is created with Cross Agent Team enabled and Sandbox off
- **THEN** the launched command includes `--dangerously-load-development-channels`
  followed by the configured channel string

#### Scenario: No token injection

- **WHEN** a claude session is launched with Cross Agent Team enabled
- **THEN** AoE does not add `CROSS_AGENT_TEAMS_MCP_TOKEN=...` to the command or its
  injected environment

#### Scenario: Coexists with YOLO Mode

- **WHEN** both YOLO Mode and Cross Agent Team are enabled for a claude session
- **THEN** the launched command includes both `--dangerously-skip-permissions` and
  `--dangerously-load-development-channels <channel>`

#### Scenario: Flag absent when disabled

- **WHEN** a claude session is created with Cross Agent Team disabled
- **THEN** the launched command does not include
  `--dangerously-load-development-channels`

### Requirement: Auto-confirm Claude startup screens

After launching a Cross Agent Team enabled `claude` pane, AoE SHALL detect Claude's
startup confirmation screens and confirm them by sending Enter, repeating until
Claude is ready or a timeout elapses.

AoE SHALL recognize at least the development-channels warning screen (identified by
text such as "Loading development channels" / "I am using this for local
development") and the workspace-trust screen (identified by text such as "trust
this folder" / "Quick safety check"). For both screens the safe-to-proceed option
is the default selection, so confirmation is a single Enter keystroke.

If the confirmation screens do not appear within the timeout, AoE SHALL stop
auto-confirming and leave the pane interactive without erroring the session.

#### Scenario: Dev-channels screen confirmed

- **WHEN** the launched claude pane shows the "Loading development channels" warning
- **THEN** AoE sends Enter to confirm the highlighted "I am using this for local
  development" option

#### Scenario: Trust-folder screen confirmed

- **WHEN** the launched claude pane shows the workspace-trust confirmation screen
- **THEN** AoE sends Enter to confirm the highlighted "Yes, I trust this folder"
  option

#### Scenario: Timeout leaves pane interactive

- **WHEN** no recognized confirmation screen appears within the auto-confirm timeout
- **THEN** AoE stops auto-confirming
- **AND** the session is not marked as failed

### Requirement: Cross Agent Team preserved across restart

Cross Agent Team setting SHALL 按 pane 持久化。  `R` restart、fresh restart 和 cold recovery SHALL 从目标 pane 的 durable config 重建 tool-specific launch command。  Claude pane SHALL 根据自己的值决定 development-channel flag 和 auto-confirm, Codex pane SHALL 根据自己的值决定 pane pre-registration 和 remote app-server bootstrap。

#### Scenario: Claude pane restart 重放自己的配置
- **WHEN**一个开启 Cross Agent Team 的 Claude pane 经 `R` restart
- **THEN**该 pane 的新命令 SHALL 包含 development-channel flag
- **AND** AoE SHALL 对该 pane 再次执行 startup auto-confirm

#### Scenario: 未开启的 sibling 不被装饰
- **WHEN**一个 session 中只有一个 pane 开启 Cross Agent Team
- **AND** session 被 restart 或 recovery
- **THEN**只有该 pane SHALL 使用 Cross Agent Team launch path
- **AND** sibling pane SHALL 使用普通 launch path

#### Scenario: Codex pane resume 保留 token 和独立开关
- **WHEN**一个开启 Cross Agent Team 的 Codex pane 使用有效 resume token restart
- **THEN**该 pane SHALL 再次执行 xats bootstrap
- **AND** native Codex resume token SHALL 保留
- **AND**其他 pane 的 Cross Agent Team 值 SHALL 不改变

#### Scenario: Codex pane fresh restart 重放 bootstrap
- **WHEN**一个开启 Cross Agent Team 的 Codex pane fresh restart
- **THEN**该 pane SHALL pre-register 并连接配置的 local app-server

### Requirement: Codex xats pane bootstrap

When Cross Agent Team is enabled for a non-sandboxed `codex` session, AoE SHALL
launch Codex through a pane-local xats bootstrap. The bootstrap MUST pre-register
the current `TMUX_PANE` with a fresh UUID before executing Codex, then connect the
Codex TUI to the local app-server with that UUID supplied as `xats.agent_id` and
the session project path supplied as the Codex working directory.

When the pane's environment carries a non-empty `XATS_IDENTITY_KEY`, the
bootstrap SHALL tell the pre-registration call to read it, by naming the
variable via `--identity-key-env`; the CLI reads the value from its own
environment. The key's value SHALL NOT appear on the argv of any process the
bootstrap script starts -- not the executed Codex command line and not the
pre-registration call's own -- because argv is readable by every process on
the machine. (The value does reach the pane through AoE's pre-existing
env-injection prefix, which transits the tmux launch argv; that mechanism
predates this change, is shared with Claude panes, and is out of scope here.
What this change adds on top is masking the value in AoE's own debug logs of
launch commands.) The pre-registration call SHALL also carry a lengthened row
TTL (`--ttl`, the flag the CLI parses) so the daemon's poke-back window covers
a Codex cold start.

If a pre-registration call carrying the declared-identity flags fails, the
bootstrap SHALL retry it once with those flags removed and every other flag
kept, so a CLI that does not parse them cannot fail a Codex launch. The retry
SHALL keep naming the pane's identity key and SHALL keep the TTL; retrying
without the key is prohibited by "Codex xats bootstrap failure is explicit"
and that prohibition wins. A pane that declares no identity SHALL make exactly
one pre-registration attempt, because it adds no flag to fall back from and any
fallback would therefore have to drop the key.

The retry decision SHALL rest on the exit code alone, not on the CLI's error
text, and SHALL survive shell options inherited from the environment
(`SHELLOPTS` carrying `errexit` reaches the bootstrap's `sh`).

The bootstrap SHALL NOT read, inject, print, or persist the xats authentication
token value. It SHALL rely on the already-configured local xats environment.

#### Scenario: Fresh Codex xats launch

- **WHEN** a user creates a non-sandboxed Codex session with Cross Agent Team enabled
- **THEN** the target pane is pre-registered with a fresh UUID
- **AND** Codex starts in remote mode against the local app-server
- **AND** Codex receives the project path and the same UUID as `xats.agent_id`

#### Scenario: Identity key rides the pre-registration environment, not any argv

- **WHEN** a Codex Cross Agent Team pane launches with `XATS_IDENTITY_KEY` in its environment
- **THEN** the pre-registration call carries `--identity-key-env` naming the variable
- **AND** neither the pre-registration argv nor the executed Codex command line contains the key's value

#### Scenario: Debug logs of launch commands mask the key's value

- **WHEN** AoE logs a pane launch command or its tmux argv at debug level
- **THEN** the logged text carries the identity-key env prefix with its value struck out

#### Scenario: A pane without an identity key pre-registers without the flag

- **WHEN** a Codex Cross Agent Team pane launches with no `XATS_IDENTITY_KEY` in its environment
- **THEN** the pre-registration call carries no identity-key flag
- **AND** the launch proceeds as before

#### Scenario: A CLI that rejects the declared-identity flags does not fail the launch

- **WHEN** a Codex Cross Agent Team pane with a declared xats identity launches
- **AND** the pre-registration call carrying the declared-identity flags exits non-zero
- **THEN** the bootstrap retries once with the declared-identity flags removed
- **AND** the retry still names the pane's identity key and carries the TTL
- **AND** a successful retry launches Codex normally
- **AND** the retry fires even under shell options inherited from the environment

#### Scenario: An undeclared pane makes exactly one attempt

- **WHEN** a Codex Cross Agent Team pane with no declared identity fails its pre-registration
- **THEN** the bootstrap SHALL NOT make a second pre-registration attempt
- **AND** the pane prints the pre-registration diagnostic and terminates with a non-zero status

#### Scenario: YOLO disabled remains non-YOLO

- **WHEN** Cross Agent Team is enabled for Codex and YOLO Mode is disabled
- **THEN** the Codex command uses the xats bootstrap
- **AND** the command does not include `--dangerously-bypass-approvals-and-sandbox`

#### Scenario: YOLO enabled coexists with xats bootstrap

- **WHEN** Cross Agent Team and YOLO Mode are both enabled for Codex
- **THEN** the Codex command uses the xats bootstrap
- **AND** the command includes `--dangerously-bypass-approvals-and-sandbox`

#### Scenario: Codex fork uses xats bootstrap

- **WHEN** a Cross Agent Team Codex session is forked from a captured native session id
- **THEN** the fork pane is pre-registered with a fresh xats claim
- **AND** the Codex fork command connects to the local app-server
- **AND** the parent native session id is preserved as the fork source

### Requirement: Codex xats bootstrap failure is explicit

When the user requests a Codex Cross Agent Team session, AoE MUST NOT silently
fall back to a normal local Codex launch, nor to a different app-server than the
one the user configured, nor to a registration without the pane's identity key.
Missing pane identity, UUID generation, local app-server availability, xats
pre-registration, or an app-server endpoint AoE will not accept SHALL produce a
specific diagnostic and terminate the pane command with a non-zero status.

Substituting the default endpoint for a rejected one is a prohibited silent
fallback. Its symptom appears on the xats side, as a Codex that connected but
cannot be resumed, so a diagnostic AoE only writes to its own log does not reach
the person debugging it.

A pre-registration that fails SHALL NOT be retried without the pane's identity
key. That key is the only thing by which the daemon recognizes which identity a
pane belongs to, so a pane registered without one is never prompted to
re-register after a restart -- it looks healthy and stays outside Cross Agent
Team for the rest of its life. A pane that legitimately holds no key yet SHALL
still pre-register without one; the prohibition is on discarding a key that
exists, not on registering without one.

#### Scenario: Local app-server unavailable

- **WHEN** a Codex Cross Agent Team pane starts and the configured local app-server is unavailable
- **THEN** the pane prints an app-server availability diagnostic naming that endpoint
- **AND** Codex is not launched without remote mode

#### Scenario: Rejected endpoint aborts the pane

- **WHEN** `CROSS_AGENT_TEAMS_CODEX_WS_URL` holds a value AoE does not accept
- **THEN** the pane command prints a diagnostic naming the variable and the rejected value
- **AND** the pane command terminates with a non-zero status
- **AND** Codex is not launched against the default endpoint

#### Scenario: Pane pre-registration fails

- **WHEN** a Codex Cross Agent Team pane cannot pre-register its pane and UUID
- **THEN** the pane prints a pre-registration diagnostic
- **AND** Codex is not launched without a valid xats pane claim

#### Scenario: A keyed pre-registration failure is not retried without the key

- **WHEN** a Codex Cross Agent Team pane holds an identity key
- **AND** its pre-registration fails
- **THEN** the pane prints a pre-registration diagnostic and terminates with a non-zero status
- **AND** AoE SHALL NOT attempt a pre-registration that omits that key

#### Scenario: A pane with no key still pre-registers

- **WHEN** a Codex Cross Agent Team pane holds no identity key
- **THEN** its pre-registration is attempted without one
- **AND** a failure terminates the pane command rather than being retried

#### Scenario: Cross Agent Team disabled preserves normal Codex

- **WHEN** a Codex session is created with Cross Agent Team disabled
- **THEN** AoE uses the existing normal Codex command path
- **AND** no xats pane bootstrap or remote app-server argument is added

### Requirement: Cross Agent Team configuration

AoE SHALL 在 Settings TUI 中继续提供 global 和 profile 两个 scope 的 Cross Agent Team 配置:

- `cross_agent_team_channel`: Claude development-channel string, 默认 `server:cross-agent-teams-channel`。
- `cross_agent_team_default`: New Session 中每个受支持 pane checkbox 的初始值, 默认 false。

profile override SHALL 继续按现有 merge 规则覆盖 global 值。  channel 可以保持 session 级共享, 但是否启用 SHALL 为 pane 级。

#### Scenario: 自定义 channel 只用于已开启的 Claude pane
- **WHEN** `cross_agent_team_channel` 设置为自定义值
- **AND**一个 Claude pane 开启 Cross Agent Team
- **THEN**该 pane 的命令 SHALL 使用自定义 channel
- **AND**未开启的 Claude pane SHALL 不添加 development-channel flag

#### Scenario: profile default 初始化每个 pane
- **WHEN** profile override 设置 `cross_agent_team_default`
- **THEN** New Session 中每个受支持 pane SHALL 使用该默认值独立初始化
- **AND**清除 override SHALL 回退到 global 值

### Requirement: Cross Agent Team panes carry a durable identity key

When Cross Agent Team is enabled for a pane, AoE SHALL associate that pane with an opaque identity key and SHALL inject it into the launched pane as the `XATS_IDENTITY_KEY` environment variable. The key SHALL be minted by AoE, SHALL be treated as an opaque value that AoE never interprets, and SHALL be injected on every launch of that pane regardless of restart mode, including the pane's first launch.

AoE MAY store and carry a xats team and agent name declared by the user for a pane, and SHALL treat both as opaque values it never interprets, exactly as it treats the identity key. AoE SHALL NOT derive, guess, or default either value from any other data it holds, including the session title, the pane's tool, or the working directory.

#### Scenario: Key injected on first launch

- **WHEN** a Cross Agent Team pane is launched for the first time
- **THEN** AoE SHALL mint an identity key for it
- **AND** the launched pane's environment SHALL contain that key as `XATS_IDENTITY_KEY`

#### Scenario: Key injected for both supported tools

- **WHEN** a Cross Agent Team pane is launched for `claude` or for `codex`
- **THEN** the launched pane's environment SHALL contain the pane's identity key

#### Scenario: Key is distinct from the codex pane pre-registration nonce

- **WHEN** a Cross Agent Team `codex` pane is launched
- **THEN** the pane SHALL carry both its durable identity key and a freshly generated single-use pane pre-registration nonce
- **AND** the two values SHALL be different

#### Scenario: No key when the feature is disabled

- **WHEN** a session is launched with Cross Agent Team disabled
- **THEN** AoE SHALL NOT mint or inject an identity key

#### Scenario: Key is not exposed through command arguments

- **WHEN** a Cross Agent Team pane is launched
- **THEN** the identity key SHALL NOT appear in the launch command's arguments
- **AND** it SHALL NOT be written to logs

#### Scenario: A declared name is never invented by AoE

- **WHEN** a Cross Agent Team pane has no declared xats team or agent name
- **THEN** AoE SHALL treat the identity as undeclared
- **AND** SHALL NOT substitute the session title or any other value in its place

### Requirement: Identity key storage follows the pane's role

每个 managed pane 的 identity key SHALL 跟随该 pane 的 durable slot 保存, 包括 primary pane。  instance record MAY 在 slot 0 建立前保留 primary identity key 作为首次启动 bootstrap 或 migration mirror, 但 SHALL NOT 在 slot 0 已有非空 key 时覆盖它。  slot SHALL 同时保存决定该 key 是否应被注入的 pane-level Cross Agent Team config。

#### Scenario: Primary key 存在 slot 0
- **WHEN** primary pane 开启 Cross Agent Team 并获得 identity key
- **THEN**该 key SHALL 保存到 slot 0
- **AND**关闭并重开 AoE 后 SHALL 仍可读取同一个 key

#### Scenario: Secondary key 存在自己的 slot
- **WHEN** secondary pane 开启 Cross Agent Team 并获得 identity key
- **THEN**该 key SHALL 保存到 secondary pane 的 durable slot
- **AND**它 SHALL 与 primary key 相互独立

#### Scenario: 旧 primary key 迁移到 slot 0
- **WHEN** migration 读取到旧 instance record 上的 primary identity key
- **THEN** migration SHALL 把它写入 slot 0
- **AND**重复运行 migration SHALL 不覆盖 slot 0 已有的非空 key

### Requirement: Panes AoE never launched receive a key at their first relaunch

Agent panes are adopted observe-first: a user may split a pane and start an agent in it by hand, and AoE never builds that pane's launch command. AoE SHALL NOT attempt to inject a key into such a pane while it is running. It SHALL mint and inject one the first time it launches that pane's slot itself, after which the key is stable like any other.

The consequence is bounded rather than permanent: the key is bound to the identity during the registration that follows its first injection, so such a pane costs one extra manual registration and recovers normally from then on.

#### Scenario: Hand-started pane has no key until AoE relaunches it

- **WHEN** a user starts an agent by hand in a split pane of a Cross Agent Team session
- **AND** the reconciler adopts that pane into a slot
- **THEN** the slot SHALL carry no identity key
- **AND** AoE SHALL NOT alter the running pane

#### Scenario: First AoE relaunch mints the slot's key

- **WHEN** AoE launches an adopted slot that has no identity key
- **THEN** AoE SHALL mint one, persist it on the slot, and inject it into the launched pane
- **AND** subsequent launches of that slot SHALL reuse it

#### Scenario: Key that is not yet bound does not fail the launch

- **WHEN** a pane is launched with a freshly minted identity key that no identity has been registered against yet
- **THEN** AoE SHALL treat the launch as successful
- **AND** SHALL retain the key so the registration that follows can bind it

### Requirement: Identity key is stable across relaunch, restart, and recovery

A pane's identity key SHALL be minted once and reused on every subsequent launch of that pane's slot. Resume restart, clean restart, resume recovery, and clean recovery SHALL all inject the slot's existing key rather than minting a new one.

#### Scenario: Clean restart reuses the key

- **WHEN** a Cross Agent Team session is restarted clean
- **THEN** each relaunched pane's environment SHALL contain the same identity key it carried before the restart

#### Scenario: Clean recovery reuses the key

- **WHEN** a recoverable Cross Agent Team instance is recovered in fresh mode
- **THEN** each recovered pane SHALL be launched with the identity key stored on its durable slot record

#### Scenario: Key survives AoE restart

- **WHEN** an identity key has been persisted for a slot and AoE is closed and reopened
- **THEN** the same key SHALL be injected on the next launch of that slot

#### Scenario: The launch that mints the key persists it

- **WHEN** a Cross Agent Team session is launched and that launch mints the instance's identity key
- **THEN** the minted key SHALL be stored on the session record as part of that launch
- **AND** the next restart SHALL inject the stored key rather than minting a new one

Minting the key on a working copy of the instance and discarding it leaves the record keyless, so the first restart mints a second key. The daemon then finds no holder for the new key and treats the restarted pane as a new identity instead of a recovering one, while the old key stays bound to the dead pane.

### Requirement: Cloned and forked sessions receive a fresh identity key

When a session is created from an existing session through new-from-selection, or when a pane is forked, AoE SHALL mint a new identity key for the resulting pane and SHALL NOT copy the source pane's key.

This is the only point at which two panes claiming one identity can be prevented. Once a copied key has been bound, the daemon cannot distinguish a pane recovering its own identity from a pane presenting a copied key.

#### Scenario: New-from-selection does not inherit the key

- **WHEN** a Cross Agent Team session is created from an existing session through new-from-selection
- **THEN** the new session's pane SHALL carry an identity key different from the source pane's key

#### Scenario: Fork does not inherit the key

- **WHEN** a Cross Agent Team pane is forked
- **THEN** the forked pane SHALL carry an identity key different from its parent's key

### Requirement: Unresolvable identity key degrades to normal registration

An identity key that no longer corresponds to a known identity SHALL be treated as a normal state. AoE SHALL NOT report an error, SHALL NOT clear the stored key, and SHALL leave the pane usable so the user can register it the same way they do without a key.

#### Scenario: Key no longer resolves

- **WHEN** a pane is launched with a stored identity key that no longer corresponds to a known identity
- **THEN** AoE SHALL NOT surface an error for the session
- **AND** AoE SHALL retain the stored key for future launches
- **AND** the pane SHALL remain usable for manual registration

### Requirement: Extra agent panes AoE launches carry an identity key from their first launch

当 AoE 启动一个 pane-level Cross Agent Team 已开启的额外 agent pane 时, SHALL 在首次启动时创建独立 identity key, 保存到该 pane 的 durable slot, 并以 `XATS_IDENTITY_KEY` 注入进程环境。  该规则覆盖 New Session secondary pane 和 `aoe session add-agent-pane` 的明确 pane 配置。

额外 pane SHALL 不复用任何 sibling pane 的 key。  只有该 pane 自己开启 Cross Agent Team 时才创建 key, session 中其他 pane 的开关 SHALL 不作为判断依据。  shell pane SHALL 永远不创建 key。

#### Scenario: 只有 secondary 开启时仍获得 key
- **WHEN** primary pane 关闭 Cross Agent Team
- **AND** secondary agent pane 开启 Cross Agent Team
- **THEN** secondary pane SHALL 在首次启动时获得并持久化自己的 key
- **AND** primary pane SHALL 不获得 key

#### Scenario: 只有 primary 开启时 secondary 不获得 key
- **WHEN** primary pane 开启 Cross Agent Team
- **AND** secondary agent pane 关闭 Cross Agent Team
- **THEN** secondary pane SHALL 不创建或注入 identity key

#### Scenario: 两个 pane 都开启时 key 不同
- **WHEN** primary 与 secondary agent pane 都开启 Cross Agent Team
- **THEN**两个 pane SHALL 分别获得不同 key

#### Scenario: Restart 复用 extra pane key
- **WHEN**一个额外 pane 已持久化 identity key
- **AND**该 pane restart 或 recovery
- **THEN**该 pane SHALL 复用原 key 而不是重新创建

#### Scenario: Shell extra pane 没有 key
- **WHEN**额外 pane Tool 为 `shell`
- **THEN**该 pane SHALL 不创建 identity key

#### Scenario: key 持久化失败需要显式报告
- **WHEN**额外 pane 已启动但其 identity key 无法持久化
- **THEN**失败 SHALL 显示给用户而不是只写日志
- **AND**系统 SHALL 精确关闭本次启动的 pane
- **AND**只回滚本次新建且尚未转交 durable state 的 Worktree
- **AND**任何复用 Worktree SHALL 保持不变

### Requirement: Cross Agent Team launch decoration is pane scoped

所有 tool-specific Cross Agent Team 行为 SHALL 使用目标 pane 的配置判断, 不得读取 sibling pane 或 session 级 enabled 值替代。  该行为包括 Claude development-channel 与 auto-confirm、Codex pre-registration 与 remote bootstrap、identity key minting 和 injection。

#### Scenario: Right-only Claude 使用 development channel
- **WHEN** primary pane 关闭 Cross Agent Team
- **AND** secondary Claude pane 开启 Cross Agent Team
- **THEN** secondary Claude command SHALL 包含 development-channel flag
- **AND** primary command SHALL 不包含 Cross Agent Team decoration

#### Scenario: Right-only Codex 使用 xats bootstrap
- **WHEN** primary pane 关闭 Cross Agent Team
- **AND** secondary Codex pane 开启 Cross Agent Team
- **THEN** secondary Codex SHALL 使用 pane-local xats bootstrap
- **AND** primary pane SHALL 保持普通启动

#### Scenario: 新 adopt 的 pane 不继承 primary 开关
- **WHEN** reconciler 首次 adopt 一个没有 durable pane config 的非 primary pane
- **THEN**该 pane 的 Cross Agent Team SHALL 初始化为 false
- **AND** primary pane 的 enabled 值 SHALL 不复制到该 pane

### Requirement: Cross Agent Team panes carry a declared xats identity

A pane with Cross Agent Team enabled SHALL be able to carry a user-declared xats
identity consisting of a team and an agent name, and a declaration SHALL belong
to exactly one live pane. Both parts SHALL be independently optional: an empty
value means undeclared, and a pane with both parts empty SHALL behave exactly as
panes behaved before this capability existed.

The declared identity SHALL be stored on the pane's own durable slot, alongside
that pane's identity key, so sibling panes in the same session declare
independently. Storage SHALL tolerate records written before this capability
existed by reading them as undeclared.

#### Scenario: Declared identity persists on the pane's slot

- **WHEN** a user declares a xats team and agent name for a Cross Agent Team pane
- **THEN** AoE SHALL store both values on that pane's durable slot
- **AND** reopening AoE SHALL read back the same values

#### Scenario: Sibling panes declare independently

- **WHEN** two panes of the same session each declare a xats identity
- **THEN** each pane's declared values SHALL be stored on its own slot
- **AND** neither pane's values SHALL overwrite the other's

#### Scenario: Records predating the capability read as undeclared

- **WHEN** AoE reads a slot record written before this capability existed
- **THEN** the pane's declared team and agent name SHALL read as empty
- **AND** the pane SHALL launch exactly as it did before

#### Scenario: Only one part declared

- **WHEN** a pane declares a team but no agent name
- **THEN** AoE SHALL store and carry the declared part
- **AND** SHALL carry the undeclared part as empty rather than substituting a value

### Requirement: Declared identity is injected into the launched pane

When a Cross Agent Team pane with a declared xats identity is launched, AoE SHALL
inject the declared parts into that pane's environment, so an agent able to read
its own environment can register under that identity without asking the user.
Undeclared parts SHALL NOT be injected as empty variables.

Unlike the identity key, the declared identity is not a credential, so it MAY
appear in launch command arguments and MAY be logged.

#### Scenario: Declared identity reaches the pane environment

- **WHEN** a Cross Agent Team pane with a declared team and agent name is launched
- **THEN** the launched pane's environment SHALL carry both declared values
- **AND** they SHALL be carried alongside the pane's identity key

#### Scenario: Undeclared identity injects nothing

- **WHEN** a Cross Agent Team pane with no declared identity is launched
- **THEN** the launched pane's environment SHALL carry no declared-identity variables
- **AND** the launch command SHALL be unchanged from before this capability existed

#### Scenario: Injection is independent of the pane's tool

- **WHEN** a Cross Agent Team pane declaring an identity is launched for any supported tool
- **THEN** the declared values SHALL be injected the same way for every such tool

### Requirement: Declared identity is stable across restart, resume, and recovery

A pane's declared xats identity SHALL survive every relaunch path that preserves
the pane's slot, including restart, resume, fresh restart, and cold-start
recovery, and SHALL be injected on each of those launches. AoE SHALL NOT mint,
clear, or rotate a declared identity on relaunch.

#### Scenario: Declared identity survives restart

- **WHEN** a pane with a declared xats identity is restarted
- **THEN** the relaunched pane SHALL carry the same declared values

#### Scenario: Declared identity survives cold-start recovery

- **WHEN** AoE recovers a pane's slot after its tmux session was lost
- **THEN** the recovered pane SHALL carry the same declared values

#### Scenario: Relaunch never clears a declaration

- **WHEN** a pane with a declared xats identity is relaunched by any path
- **THEN** AoE SHALL NOT write an empty declared identity over the stored one

#### Scenario: A fork does not inherit the parent's declaration

- **WHEN** a session whose pane declares a xats identity is forked
- **THEN** the forked session's pane SHALL start undeclared
- **AND** the parent SHALL keep its own declaration

### Requirement: Declared identity reaches the daemon through Codex pre-registration

A non-sandboxed Codex pane's bootstrap SHALL pass that pane's declared xats identity to the pre-registration call as arguments, so the daemon can address the pane by identity even when the pane's identity key resolves to no holder.

This channel exists because Codex tool processes run inside a shared app-server
and therefore read that server's environment rather than their own pane's: the
declaration must reach the daemon before Codex starts, since Codex itself can
never see it.

Undeclared parts SHALL NOT be passed. A pane with no declared identity SHALL
produce the same pre-registration call it produced before this capability
existed.

#### Scenario: Declared identity is passed at pre-registration

- **WHEN** a Codex Cross Agent Team pane with a declared team and agent name launches
- **THEN** the pre-registration call SHALL carry both declared values as arguments

#### Scenario: Undeclared Codex pane is unchanged

- **WHEN** a Codex Cross Agent Team pane with no declared identity launches
- **THEN** the pre-registration call SHALL carry no declared-identity arguments

### Requirement: Declared identity is entered per pane where the feature is switched on

Wherever a user can turn Cross Agent Team on for a pane, the user SHALL also be
able to declare that pane's xats team and agent name. The fields SHALL be
presented per pane, SHALL accept an empty value to mean undeclared, SHALL refuse
values storage would refuse, and SHALL be inert when Cross Agent Team is off for
that pane.

A declaration is entered when the pane is configured. AoE has no flow for
reconfiguring an already-created pane, so this capability adds none: a stored
declaration SHALL be replaced only by a later non-empty declaration for that
slot, and SHALL NOT be cleared by any launch, restart, or recovery that carries
no declaration.

#### Scenario: Declaring an identity when creating a session

- **WHEN** a user enables Cross Agent Team for a pane while creating a session
- **THEN** the user SHALL be able to enter that pane's xats team and agent name
- **AND** each pane of that session SHALL take its own values

#### Scenario: Clearing a field before submitting means undeclared

- **WHEN** a user types a declaration and then clears the field before submitting
- **THEN** the submitted pane SHALL carry that part as undeclared

#### Scenario: A rejected value never reaches storage

- **WHEN** a user enters a value the storage boundary refuses
- **THEN** the field SHALL refuse it at entry

#### Scenario: Characters the daemon reads as addressing syntax are refused at entry

- **WHEN** a user types a declared agent name containing `:`, `(` or `)`
- **THEN** the field SHALL refuse those characters
- **AND** a declared team SHALL likewise refuse `(` and `)`
- **AND** the refusal SHALL happen at entry rather than at launch, because the
  Codex bootstrap cannot distinguish the daemon rejecting a bad value from an
  older CLI rejecting an unknown flag, and would drop the declaration silently

#### Scenario: A declaration carries no quote and no line terminator

- **WHEN** a user types a double quote into either declared-identity field
- **THEN** the field SHALL refuse it, because the daemon interpolates a declared
  name into a notice as `name="${name}"` and a quote closes that early
- **AND** both fields SHALL likewise refuse U+2028 and U+2029, which terminate a
  line without belonging to the control-character category a general "no control
  characters" rule tests

#### Scenario: A tightened daemon rule reaches this validation

- **WHEN** the daemon starts refusing a character these fields still accept
- **THEN** this validation SHALL be updated to match
- **AND** the reason it SHALL not be left to the daemon alone is that the Codex
  bootstrap's retry turns a value the daemon refuses into a launch that looks
  healthy and carries no declaration -- so a rule tightened on one side and not
  the other fails silently on this side

#### Scenario: Fields are inert when the feature is off

- **WHEN** Cross Agent Team is off for a pane
- **THEN** the declared-identity fields SHALL NOT be editable for that pane
- **AND** a declaration typed before the switch was turned off SHALL NOT be submitted

