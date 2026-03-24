## Context

AoE supports graceful agent restart with session resumption for agents like Claude and Codex. The current flow sends exit keys, waits for the pane to die, captures pane output, extracts a resume token via regex, and respawns with that token. This works when the user triggers restart while the agent is still alive.

However, when an agent exits on its own (crash, user typed `/exit`, etc.), the pane becomes dead before the user presses R. Commit 2ffafc7 added an early return in `initiate_graceful_restart()` to skip graceful restart for dead panes, because by restart time the pane output may be stale or the token consumed by a manual agent start. This means agents that exit on their own always get a fresh restart, losing conversation context.

The status poller already detects pane death as part of its regular polling loop. At the moment of first death detection, the pane output is still fresh and the resume token is valid. This is the ideal capture point.

## Goals / Non-Goals

**Goals:**

- Enable resume restart for agents whose panes died before the user pressed R.
- Capture the resume token at the earliest reliable moment (status poller detecting pane death).
- Persist the token to sessions.json so it survives AoE restarts.
- Maintain full backwards compatibility with existing sessions.json files.

**Non-Goals:**

- Changing the live graceful restart flow (exit keys, tick-driven state machine). That path continues to work as-is for live panes.
- Implementing `--print-session-id` capture-on-start pattern (agent-deck style). Too invasive for the current startup flow.
- Syncing resume tokens to tmux environment variables. Adds complexity without benefit since we already persist Instance data.
- Auto-restarting agents that die. The user still decides when to restart.

## Decisions

### Decision 1: Store resume token as `Option<String>` on Instance

**Choice**: Add `resume_token: Option<String>` to the `Instance` struct with `#[serde(default, skip_serializing_if = "Option::is_none")]`.

**Rationale**: Instance is already serialized to sessions.json with serde. Using `Option<String>` with serde default means old session files without this field deserialize without error. The token is naturally associated with a specific instance and should live next to it.

**Alternative considered**: Store in a separate file or tmux environment variable. Both add complexity and coupling without benefit.

### Decision 2: Capture token in StatusUpdate, apply in TUI event loop

**Choice**: Extend `StatusUpdate` with an optional `resume_token: Option<String>` field. The status poller captures the token when it detects the pane has transitioned from alive to dead. The TUI event loop (`app.rs`) applies the token to the Instance when processing status updates.

**Rationale**: The status poller runs on a background thread and only communicates via `StatusUpdate` messages. Adding the token to this existing message type keeps the architecture clean. The poller already captures pane content for status detection, so extracting the resume token is a small addition.

**Alternative considered**: Capture directly in `update_status()` on Instance. This would work but the Instance in the poller is a clone, so the token would need to travel back through `StatusUpdate` anyway.

### Decision 3: Track previous status to detect alive-to-dead transition

**Choice**: The status poller maintains a `HashMap<String, Status>` of previous statuses. A resume token is only captured when the previous status was not `Error` and the current status is `Error` due to pane death. This ensures we only capture on the first death transition, not on every poll of an already-dead pane.

**Rationale**: Pane output is only guaranteed fresh on the first detection of death. Subsequent polls of a dead pane may capture output from a manually started replacement process, or the pane may have been respawned.

### Decision 4: Stored token consumed on restart, cleared on fresh start

**Choice**: When `respawn_agent_pane_with_resume` is called, it checks for a stored `resume_token` if no live-extracted token is provided. After consumption (or fresh start), the stored token is cleared.

**Rationale**: A resume token is single-use. Once the agent is restarted with it, the token is consumed server-side and cannot be reused. Clearing prevents stale token accumulation.

### Decision 5: Dead-pane restart path uses stored token

**Choice**: Modify `initiate_graceful_restart()` so that when the pane is already dead AND a stored resume token exists, it directly calls `respawn_agent_pane_with_resume` with the stored token instead of returning `false`.

**Rationale**: This is the key behavioral change. Currently dead panes always fall back to fresh restart. With a stored token, we can skip the exit-key/wait/capture cycle entirely and go straight to respawn.

## Risks / Trade-offs

- **[Stale token]** If the user manually starts an agent in the same pane between death detection and pressing R, the stored token may be consumed. -> Mitigation: Clear stored token when the pane is respawned or when a new `Starting` status is detected.

- **[Token validity window]** Resume tokens may expire server-side after some time. -> Mitigation: This is the same risk as the current live-extraction path. If the token is rejected, the agent starts fresh anyway. No additional mitigation needed.

- **[Disk write frequency]** Storing the token triggers a sessions.json save. -> Mitigation: sessions.json is already saved on every status change in the TUI. The additional token field is negligible.

- **[Polling race]** If the poller misses the alive-to-dead transition (e.g., AoE restarts between polls), no token is captured. -> Mitigation: Acceptable. The user gets a fresh restart, which is the current behavior. The stored token is a best-effort improvement.
