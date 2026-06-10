## Context

Users launch Claude for cross-agent-teams (xats) work via a `free-xats-claude`
shell alias:

```sh
claude --dangerously-skip-permissions \
  --dangerously-load-development-channels server:cross-agent-teams-channel
# with CROSS_AGENT_TEAMS_MCP_TOKEN=xats exported in the shell
```

Two of these three pieces already exist in AoE:

- `--dangerously-skip-permissions` is the claude YOLO Mode flag
  (`agents.rs` `YoloMode::CliFlag`).
- The command-building path `Instance::build_agent_command` is shared by initial
  launch and `R` restart, so any persisted flag is re-applied on restart for free.

What is missing: the dev-channels flag, a way to opt in from New Session, and
auto-confirmation of Claude's startup screens.

Empirically verified during exploration:

- Launching with `--dangerously-load-development-channels` always shows a
  "WARNING: Loading development channels" screen whose default-highlighted option
  is "1. I am using this for local development"; confirmation is a single Enter.
- In untrusted directories a "Quick safety check" / "trust this folder" screen
  also appears first, default-highlighted "1. Yes, I trust this folder"; also a
  single Enter.
- A tmux pane created by a process that has `CROSS_AGENT_TEAMS_MCP_TOKEN` in its
  environment inherits the value, so AoE does not need to inject it.

## Goals / Non-Goals

**Goals:**
- One-click "Cross Agent Team" launch for claude from New Session, independent of
  YOLO Mode.
- Auto-confirm Claude's startup screens after launch and after `R` restart.
- Persist the setting and reuse the single command-build path so restart works
  with no extra wiring.
- Configurable channel string + default toggle, editable globally and per profile.

**Non-Goals:**
- Support for tools other than `claude` (codex `free-xats-codex` uses a different
  app-server / pre-register flow; out of scope here).
- Sandbox support (the dev-channels server is local-only; the option is disabled
  when Sandbox is on).
- Injecting or managing `CROSS_AGENT_TEAMS_MCP_TOKEN` (inherited from environment).
- Registering the AoE session itself into xats (this only configures the launched
  claude pane).

## Decisions

### D1: Persist a single `cross_agent_team: bool` on `Instance`
Add a serde-persisted `cross_agent_team` field (mirrors `yolo_mode`). Command
building reads it in `build_agent_command`; because that path is shared by launch
and restart, the flag survives `R` automatically. Alternative (store only in the
dialog / regenerate from config) rejected: it would not survive restart and would
duplicate logic.

### D2: Append the flag only on the claude host-launch path
In `build_agent_command`, on the non-sandboxed claude branch, when
`cross_agent_team` is set, append `--dangerously-load-development-channels <channel>`
after the base command (and after the YOLO flag if present). The sandboxed branch
never appends it (option is disabled for sandbox). Channel comes from resolved
config.

### D3: Cross Agent Team is a separate focusable field on the YOLO row
Render the checkbox on the same row as YOLO Mode, to its right, but as its own
focusable field placed immediately after `yolo_mode_field` in tab order. Toggle
with Space/Enter, highlight label when focused. This honors "to the right of YOLO
Mode" visually while keeping the one-field-per-focus model intact. The field is
only present (counted in indices) when tool is `claude` and Sandbox is off, via a
`has_cross_agent_team_field()` helper analogous to `has_yolo_field()`. All
downstream field-index calculations in `render.rs` and `handle_key` shift
accordingly.

### D4: Auto-confirm via a detached background thread (IMPLEMENTED)
Original plan was a `PendingAutoConfirm` state machine ticked by the TUI loop
(mirroring `PendingResume`). Implementation revealed a blocker: attaching to a
managed tmux session suspends the AoE TUI loop, so a loop-driven tick cannot run
during the exact window the confirmation screens appear. Instead, `spawn_auto_confirm`
launches a detached `std::thread` that polls `capture_pane` and sends Enter when a
recognized marker ("Loading development channels" / "I am using this for local
development" / "Quick safety check" / "trust this folder") is on screen. tmux
`send-keys` works regardless of attach state, so this keeps working while the user
is attached. The thread sends only while a marker is present (so it never injects
Enter into the live Claude prompt), answers both screens in sequence, and exits
when no marker has been seen for a short grace period after a send, or on a 20s
timeout. Markers and the single-Enter keystroke are justified by the exploration
findings.

### D5: Trigger auto-confirm on all (re)launch paths
`spawn_auto_confirm` is called inside the two pane-launch chokepoints:
`start_with_size_opts` (initial launch and single-pane kill-and-recreate restart)
and `respawn_agent_pane_with_resume` (multi-pane respawn and graceful-resume
restart). Both read `is_cross_agent_team()` so the thread is only spawned for
cross_agent_team claude sessions; all other launches are unaffected.

### D6: Config fields with profile override merge
Add `cross_agent_team_channel: String` (default
`server:cross-agent-teams-channel`) and `cross_agent_team_default: bool` (default
false) to the session config, plus `*Override` fields in `profile_config.rs` with
`merge_configs` logic, and settings-TUI wiring (`FieldKey`, `build_*_fields`,
`apply_field_to_global`, `apply_field_to_profile`, `clear_profile_override`). This
follows the mandatory settings checklist in AGENTS.md and the `yolo_mode_default`
precedent.

## Risks / Trade-offs

- [Token not in AoE environment] → If AoE is launched from a context without
  `CROSS_AGENT_TEAMS_MCP_TOKEN` exported, the pane won't authenticate. Mitigation:
  document that AoE must be started from a shell that exports it (the user's zshrc
  already does); do not silently inject a value.
- [Claude UI/prompt text changes] → The auto-confirm markers are matched against
  Claude's screen text, which could change across Claude versions. Mitigation:
  match on stable substrings, keep markers in one constant, and fail safe (timeout
  leaves the pane interactive rather than erroring).
- [Field-index regressions] → Inserting a new focusable field can break the
  carefully indexed `render.rs`/`handle_key` mapping. Mitigation: gate via a single
  `has_cross_agent_team_field()` helper and add dialog tests covering focus order
  with the field present and absent.
- [Double Enter into Claude] → Over-eager confirmation could send Enter into the
  live Claude prompt. Mitigation: stop polling as soon as the normal UI is detected
  and only send Enter while a recognized confirmation marker is on screen.

## Open Questions

- None blocking. Exact Claude-UI "ready" detection marker for ending the
  auto-confirm loop will be finalized during implementation against the existing
  status-detection helpers.
