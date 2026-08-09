## ADDED Requirements

### Requirement: Host OpenCode uses an exact per-pane runtime

AoE SHALL launch each managed host OpenCode pane through an AoE-owned runtime that starts a loopback-only OpenCode server and attaches the pane TUI to one exact session on that server.  Each pane MUST own a distinct endpoint and MUST NOT select a session by cwd, recency or a process-global latest value.

#### Scenario: Fresh runtime creates one exact session
- **WHEN** AoE launches a host OpenCode pane without a resume session id
- **THEN** the runtime SHALL create a new session through that pane's loopback server
- **AND** SHALL attach the TUI to the returned exact session id

#### Scenario: Resume runtime loads one exact session
- **WHEN** AoE launches a host OpenCode pane with a durable resume session id
- **THEN** the runtime SHALL verify that exact id on the new server
- **AND** SHALL attach the TUI using `--session <id>`

#### Scenario: Same cwd panes never share selection
- **WHEN** two OpenCode panes use the same working directory
- **THEN** each runtime SHALL use its own durable slot session id and endpoint
- **AND** neither runtime SHALL query a latest session to choose its conversation

### Requirement: OpenCode xats recovery reserves before launch

For a Cross Agent Team OpenCode pane with an identity key, AoE SHALL persist a higher positive runtime generation and synchronously invoke the xats daemon `POST /api/runtime/opencode/reserve` control API before creating or respawning the OpenCode process.  AoE SHALL discover the daemon only from its validated pid file, connect only to its explicit loopback port, and MUST NOT require a PATH CLI.

#### Scenario: Known identity is reserved before respawn
- **WHEN** an existing OpenCode slot starts generation N
- **THEN** AoE SHALL persist N and receive `{ok: true, state: "reserved", runtime_generation: N, changed: boolean}` before replacing the old pane process

#### Scenario: Unknown identity permits first launch
- **WHEN** reserve returns `{ok: true, need_register: true, state: "unregistered"}` for a new identity key
- **THEN** AoE SHALL permit the first OpenCode launch
- **AND** SHALL NOT invent name or team data

#### Scenario: Reserve failure is fail closed
- **WHEN** reserve returns stale generation, type mismatch, protocol mismatch, daemon failure, auth failure, storage failure or invalid arguments
- **THEN** AoE SHALL report the error and SHALL NOT start the requested OpenCode runtime

#### Scenario: Daemon discovery is fail closed
- **WHEN** the xats daemon pid file is missing, malformed, refers to a dead pid or contains an invalid port
- **THEN** AoE SHALL reject the reserve or commit operation
- **AND** SHALL NOT scan ports or fall back to a PATH executable

### Requirement: Exact session delivery is committed before attach

After the OpenCode server has produced or loaded the exact session, the runtime SHALL invoke `POST /api/runtime/opencode/commit` with the same identity key, runtime generation, loopback base URL and session id before attaching the interactive TUI.  The runtime SHALL treat xats control-plane commit as delivery setup, not as proof that the OpenCode MCP connection is bound.

#### Scenario: Commit receives exact runtime tuple
- **WHEN** generation N reaches session ready
- **THEN** commit SHALL receive `(identity_key, N, base_url, session_id)` for that pane
- **AND** the identity key SHALL appear only in the strict JSON request body, never in the URL, response, log or diagnostic

#### Scenario: Partial commit retries the identical delivery
- **WHEN** commit returns `connection_bind_trigger_failed`, `opencode_unreachable` or `session_not_found`
- **THEN** AoE SHALL retry only within a bounded attempt count
- **AND** every retry SHALL use the identical generation, base URL and session id

#### Scenario: Fatal control outcome is not retried
- **WHEN** commit returns a client HTTP status, protocol mismatch, type conflict, stale generation, CAS conflict, delivery conflict, missing auth token or an unknown outcome
- **THEN** AoE SHALL fail closed immediately
- **AND** SHALL NOT submit another commit attempt

