## Why

Launching `claude` for cross-agent-teams (xats) work currently requires a manual
`free-xats-claude` shell alias plus answering Claude's development-channels
confirmation screen by hand every time. AoE should let a user opt into this launch
mode directly from New Session (and have it survive `R` restarts) so the xats
workflow is a first-class, one-click option.

## What Changes

- Add a **Cross Agent Team** checkbox to the New Session dialog, placed to the
  right of the YOLO Mode checkbox. It is independent of YOLO Mode.
- The option is only available for the `claude` tool, and is hidden/disabled when
  Sandbox is enabled (the development-channels server is a local-only service that
  a container cannot reach).
- When enabled, AoE appends `--dangerously-load-development-channels <channel>` to
  the launched `claude` command. The `CROSS_AGENT_TEAMS_MCP_TOKEN` value is NOT
  injected by AoE; it is inherited from the environment AoE runs in.
- After the pane launches, AoE auto-confirms Claude's startup confirmation screens
  (the "Loading development channels" warning, and the "trust this folder" prompt
  if shown) by detecting them and sending Enter, until Claude is ready or a
  timeout elapses.
- The same flag and the same auto-confirm behavior apply on `R` restart, for both
  the graceful-resume and the kill-and-recreate restart paths.
- The development-channels channel string is configurable in the settings TUI
  (default `server:cross-agent-teams-channel`), with a session-default toggle.

## Capabilities

### New Capabilities
- `cross-agent-team`: Opt-in Claude launch mode that loads xats development
  channels and auto-confirms Claude's startup confirmation screens, available from
  New Session and preserved across restarts.

### Modified Capabilities
<!-- No existing capability's requirements change; the New Session checkbox and
     settings field are additive surfaces of the new capability. -->

## Impact

- `src/agents.rs` / `src/session/instance.rs`: command construction
  (`build_agent_command`) gains the dev-channels flag for the claude host-launch
  path; a new persisted `cross_agent_team` flag on `Instance`.
- `src/session/builder.rs` + `NewSessionData`: thread the new flag through session
  creation.
- `src/tui/dialogs/new_session/`: new checkbox, field/focus wiring, claude-only +
  non-sandbox visibility.
- `src/session/config.rs` (+ `profile_config.rs`): `cross_agent_team_channel` and
  `cross_agent_team_default` config fields with profile-override merge logic.
- `src/tui/settings/`: settings TUI fields for the new config (FieldKey,
  build_*_fields, apply_*, clear_profile_override).
- Auto-confirm state machine (modeled on the existing `PendingResume` flow) in
  `src/session/instance.rs`, triggered on initial launch and on `R` restart.
