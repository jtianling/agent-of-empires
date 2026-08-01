# cross-agent-team Specification

## Purpose
TBD - created by archiving change cross-agent-team-launch. Update Purpose after archive.
## Requirements
### Requirement: Cross Agent Team launch option in New Session

The New Session dialog SHALL present a "Cross Agent Team" checkbox positioned to
the right of the YOLO Mode checkbox. The option SHALL be independent of YOLO Mode:
toggling one MUST NOT change the other.

The option SHALL only be available when the selected primary tool is `claude` or
`codex`, and MUST be hidden or disabled when Sandbox is enabled.

The checkbox's initial state SHALL be taken from the `cross_agent_team_default`
configuration value resolved through the active profile.

#### Scenario: Option visible for Claude without Sandbox

- **WHEN** the user opens New Session with tool `claude` and Sandbox not enabled
- **THEN** a "Cross Agent Team" checkbox is shown to the right of YOLO Mode
- **AND** it can be toggled independently of YOLO Mode

#### Scenario: Option visible for Codex without Sandbox

- **WHEN** the user opens New Session with tool `codex` and Sandbox not enabled
- **THEN** a "Cross Agent Team" checkbox is shown to the right of YOLO Mode
- **AND** it can be toggled independently of YOLO Mode

#### Scenario: Option hidden for unsupported tools

- **WHEN** the user selects a primary tool other than `claude` or `codex` in New Session
- **THEN** the Cross Agent Team checkbox is not shown

#### Scenario: Option disabled when Sandbox enabled

- **WHEN** the user enables Sandbox in New Session with tool `claude` or `codex`
- **THEN** the Cross Agent Team checkbox is hidden or non-selectable
- **AND** the session is not launched with tool-specific Cross Agent Team behavior

#### Scenario: Default state from configuration

- **WHEN** `cross_agent_team_default` is true for the active profile
- **AND** the user opens New Session with tool `claude` or `codex` and Sandbox not enabled
- **THEN** the Cross Agent Team checkbox is pre-checked

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

The Cross Agent Team setting SHALL persist with the session. On `R` restart and
fresh restart, AoE SHALL rebuild the tool-specific launch command. Claude SHALL
receive the development-channels flag and auto-confirm flow. Codex SHALL repeat
the pane pre-registration and remote app-server bootstrap.

#### Scenario: Claude graceful resume re-applies behavior

- **WHEN** a Cross Agent Team Claude session is restarted via `R` along the graceful-resume path
- **THEN** the resumed command includes `--dangerously-load-development-channels`
- **AND** AoE auto-confirms the startup screens again

#### Scenario: Claude kill-and-recreate re-applies behavior

- **WHEN** a Cross Agent Team Claude session is restarted via `R` along the kill-and-recreate path
- **THEN** the recreated command includes `--dangerously-load-development-channels`
- **AND** AoE auto-confirms the startup screens again

#### Scenario: Codex resume restart re-applies xats bootstrap

- **WHEN** a Cross Agent Team Codex session is restarted via `R` with a resume token
- **THEN** the pane is pre-registered with a fresh xats claim
- **AND** the resumed Codex command connects to the configured local app-server
- **AND** the native Codex resume token is preserved

#### Scenario: Codex fresh restart re-applies xats bootstrap

- **WHEN** a Cross Agent Team Codex session is restarted fresh
- **THEN** the pane is pre-registered with a fresh xats claim
- **AND** a fresh Codex TUI connects to the configured local app-server

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