#### Scenario: Commit schedules connection recovery
- **WHEN** commit returns `{ok: true, state: "delivery_committed", delivery_committed: true, connection_bound: false, recovery_prompt: "scheduled"}`
- **THEN** the runtime SHALL allow attach to continue
- **AND** xats MAY complete connection binding through its exact-session recovery prompt and agent reconnect

#### Scenario: Commit failure stops the runtime
- **WHEN** commit cannot validate or submit the exact delivery after bounded retry
- **THEN** the runtime SHALL terminate its OpenCode server child
- **AND** SHALL exit with a diagnostic instead of attaching an unfenced TUI

### Requirement: Runtime owns child lifecycle and reserved arguments

The AoE runtime SHALL terminate the OpenCode server it created when attach exits or startup fails.  It SHALL allow only attach-supported extra arguments that do not override runtime-owned endpoint, directory, authentication or session selection.  It SHALL reject default-TUI-only and unknown arguments before server startup or slot generation advancement.

#### Scenario: Attach exit cleans the server
- **WHEN** the attached OpenCode TUI exits
- **THEN** the runtime SHALL terminate only its own server child
- **AND** SHALL return the TUI exit status

#### Scenario: Conflicting argument is rejected
- **WHEN** a managed runtime receives a user extra argument that can override endpoint or session selection
- **THEN** it SHALL fail before server startup with a diagnostic naming the conflicting option

#### Scenario: Default TUI argument is rejected before launch
- **WHEN** a managed runtime receives `--model`, `--agent`, `--prompt` or another argument unsupported by `opencode attach`
- **THEN** it SHALL fail before server startup or slot generation advancement
- **AND** SHALL NOT create a shell pane as a fallback

#### Scenario: xats REST call is bounded and authenticated
- **WHEN** a reserve or commit HTTP call does not finish within 5 seconds
- **THEN** AoE SHALL return a bounded failure diagnostic instead of blocking pane launch indefinitely
- **AND** when `CROSS_AGENT_TEAMS_MCP_TOKEN` is present, the request SHALL carry it as a bearer token

#### Scenario: xats control input and output are size bounded
- **WHEN** the daemon pid file exceeds 4 KiB or an HTTP 200 response exceeds 64 KiB with or without a Content-Length header
- **THEN** AoE SHALL reject it before unbounded parsing or buffering
- **AND** SHALL fail with a diagnostic that contains neither identity key nor bearer token

#### Scenario: Identity key is absent from the pane command
- **WHEN** AoE builds a Cross Agent Team OpenCode pane command
- **THEN** the command SHALL contain instance id, slot and generation but SHALL NOT contain the identity key
- **AND** the runtime SHALL load the exact key from the matching durable slot before injecting it only into OpenCode child environments

#### Scenario: HTTP and domain failures remain distinct
- **WHEN** the REST adapter returns a non-200 transport status
- **THEN** AoE SHALL retry only HTTP 503 and SHALL immediately fail other statuses with the HTTP status class
- **WHEN** the adapter returns HTTP 200 with a protocol mismatch or fail-closed domain outcome
- **THEN** AoE SHALL validate `{ok: false, error: "protocol_version_mismatch", cli_protocol_version: N, daemon_protocol_version: 1}` or `{ok: false, error, detail?}` and stop the runtime
- **AND** no diagnostic SHALL contain the identity key

#### Scenario: Managed host fork fails closed
- **WHEN** the user requests fork for a managed host OpenCode session
- **THEN** AoE SHALL reject the operation with an exact-session runtime fork diagnostic
- **AND** SHALL NOT select a parent session by cwd or recency

#### Scenario: Internal loopback endpoint does not inherit user auth
- **WHEN** AoE starts its temporary managed OpenCode server
- **THEN** it SHALL remove inherited OpenCode server username and password variables from the server and attach children
- **AND** SHALL bind the unauthenticated endpoint only to loopback for the paired xats exact probe
