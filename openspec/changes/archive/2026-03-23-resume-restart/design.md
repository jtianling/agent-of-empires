## Context

AoE manages agent processes inside tmux panes. The R key restarts an agent by killing its process tree and respawning the pane with a freshly built command. This loses all conversation context.

Agents like Claude Code and Codex print a resume token on exit (e.g., `claude --resume <uuid>`, `codex resume <uuid>`). Today users must manually copy this token to resume. This design automates that capture-and-resume cycle as a generic mechanism.

Current restart flow: `kill_agent_pane_process_tree()` -> `respawn_agent_pane(cmd)`.
The agent pane already has `remain-on-exit on`, so panes survive process exit.

## Goals / Non-Goals

**Goals:**
- Automate graceful exit -> capture resume token -> restart with token for agents that support it
- Keep the TUI responsive during the graceful exit wait (no blocking)
- Generic mechanism: any agent can opt in by declaring a `ResumeConfig`
- Reliable fallback: any failure reverts to current kill-and-fresh behavior
- Configure Claude and Codex as the first two agents with resume support

**Non-Goals:**
- Resuming custom commands (`aoe add --cmd "..."`) -- these bypass the mechanism entirely
- Sandboxed (Docker) resume -- can be added later but not in this change
- Adding a separate "force fresh restart" keybinding -- user can start fresh manually
- Persisting resume tokens across AoE restarts -- the token is ephemeral, used once

## Decisions

### D1: Graceful exit via tmux send-keys, parse pane output

**Choice**: Send exit keys to the agent pane, wait for the process to die, then capture and parse pane output for the resume token.

**Alternatives considered**:
- `--continue` flag (resumes most recent session): unreliable when multiple agents share the same project directory within one AoE session
- Read agent state files from filesystem: fragile, depends on internal storage format that can change between versions
- Inject a wrapper script that captures output: over-engineered, same result achievable with tmux capture-pane

**Rationale**: The pane output approach uses only stable public interfaces (the agent's documented CLI flags and its visible terminal output). tmux `capture-pane` and `send-keys` are battle-tested primitives already used elsewhere in AoE.

### D2: Tick-driven state machine, not blocking or background thread

**Choice**: Model the graceful restart as a state machine (`PendingResume`) on the Instance, driven by the TUI tick loop.

**Alternatives considered**:
- Blocking call in the action handler: freezes the TUI for up to 10 seconds
- Background thread with channel: adds threading complexity, needs synchronization with Instance state

**Rationale**: AoE already has a tick-based status poller that checks pane state periodically. The state machine fits naturally into this loop. Each tick advances the state (send next key batch, check if pane is dead, parse output, respawn). No new threads or channels needed.

### D3: Exit key sequence sent in steps with inter-step delays

**Choice**: `exit_sequence` is a `&[&[&str]]` -- an array of key groups. Each group is sent in one `tmux send-keys` call. Groups are sent one per tick (~200ms apart).

For Claude: `[["C-c"], ["C-c"]]` -- first Ctrl+C interrupts any running task, second Ctrl+C triggers exit.
For Codex: `[["C-c"], ["C-c"]]` -- same pattern.

**Rationale**: Agents need time to process the first signal before the second is meaningful. Sending all keys in one `send-keys` call can result in the second Ctrl+C being swallowed. One group per tick provides natural pacing via the existing tick interval.

### D4: Resume token inserted after binary in built command

**Choice**: `build_agent_command()` accepts an optional `resume_token: Option<&str>`. When present, it inserts `resume_flag.replace("{}", token)` immediately after the binary name, before extra_args and yolo flags.

Example: `claude --resume <uuid> --dangerously-skip-permissions ...`
Example: `codex resume <uuid> --dangerously-bypass-approvals-and-sandbox ...`

**Rationale**: Both Claude and Codex expect the resume argument right after the binary. Other flags (yolo, instruction) come after and remain compatible.

### D5: Custom commands skip resume entirely

**Choice**: If `instance.command` is non-empty (custom command via `--cmd`), the restart always uses the current kill-and-fresh behavior.

**Rationale**: Custom commands may already contain their own `--resume` flag or have completely different argument formats. Attempting to parse and modify arbitrary user commands is fragile and unnecessary -- the user chose a custom command deliberately.

## Risks / Trade-offs

- **[Agent output format changes]** -> The resume pattern is a regex on visible output. If Claude/Codex change their exit message format, the regex fails silently and falls back to fresh restart. Mitigation: patterns are per-agent in `ResumeConfig`, easy to update. Fallback is always safe.

- **[Agent hangs on exit]** -> Some agents may not exit cleanly after receiving Ctrl+C. Mitigation: configurable timeout (default 10s). On timeout, fall back to force kill.

- **[Race between R presses]** -> User presses R twice quickly. Mitigation: if `pending_resume` is already `Some`, ignore subsequent R presses for that instance.

- **[Pane output scrollback insufficient]** -> The resume token might scroll off the captured area if the agent prints a lot on exit. Mitigation: capture 100 lines of scrollback, which is generous for an exit message.

## Open Questions

- Should the `Restarting` status have its own visual indicator in the status bar? (Suggestion: reuse the `Starting` spinner with "Restarting..." text)
- Should the timeout be user-configurable per profile, or is a hardcoded per-agent default sufficient for now?