If a pre-registration call carrying the new flags fails, the bootstrap SHALL
retry it once as the exact pre-change call (no identity-key flag, no TTL), so
a daemon that predates the flags cannot fail a Codex launch. The retry
decision SHALL rest on the exit code alone, not on the CLI's error text, and
SHALL survive shell options inherited from the environment (`SHELLOPTS`
carrying `errexit` reaches the bootstrap's `sh`).

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

#### Scenario: An old daemon that rejects the new flags does not fail the launch

- **WHEN** the pre-registration call carrying the new flags exits non-zero
- **THEN** the bootstrap retries the pre-registration without the new flags
- **AND** a successful retry launches Codex normally
- **AND** the retry fires even under shell options inherited from the environment

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
fall back to a normal local Codex launch. Missing pane identity, UUID generation,
local app-server availability, or xats pre-registration SHALL produce a specific
diagnostic and terminate the pane command with a non-zero status. The
identity-key fallback retry is the one sanctioned second attempt: only after
both pre-registration calls fail does the launch terminate.

#### Scenario: Local app-server unavailable

- **WHEN** a Codex Cross Agent Team pane starts and the configured local app-server is unavailable
- **THEN** the pane prints an app-server availability diagnostic
- **AND** Codex is not launched without remote mode

#### Scenario: Pane pre-registration fails

- **WHEN** both pre-registration attempts of a Codex Cross Agent Team pane fail
- **THEN** the pane prints a pre-registration diagnostic
- **AND** Codex is not launched without a valid xats pane claim

#### Scenario: Cross Agent Team disabled preserves normal Codex

- **WHEN** a Codex session is created with Cross Agent Team disabled
- **THEN** AoE uses the existing normal Codex command path
- **AND** no xats pane bootstrap or remote app-server argument is added

### Requirement: Cross Agent Team configuration

AoE SHALL expose Cross Agent Team configuration in the settings TUI, editable both
globally and per profile:

- `cross_agent_team_channel`: the channel string appended after the
  development-channels flag, defaulting to `server:cross-agent-teams-channel`.
- `cross_agent_team_default`: whether the New Session checkbox starts checked,
  defaulting to false.

Profile overrides SHALL merge with global values following the existing profile
override merge logic.

#### Scenario: Channel value used on launch

- **WHEN** `cross_agent_team_channel` is set to a custom value
- **AND** a Cross Agent Team claude session is launched
- **THEN** the launched command appends that custom channel string after
  `--dangerously-load-development-channels`

#### Scenario: Per-profile override

- **WHEN** a profile overrides `cross_agent_team_default` or
  `cross_agent_team_channel`
- **THEN** sessions created under that profile use the overridden value
- **AND** clearing the override falls back to the global value

### Requirement: Cross Agent Team panes carry a durable identity key

When Cross Agent Team is enabled for a pane, AoE SHALL associate that pane with an opaque identity key and SHALL inject it into the launched pane as the `XATS_IDENTITY_KEY` environment variable. The key SHALL be minted by AoE, SHALL be treated as an opaque value that AoE never interprets, and SHALL be injected on every launch of that pane regardless of restart mode, including the pane's first launch.

AoE SHALL NOT read, store, display, or configure a xats team or agent name.

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

### Requirement: Identity key storage follows the pane's role

The primary pane's identity key SHALL be stored on the instance record, alongside the other state describing that same agent (its pre-allocated session id, resume token, and pending fork). An adopted pane's identity key SHALL be stored on its durable slot record.

#### Scenario: Primary key survives with the instance record

- **WHEN** a Cross Agent Team session's primary pane has an identity key and AoE is closed and reopened
- **THEN** the instance record SHALL still carry that key

#### Scenario: Adopted slot key survives with the slot record

- **WHEN** an adopted pane's slot has an identity key and AoE is closed and reopened
- **THEN** the durable slot record SHALL still carry that key

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

When AoE launches an additional agent pane for a Cross Agent Team session, it SHALL mint an identity key for that pane at launch, persist it on the pane's durable slot record, and inject it into the launched process as `XATS_IDENTITY_KEY`. This covers both launch entry points: the right pane of a new session, and `aoe session add-agent-pane`.

The key SHALL be freshly minted for that pane. AoE SHALL NOT reuse the instance's own key for it: two live panes presenting one identity is the state the recovery design cannot resolve, and it is preventable only at the moment the second pane is launched.

An extra pane is not a pane AoE never launched. AoE builds its command and knows its slot, so the allowance that a pane may run keyless until its first relaunch applies only to panes a user started by hand.

The key travels the same route as the primary pane's: an environment assignment prefixing the pane's start command, which the pane's shell consumes before the agent runs. The agent process therefore never carries it in its arguments. It does remain readable from the pane's recorded start command by anything that can talk to the same tmux server, which is a property of the existing injection route rather than of this change, and is not addressed here.

#### Scenario: Right pane of a new session is launched with a key

- **WHEN** a Cross Agent Team session is created with a right pane agent tool
- **THEN** the right pane process environment SHALL contain `XATS_IDENTITY_KEY`
- **AND** the key SHALL be recorded on that pane's durable slot record

#### Scenario: A pane added through the CLI is launched with a key

- **WHEN** `aoe session add-agent-pane` launches an agent pane into a Cross Agent Team session
- **THEN** the launched pane's environment SHALL contain `XATS_IDENTITY_KEY`

#### Scenario: The extra pane's key is not the primary pane's key

- **WHEN** a Cross Agent Team session is created with a right pane agent tool
- **THEN** the right pane's identity key SHALL differ from the key injected into the primary pane

#### Scenario: The extra pane's key is reused rather than reminted on restart

- **WHEN** a session whose extra pane was launched with a key is restarted
- **THEN** the relaunched extra pane SHALL carry the same identity key it carried before the restart

#### Scenario: No key when Cross Agent Team is disabled

- **WHEN** an extra agent pane is launched for a session that does not have Cross Agent Team enabled
- **THEN** no identity key SHALL be minted and no `XATS_IDENTITY_KEY` SHALL be injected

#### Scenario: A shell extra pane receives no key

- **WHEN** an extra pane is launched with the `shell` tool
- **THEN** no identity key SHALL be minted for it

#### Scenario: Failing to record the key is reported, not swallowed

- **WHEN** an extra agent pane is launched but its identity key cannot be persisted
- **THEN** the failure SHALL be surfaced to the user rather than only logged
- **AND** the pane SHALL be left running, because it is usable and relaunching it would not repair the record

#### Scenario: Key reaches the agent through the environment, not its arguments

- **WHEN** an extra agent pane is launched with an identity key
- **THEN** the key SHALL be passed as an environment assignment that the pane's shell consumes
- **AND** the key value SHALL NOT appear in the arguments of the agent process itself

