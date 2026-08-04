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

For a Cross Agent Team OpenCode pane with an identity key, AoE SHALL persist a higher positive runtime generation and synchronously invoke the paired xats `reserve-opencode-runtime` control-plane CLI before creating or respawning the OpenCode process.  The identity key MUST be read by the CLI from the named environment variable and MUST NOT appear in argv or logs.

#### Scenario: Known identity is reserved before respawn
- **WHEN** an existing OpenCode slot starts generation N
- **THEN** AoE SHALL persist N and receive a successful reserve result before replacing the old pane process

#### Scenario: Unknown identity permits first launch
- **WHEN** reserve returns the explicit `need_register` state for a new identity key
- **THEN** AoE SHALL permit the first OpenCode launch
- **AND** SHALL NOT invent name or team data

#### Scenario: Reserve failure is fail closed
- **WHEN** reserve returns stale generation, type mismatch, protocol mismatch, daemon failure, auth failure, storage failure or invalid arguments
- **THEN** AoE SHALL report the error and SHALL NOT start the requested OpenCode runtime

### Requirement: Exact session delivery is committed before attach

After the OpenCode server has produced or loaded the exact session, the runtime SHALL invoke `commit-opencode-runtime` with the same identity key, runtime generation, loopback base URL and session id before attaching the interactive TUI.  The runtime SHALL treat xats control-plane commit as delivery setup, not as proof that the OpenCode MCP connection is bound.

#### Scenario: Commit receives exact runtime tuple
- **WHEN** generation N reaches session ready
- **THEN** commit SHALL receive `(identity_key, N, base_url, session_id)` for that pane
- **AND** the identity key SHALL reach the CLI only through its environment

#### Scenario: Commit schedules connection recovery
- **WHEN** commit succeeds for a known identity
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

#### Scenario: Paired xats CLI is bounded
- **WHEN** a reserve or commit control-plane child does not exit within 5 seconds
- **OR** its direct child exits but a background descendant keeps stdout or stderr open
- **OR** an output reader cannot be created
- **THEN** AoE SHALL terminate the entire owned process group
- **AND** SHALL use a bounded cleanup window to reap the direct child and output readers
- **AND** SHALL keep the control-plane child stdin closed
- **AND** SHALL return a bounded failure diagnostic instead of blocking pane launch indefinitely

#### Scenario: Managed host fork fails closed
- **WHEN** the user requests fork for a managed host OpenCode session
- **THEN** AoE SHALL reject the operation with an exact-session runtime fork diagnostic
- **AND** SHALL NOT select a parent session by cwd or recency

#### Scenario: Internal loopback endpoint does not inherit user auth
- **WHEN** AoE starts its temporary managed OpenCode server
- **THEN** it SHALL remove inherited OpenCode server username and password variables from the server and attach children
- **AND** SHALL bind the unauthenticated endpoint only to loopback for the paired xats exact probe
