# cross-agent-team Specification

## Purpose
TBD - created by archiving change cross-agent-team-launch. Update Purpose after archive.
## Requirements
### Requirement: Cross Agent Team launch option in New Session

The New Session dialog SHALL present a "Cross Agent Team" checkbox positioned to
the right of the YOLO Mode checkbox. The option SHALL be independent of YOLO Mode:
toggling one MUST NOT change the other.

The option SHALL only be available when the selected tool is `claude`, and MUST be
hidden or disabled when Sandbox is enabled.

The checkbox's initial state SHALL be taken from the `cross_agent_team_default`
configuration value (resolved through the active profile).

#### Scenario: Option visible for claude without sandbox

- **WHEN** the user opens New Session with tool `claude` and Sandbox not enabled
- **THEN** a "Cross Agent Team" checkbox is shown to the right of YOLO Mode
- **AND** it can be toggled independently of YOLO Mode

#### Scenario: Option hidden for non-claude tools

- **WHEN** the user selects a tool other than `claude` in New Session
- **THEN** the Cross Agent Team checkbox is not shown

#### Scenario: Option disabled when sandbox enabled

- **WHEN** the user enables Sandbox in New Session with tool `claude`
- **THEN** the Cross Agent Team checkbox is hidden or non-selectable
- **AND** the session is not launched with the development-channels flag

#### Scenario: Default state from configuration

- **WHEN** `cross_agent_team_default` is true for the active profile
- **AND** the user opens New Session with tool `claude` and Sandbox not enabled
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

The Cross Agent Team setting SHALL persist with the session. On `R` restart, AoE
SHALL rebuild the launch command with the development-channels flag and SHALL run
the auto-confirm flow again, for both the graceful-resume and the
kill-and-recreate restart paths.

#### Scenario: Graceful resume re-applies flag and auto-confirm

- **WHEN** a Cross Agent Team session is restarted via `R` along the graceful-resume
  path
- **THEN** the resumed command includes `--dangerously-load-development-channels`
- **AND** AoE auto-confirms the startup screens again

#### Scenario: Kill-and-recreate re-applies flag and auto-confirm

- **WHEN** a Cross Agent Team session is restarted via `R` along the
  kill-and-recreate path
- **THEN** the recreated command includes `--dangerously-load-development-channels`
- **AND** AoE auto-confirms the startup screens again

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

