## ADDED Requirements

### Requirement: Agent runtime capability comes from the registry

AoE SHALL express "this agent uses an AoE-prepared exact session runtime" and the runtime's server ownership as `AgentDef` fields rather than as tool-name comparisons at call sites.  Server ownership MUST distinguish an AoE-owned per-pane server from a shared singleton server that AoE MUST NOT terminate.

#### Scenario: Adding an agent does not add call-site comparisons
- **WHEN** a new agent is given an exact session runtime
- **THEN** the launch, restart and identity paths SHALL branch on the registry field
- **AND** SHALL NOT add a new tool-name string comparison

#### Scenario: Shared server ownership forbids termination
- **WHEN** an agent's runtime is declared as a shared singleton server
- **THEN** AoE SHALL NOT start or terminate that server as part of pane lifecycle
- **AND** SHALL treat an absent server as a launch failure

### Requirement: Kimi shared server is discovered and never managed

AoE SHALL locate the kimi server only through its instance registry under the kimi home directory, selecting the earliest started live instance, and SHALL connect to that instance's explicit loopback address.  AoE MUST NOT start, restart or terminate the kimi server, and MUST NOT assume a hard coded port.

#### Scenario: Earliest live instance is selected
- **WHEN** the registry contains multiple instances
- **THEN** AoE SHALL order them by start time ascending and select the first live one
- **AND** SHALL use that instance's host and port for every request of this launch

#### Scenario: Liveness uses process existence only
- **WHEN** AoE evaluates whether a registry entry is live
- **THEN** it SHALL decide from process existence for the recorded pid
- **AND** SHALL NOT treat a stale heartbeat timestamp as evidence of death

#### Scenario: Unreadable entries are preserved
- **WHEN** a registry entry exceeds the size limit, is not JSON or lacks required fields
- **THEN** AoE SHALL ignore that entry for selection
- **AND** SHALL NOT delete it

#### Scenario: No live server fails closed
- **WHEN** the registry directory is missing or contains no live instance
- **THEN** AoE SHALL fail the kimi pane launch with an actionable diagnostic
- **AND** SHALL NOT start the server, scan ports or fall back to a shell pane

### Requirement: Cross Agent Team kimi panes require a capable engine

AoE SHALL determine, before starting the pane process, whether the selected kimi server runs an engine that can host the pane's session, and SHALL refuse the launch when it cannot.  AoE MUST NOT launch a Cross Agent Team kimi pane in a degraded mode where delivery reaches an engine the user cannot see.

#### Scenario: Incapable engine fails closed
- **WHEN** the selected server cannot host the pane's session for the TUI
- **THEN** AoE SHALL refuse the launch and explain what the server must provide
- **AND** SHALL NOT fall back to a mode where a poke runs a turn the user cannot observe

#### Scenario: Capability is judged from the server and the binary together
- **WHEN** AoE evaluates engine capability
- **THEN** it SHALL require a positive signal from the running server instance
- **AND** SHALL require that the pane's kimi binary be explicitly configured rather than resolved from the search path
- **AND** SHALL refuse when either part is unsatisfied

#### Scenario: Version is never a capability signal
- **WHEN** AoE evaluates engine capability
- **THEN** it SHALL NOT use the reported engine version in either direction
- **AND** SHALL treat a server-side signal left behind by a dead instance as absent

#### Scenario: Binding is confirmed rather than assumed
- **WHEN** a Cross Agent Team kimi pane has started
- **THEN** AoE SHALL be able to distinguish "an agent bound this session and acted" from "nothing ever bound"
- **AND** SHALL NOT report the pane as connected on the strength of a successful commit alone

### Requirement: Exact kimi session is minted before the TUI starts

For a fresh kimi conversation AoE SHALL create the session through the kimi server REST API, set its model and permission mode, and materialize its main agent, all before starting the pane process.  The created session's recorded working directory MUST equal the pane's working directory.  AoE MUST NOT select a session by cwd, recency or title.

#### Scenario: Fresh launch mints one exact session
- **WHEN** AoE launches a kimi pane without a durable session id
- **THEN** AoE SHALL create a session, set its profile, materialize its main agent
- **AND** SHALL persist the returned id to that pane's durable slot before starting the TUI

#### Scenario: Main agent materialization leaves no message
- **WHEN** AoE materializes the main agent of a newly created session
- **THEN** it SHALL use the read path that performs materialization as a side effect
- **AND** the conversation SHALL contain no AoE-authored message
- **AND** AoE SHALL NOT poll for a filesystem artifact

#### Scenario: Working directory mismatch is rejected
- **WHEN** the pane working directory differs from the session's recorded working directory
- **THEN** AoE SHALL fail the launch before starting the TUI

#### Scenario: Profile setup is mandatory
- **WHEN** session creation succeeds but setting the model or permission mode fails
- **THEN** AoE SHALL fail the launch
- **AND** SHALL NOT start a TUI on a session whose server driven turns cannot run

#### Scenario: Same cwd panes never share a session
- **WHEN** two kimi panes use the same working directory
- **THEN** each pane SHALL use its own durable slot session id
- **AND** neither SHALL query a session list to choose its conversation

### Requirement: Kimi delivery coordinates are committed before attach

For a Cross Agent Team kimi pane with an identity key, AoE SHALL invoke the xats daemon `POST /api/runtime/kimi/commit` with `{protocol_version, identity_key, base_url, session_id}` on every launch, before starting the pane process.  AoE SHALL discover the daemon only from its validated pid file and connect only to its explicit loopback port.

#### Scenario: Commit runs on resume as well as on fresh
- **WHEN** a kimi pane launches with an unchanged durable session id
- **THEN** AoE SHALL still invoke commit
- **AND** SHALL accept the idempotent result as success

