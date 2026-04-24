## Context

AoE supports two kinds of sessions: **agent sessions** (tool = `claude`, `codex`, `gemini`, etc.) and **shell sessions** (tool = `shell`). Shell sessions are a deliberate escape hatch: they start a plain user shell so the user can run whatever command they want inside the managed pane.

A very common workflow is: create a shell session → attach → manually launch an agent in the shell (e.g. `claude`) → work → detach back to the TUI. In this flow, the TUI status indicator stays at `?` (Unknown) forever.

Relevant code paths today:

- `src/tmux/status_detection.rs:443` `detect_terminal_status` is a stub returning `Status::Idle` for the shell tool.
- `src/session/instance.rs:1218-1228` converts that `Idle` into `Status::Unknown` whenever `has_custom_command()` is true (which is the shell case).
- `src/session/instance.rs:1262-1266` already uses `detect_agent_type_from_pane` to discover agents inside **extra (user-split) panes**, and dispatches to per-agent content detectors for those.
- `src/tmux/status_detection.rs:507` `detect_agent_type_from_pane` inspects the pane's current process / foreground process tree and returns `"claude"`, `"codex"`, `"gemini"`, `"shell"`, etc., or `None`.
- `src/tui/app.rs:812-817` is the post-attach return path — control returns here immediately after the user detaches from the tmux session back to the AoE TUI.

The existing `detect_extra_pane_statuses` already contains the full recipe we need (agent discovery on a pane → per-agent content detection) but it is scoped to index > 0 panes. The shell-primary-pane case is not covered.

## Goals / Non-Goals

**Goals:**
- Detect a manually-launched agent in the primary pane of a shell session and show its real status in the TUI.
- One detection per detach event (zero polling overhead).
- In-memory only — the session's persisted `tool` field stays `shell`.
- Reuse existing `detect_agent_type_from_pane` and the per-agent content detectors.
- Gracefully fall back to today's `?` behavior when no agent is detected, or when the detected agent has no content-based detector.

**Non-Goals:**
- Persisting the detected agent. Shell sessions are intentionally mutable; the user may swap agents at will.
- Injecting hooks into manually-launched agents. Hooks require launch-time installation; we explicitly skip that path.
- High-frequency re-detection during idle polling. One detection per detach is sufficient for the stated workflow.
- Promoting a shell session to an agent session (no tool-field rewrite, no migration).
- Changing behavior for agent sessions. They keep their existing detectors and hook paths unchanged.

## Decisions

### 1. Detection trigger: post-attach return, not status poll

Run `detect_agent_type_from_pane` on the primary pane exactly once, immediately after the user detaches back to the AoE TUI, for sessions whose tool is `shell`.

**Where:** `src/tui/app.rs::attach_session`, right after `with_raw_mode_disabled(..., tmux_session.attach())` returns (around line 812-817, before `reload`).

**Why not during status polling?** The TUI does not show status for a session the user is actively inside (they can see the pane directly). Detection only matters when control returns to the TUI. Polling-time detection would run many times per second for no additional benefit and adds load to every poll cycle for every shell session.

**Alternative considered:** trigger via a tmux hook (`client-detached`). Rejected — requires tmux option management, complicates cleanup, and the in-process post-attach point is simpler and already exists.

### 2. Storage: in-memory field on `Instance`

Add a non-serialized field to the in-memory `Instance` (or equivalent session-state struct in `src/session/instance.rs`):

```rust
pub detected_inner_agent: Option<String>,  // e.g. Some("claude"); persists only within this aoe process
```

Marked `#[serde(skip)]` (or not serialized at all) so on-disk session state is unaffected. No migration.

**Alternative considered:** persist to session state file. Rejected per the explicit requirement from the user — shell sessions are meant to be mutable and re-detection is cheap.

### 3. Status-detection dispatch: check `detected_inner_agent` before shell stub

In `update_status_with_options` (the primary-pane status path, around `src/session/instance.rs:1180-1230`):

```
if tool == "shell" && detected_inner_agent.is_some() {
    agent = detected_inner_agent.as_deref().unwrap();
    status = detect_status_from_content(content, agent, title);
} else {
    status = AGENTS[tool].detect_status(content);  // existing path, includes shell stub
}
```

Then let the existing `Status::Idle → Status::Unknown` heuristic apply **only when `detected_inner_agent` is None**. If a detected agent returned `Idle` from its content detector, that should surface as `Idle`, not be rewritten to `Unknown` — the user has a real signal.

### 4. Re-detection and clearing

Each post-attach return runs detection again and **overwrites** `detected_inner_agent`:
- `Some("claude")` → previously detected, still claude → unchanged.
- `Some("codex")` → previously claude, now codex → replaced.
- `Some("shell")` or `None` → pane is back to bare shell or untranslatable → set to `None`, TUI returns to `?`.

Between detach events, `detected_inner_agent` is **not** touched by the status poller. This matches the "stale but stable until next detach" semantic the user asked for.

### 5. Scope of reuse

The existing `detect_agent_type_from_pane` already handles the process-tree inspection (foreground child of shell). Reuse it verbatim — no new pane-discovery code.

The existing `detect_status_from_content(content, agent, title)` helper in `src/tmux/status_detection.rs` (used by `detect_extra_pane_statuses`) already dispatches to per-agent detectors. Reuse it verbatim for the primary-pane path.

## Risks / Trade-offs

- **Risk**: `detect_agent_type_from_pane` cannot see agents that are wrapped by a parent script which itself uses `exec`. → **Mitigation**: process-tree inspection already walks the foreground PID; accept the residual gap as a known limitation (fall back to `?`, same as today).
- **Risk**: A detected agent has no content-based detector (hook-only agent). → **Mitigation**: `detect_status_from_content` falls back to `Idle` for unknown tools. Combined with today's `Idle → Unknown` heuristic for `has_custom_command`, the user still sees `?` — no regression.
- **Risk**: The detection result is stale when the user exits the inner agent without detaching (e.g. `claude` quits, bash prompt returns, user keeps working in bash, then detaches). → **Mitigation**: next detach re-runs detection and clears to `None`; no state leak beyond one attach cycle.
- **Trade-off**: A tmux pane that is captured at the exact moment of detach may catch a transient state (e.g. bash prompt re-appearing for 100ms). → Accept: the next status-poll tick uses the cached `detected_inner_agent`; if it's wrong, the next detach will correct it.
- **Trade-off**: One extra `detect_agent_type_from_pane` call per detach. This involves a tmux pane-info read and potentially a `/proc`/`ps` syscall. Acceptable — it's bounded, infrequent, and not on the hot path.

## Migration Plan

No migration. Field is non-persistent and in-memory only. On aoe restart, `detected_inner_agent` starts `None` on every session; the user's next attach-detach cycle restores it.

## Open Questions

None — all decisions from the exploration session are captured above.
