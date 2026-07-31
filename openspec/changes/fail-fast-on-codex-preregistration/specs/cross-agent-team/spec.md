## MODIFIED Requirements

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
