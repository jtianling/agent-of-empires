## MODIFIED Requirements

### Requirement: Codex xats pane bootstrap

When Cross Agent Team is enabled for a non-sandboxed `codex` session, AoE SHALL
launch Codex through a pane-local xats bootstrap. The bootstrap MUST pre-register
the current `TMUX_PANE` with a fresh UUID before executing Codex, then connect the
Codex TUI to the configured local app-server with that UUID supplied as
`xats.agent_id` and the session project path supplied as the Codex working
directory.

The configured local app-server endpoint SHALL be read from the
`CROSS_AGENT_TEAMS_CODEX_WS_URL` environment variable, which is the same variable
xats resolves its own Codex endpoint from, and SHALL default to
`ws://127.0.0.1:8799` when that variable is unset. When only
`CROSS_AGENT_TEAMS_CODEX_WS_URLS` is set, AoE SHALL use its sole entry if it has
exactly one, because AoE commits to an endpoint before any thread exists and
cannot probe several the way xats does. The availability gate and the Codex
remote argument SHALL both derive from that single endpoint, so they cannot name
different servers.

AoE SHALL accept the endpoints xats accepts: any URL whose scheme is `ws` or
`wss`, with any path preserved. AoE SHALL NOT be stricter than xats about which
endpoints are legitimate, because a value xats uses and AoE refuses puts the two
on different servers. The URL SHALL be carried as written rather than
re-serialized, so an authority-only URL does not acquire a trailing path.

The host AoE interpolates into the generated pane command SHALL be restricted to
ASCII alphanumerics, `.`, `-`, and `:`, and SHALL be shell-escaped. This
constrains what reaches a generated shell command and is independent of which
endpoints are accepted.

The bootstrap SHALL NOT read, inject, print, or persist the xats authentication
token value. It SHALL rely on the already-configured local xats environment.

#### Scenario: Fresh Codex xats launch

- **WHEN** a user creates a non-sandboxed Codex session with Cross Agent Team enabled
- **THEN** the target pane is pre-registered with a fresh UUID
- **AND** Codex starts in remote mode against the configured local app-server
- **AND** Codex receives the project path and the same UUID as `xats.agent_id`

#### Scenario: Configured endpoint replaces the default

- **WHEN** `CROSS_AGENT_TEAMS_CODEX_WS_URL` is set to `ws://localhost:8899`
- **AND** a Cross Agent Team Codex pane command is built
- **THEN** the Codex remote argument names `ws://localhost:8899`
- **AND** the availability gate probes port `8899` on `localhost`
- **AND** no part of the default endpoint appears in the command

#### Scenario: Unset endpoint keeps the default

- **WHEN** `CROSS_AGENT_TEAMS_CODEX_WS_URL` is unset
- **AND** a Cross Agent Team Codex pane command is built
- **THEN** the Codex remote argument names `ws://127.0.0.1:8799`
- **AND** the availability gate probes port `8799` on `127.0.0.1`

#### Scenario: A secure or path-bearing endpoint is accepted

- **WHEN** `CROSS_AGENT_TEAMS_CODEX_WS_URL` names a `wss` endpoint, or a `ws` endpoint carrying a path
- **THEN** AoE accepts it, because xats accepts it
- **AND** the availability gate probes the endpoint's host and port, ignoring the path
- **AND** the path is preserved in the Codex remote argument

#### Scenario: Single-entry multi-endpoint form is honored

- **WHEN** `CROSS_AGENT_TEAMS_CODEX_WS_URLS` holds exactly one endpoint and `CROSS_AGENT_TEAMS_CODEX_WS_URL` is unset
- **THEN** AoE uses that endpoint
- **AND** the default endpoint appears nowhere in the command

#### Scenario: Ambiguous multi-endpoint form aborts rather than guessing

- **WHEN** `CROSS_AGENT_TEAMS_CODEX_WS_URLS` holds more than one endpoint and `CROSS_AGENT_TEAMS_CODEX_WS_URL` is unset
- **THEN** the pane command reports that AoE needs a single endpoint and names `CROSS_AGENT_TEAMS_CODEX_WS_URL`
- **AND** the pane command terminates with a non-zero status
- **AND** Codex is not launched against the default endpoint

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
- **AND** the Codex fork command connects to the configured local app-server
- **AND** the parent native session id is preserved as the fork source

### Requirement: Codex xats bootstrap failure is explicit

When the user requests a Codex Cross Agent Team session, AoE MUST NOT silently
fall back to a normal local Codex launch, nor to a different app-server than the
one the user configured. Missing pane identity, UUID generation, local
app-server availability, xats pre-registration, or an app-server endpoint AoE
will not accept SHALL produce a specific diagnostic and terminate the pane
command with a non-zero status.

Substituting the default endpoint for a rejected one is a prohibited silent
fallback. Its symptom appears on the xats side, as a Codex that connected but
cannot be resumed, so a diagnostic AoE only writes to its own log does not reach
the person debugging it.

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

#### Scenario: Cross Agent Team disabled preserves normal Codex

- **WHEN** a Codex session is created with Cross Agent Team disabled
- **THEN** AoE uses the existing normal Codex command path
- **AND** no xats pane bootstrap or remote app-server argument is added
