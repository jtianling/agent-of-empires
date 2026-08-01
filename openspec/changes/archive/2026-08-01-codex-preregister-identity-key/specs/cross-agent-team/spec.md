## MODIFIED Requirements

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
