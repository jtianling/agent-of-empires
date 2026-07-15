## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Codex xats pane bootstrap

When Cross Agent Team is enabled for a non-sandboxed `codex` session, AoE SHALL
launch Codex through a pane-local xats bootstrap. The bootstrap MUST pre-register
the current `TMUX_PANE` with a fresh UUID before executing Codex, then connect the
Codex TUI to the local app-server with that UUID supplied as `xats.agent_id` and
the session project path supplied as the Codex working directory.

The bootstrap SHALL NOT read, inject, print, or persist the xats authentication
token value. It SHALL rely on the already-configured local xats environment.

#### Scenario: Fresh Codex xats launch

- **WHEN** a user creates a non-sandboxed Codex session with Cross Agent Team enabled
- **THEN** the target pane is pre-registered with a fresh UUID
- **AND** Codex starts in remote mode against the local app-server
- **AND** Codex receives the project path and the same UUID as `xats.agent_id`

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
diagnostic and terminate the pane command with a non-zero status.

#### Scenario: Local app-server unavailable

- **WHEN** a Codex Cross Agent Team pane starts and the configured local app-server is unavailable
- **THEN** the pane prints an app-server availability diagnostic
- **AND** Codex is not launched without remote mode

#### Scenario: Pane pre-registration fails

- **WHEN** a Codex Cross Agent Team pane cannot pre-register its pane and UUID
- **THEN** the pane prints a pre-registration diagnostic
- **AND** Codex is not launched without a valid xats pane claim

#### Scenario: Cross Agent Team disabled preserves normal Codex

- **WHEN** a Codex session is created with Cross Agent Team disabled
- **THEN** AoE uses the existing normal Codex command path
- **AND** no xats pane bootstrap or remote app-server argument is added