#### Scenario: Conflict is surfaced before the TUI starts
- **WHEN** commit reports that the session is claimed by another agent row
- **THEN** AoE SHALL fail the launch with that diagnostic
- **AND** SHALL NOT start the pane process

#### Scenario: First launch permits an unregistered identity
- **WHEN** commit reports that the identity key is not yet registered
- **THEN** AoE SHALL treat it as a normal first launch and continue
- **AND** SHALL NOT invent name or team data

#### Scenario: Only probe failure is retried
- **WHEN** commit reports that the session could not be verified
- **THEN** AoE SHALL retry within a bounded attempt count using the identical tuple
- **WHEN** commit reports a protocol mismatch, an agent type conflict, a missing auth token, a client HTTP status or an unknown outcome
- **THEN** AoE SHALL fail closed immediately

#### Scenario: A key-less row is adopted by its previous coordinates
- **WHEN** a launch mints a different session id than the slot previously held
- **THEN** AoE SHALL first commit the previous coordinates so the daemon can resolve that row and adopt the identity key
- **AND** SHALL then commit the new coordinates, which the daemon resolves by that key
- **WHEN** the launch keeps the same session id
- **THEN** AoE SHALL commit once, because the two calls would be identical

#### Scenario: Commit success is not a health claim
- **WHEN** commit succeeds while reporting that it did not probe the session
- **THEN** AoE SHALL NOT report or record the session as verified alive

### Requirement: Commit is the last writer of delivery coordinates

AoE SHALL complete the xats commit before starting the pane process, and SHALL NOT trigger any further xats registration action for that pane afterwards.  This ordering MUST hold regardless of whether the kimi runtime supports automatic connection binding.

#### Scenario: No registration action follows commit
- **WHEN** a kimi pane launch has committed its coordinates
- **THEN** AoE SHALL start the pane process as the next xats-affecting step
- **AND** SHALL NOT issue another registration or reconnect call for that pane

#### Scenario: Fresh conversation replacement is serialized
- **WHEN** the user requests a fresh conversation for a running kimi pane
- **THEN** AoE SHALL terminate the previous pane process and confirm its exit
- **AND** SHALL mint the new session and commit only after that confirmation

### Requirement: Kimi MCP configuration is validated and never rewritten

AoE SHALL verify that the user's kimi MCP configuration declares the xats server with session scope and with the session id header template before launching a Cross Agent Team kimi pane.  AoE MUST NOT create or modify that configuration.

#### Scenario: Missing or non-conforming configuration fails closed
- **WHEN** the configuration is absent, omits the xats server, omits the session id header or does not declare session scope
- **THEN** AoE SHALL fail the launch
- **AND** SHALL print the exact configuration the user needs to add
- **AND** SHALL NOT write the configuration file

#### Scenario: One configuration serves every pane
- **WHEN** several kimi panes run concurrently
- **THEN** AoE SHALL rely on a single user level configuration
- **AND** SHALL NOT generate per pane configuration files

### Requirement: Kimi pane environment is per pane and carries no identity key

AoE SHALL inject the selected base URL, that pane's session id and the remote engine mode into the kimi child environment, removing any inherited value of those names first.  AoE MUST NOT place the identity key in the kimi pane environment or command, because a kimi tool process spawned by the shared server inherits that server's environment and would expose the key to every kimi agent on that server.

#### Scenario: Identity key never reaches a kimi pane
- **WHEN** AoE builds a Cross Agent Team kimi pane command
- **THEN** neither the command nor the injected environment SHALL contain the identity key
- **AND** the key SHALL be read from the durable slot and used only in the xats control plane request body

#### Scenario: Inherited values cannot leak between panes
- **WHEN** AoE builds a kimi pane command
- **THEN** each injected variable SHALL be removed before being set
- **AND** the command SHALL NOT contain any other pane's session id

#### Scenario: Remote engine mode is required
- **WHEN** AoE launches a Cross Agent Team kimi pane
- **THEN** the child environment SHALL select the shared server engine
- **AND** SHALL NOT leave the TUI on its default in-process engine

### Requirement: Kimi restart uses the durable session

`Shift+R` SHALL resume each kimi slot's persisted conversation and `Shift+C` SHALL create a new conversation for that slot while preserving its identity key.  Neither action may fall back to cwd or recency inference.

#### Scenario: Resume uses the exact stored session
- **WHEN** the user resumes a kimi pane
- **THEN** AoE SHALL attach to the durable slot's session id
- **AND** SHALL report a per-pane error when that id is missing or invalid rather than starting a fresh conversation

#### Scenario: Fresh keeps the identity key
- **WHEN** the user requests a fresh conversation for a kimi pane
- **THEN** AoE SHALL mint a new session id for that slot
- **AND** SHALL reuse the slot's existing identity key

#### Scenario: Sandboxed kimi fails closed
- **WHEN** the user requests a sandboxed kimi pane
- **THEN** AoE SHALL fail before session minting
- **AND** SHALL NOT create a shell pane as a fallback

### Requirement: Kimi control plane input and output are bounded and redacted

AoE SHALL bound every kimi server and xats control plane interaction with an explicit deadline and response size limit, and SHALL keep the identity key and bearer token out of every log and diagnostic.

#### Scenario: Secrets never reach a diagnostic
- **WHEN** any kimi server or xats control plane call fails
- **THEN** the reported diagnostic SHALL contain neither the identity key nor any bearer token

#### Scenario: Oversized responses are rejected
- **WHEN** a registry entry or control plane response exceeds its size limit with or without a declared length
- **THEN** AoE SHALL reject it before unbounded buffering
