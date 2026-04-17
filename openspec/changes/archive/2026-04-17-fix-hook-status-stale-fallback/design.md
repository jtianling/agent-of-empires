## Context

Hook-based status detection was introduced to give Claude Code (and Cursor) sub-second, event-accurate state without having to parse pane content. Agents' hook scripts write `running`/`waiting`/`idle` to `/tmp/aoe-hooks/<instance_id>/status` on events like `UserPromptSubmit`, `PreToolUse`, `Notification`, and `Stop`. AoE reads this file on every poll and short-circuits all other detection layers when it finds a value.

The short-circuit has no freshness check. The pipeline relies on the hook emitting `Stop` (which writes `idle`) to close out a turn. In practice, several real-world paths skip `Stop`:

- User presses Esc to interrupt a streaming response.
- User types a client-side slash command (`/plugin`, `/compact`, `/reload-plugins`, etc.) that never enters the model turn lifecycle.
- Agent process crashes or is killed before sending `Stop`.

Once the last event in a turn was `UserPromptSubmit`/`PreToolUse`, the file stays at `running` indefinitely. We observed a live session where the file mtime was 3.5 hours old, content `running`, and every pane was at an idle prompt. AoE still displayed Running.

The existing `openspec/specs/status-detection/spec.md` already states "the hook status file exists **and is fresh**" in the scenario that describes hook short-circuiting, but nothing in the implementation enforces freshness. This change closes that gap.

## Goals / Non-Goals

**Goals:**
- Stale hook files MUST NOT pin a session's displayed status indefinitely.
- Fresh hook files MUST continue to short-circuit (preserve current performance and event-accuracy on hot paths).
- Behavior MUST be identical on the TUI status poller and the notification monitor (both read `read_hook_status` today).
- Fallback path MUST reuse the existing content-based detection; no new detection code paths.

**Non-Goals:**
- We do NOT rewrite or delete the on-disk hook file from AoE. Hook ownership stays with the agent.
- We do NOT add a user-configurable freshness window. The constant lives in code, can be tuned later if we have evidence.
- We do NOT change the hook script side (the shell snippets in `.claude/settings.json`).
- We do NOT add new hook events for Claude Code. If Claude adds more events upstream, we can register them separately.
- We do NOT touch non-Claude hook-based agents beyond ensuring the generic mechanism applies uniformly.

## Decisions

### Decision 1: Freshness window = 30 seconds

Rationale:
- A Claude turn typically has a `PreToolUse` or `UserPromptSubmit` event every few seconds while working (every tool call fires a hook). 30s of silence strongly suggests the turn has ended without a `Stop`.
- For long-running single tool calls (e.g. a multi-minute `Bash(cargo build)`), we still get a `PreToolUse` at the start which refreshes mtime; during the Bash itself there are no more events, so after 30s the hook will be marked stale and we fall through to content detection. Content detection for Claude sees the spinner ⠋ or tool-use output → still Running. No regression.
- If content detection is wrong (unlikely for Claude since we have robust spinner + tool-use heuristics), we'd wrongly flip to Idle for the remainder of the long Bash. This is strictly better than the current "stuck Running for hours" bug and matches the behavior non-hook agents already live with.

Alternatives considered:
- **5 seconds**: too aggressive; normal UI re-renders or hook flushing jitter could look stale.
- **5 minutes**: matches the Claude API cache TTL superstition but is too slow to repair the bug we care about.
- **Configurable**: premature generality. Start with a hard-coded constant; revisit if users complain.

### Decision 2: Expose mtime alongside status, not a pre-baked boolean

`read_hook_status` currently returns `Option<Status>`. We change it to also return the file mtime (or introduce a sibling `read_hook_status_with_mtime` / a small struct) so the caller owns the freshness decision.

Rationale:
- The freshness check lives naturally in the status pipeline code (where we have access to `Instant::now()` equivalents). Keeping it out of the low-level reader means the reader stays dumb and test-friendly.
- Notification monitor and TUI poller can share the same freshness constant and gating logic in one place (likely via a helper function in the hooks module that returns `Option<(Status, SystemTime)>` plus a `is_fresh(mtime)` helper).

Alternatives considered:
- Return `Option<Status>` and silently drop stale → loses information and makes tests brittle (need to assert on absence for both "no file" and "stale file").
- Separate `read_hook_status_fresh()` wrapper → cleaner call site but hides the mtime, making debug logging harder. We'd still want the mtime for tracing.

### Decision 3: Do not touch the hook file on disk

Rationale:
- AoE did not create the file. Changing it from AoE would conflict with a later hook write and is a layering violation.
- Deleting the file on staleness is tempting but introduces TOCTOU risk: the agent could be mid-write.
- The file is small; reading + stat is cheap. We pay the cost every poll anyway.

### Decision 4: Apply freshness gating symmetrically to the notification monitor

`src/tmux/notification_monitor.rs` has its own `read_hook_status` call path (the shared status-detection spec already says "Notification monitor uses shared detection pipeline"). We route both sites through the same freshness-aware helper so the monitor and TUI never diverge. Without this, a session could show Idle in the TUI but still trigger notifications as Running, or vice versa.

## Risks / Trade-offs

- **Long-running single tool without intermediate events**: e.g. a 10-minute `Bash`. After 30s the hook is stale; we fall through to content detection. For Claude Code this is fine -- the spinner and `Bash(…)` tool-use marker are in the last 10 lines and content detection returns Running. → Mitigation: already covered by Decision 1; add an e2e-ish unit test that simulates "fresh PreToolUse then 31s quiet with spinner still visible" and expect Running.
- **Clock skew / mtime anomalies**: if `/tmp` is on a filesystem with wonky mtime (e.g. some container overlays) we could see "future" mtimes. Freshness check uses `SystemTime::now().duration_since(mtime)` which returns an Err for future mtimes. → Mitigation: on `Err` (future mtime), treat as fresh (best-effort; the next poll will correct as soon as mtime is in the past).
- **First-read race at agent start**: when the agent just started, the hook file may not yet exist. This is already handled (read returns None). No change.
- **Poller cycle timing** vs. freshness window: poller runs with adaptive tiers; Running sessions poll every cycle (~1s), Idle every 5 cycles (~5s). 30s freshness window is comfortably larger than any polling interval so a stale-recovery will happen within at most one additional content-detection cycle.

## Migration Plan

No migration required. The change is purely in read-path logic; existing hook files on disk continue to work as-is.

Rollback: revert the change. No persisted state is modified.

## Open Questions

- Should the freshness window be different for `waiting` vs `running`? A `waiting` status typically means the agent is showing a permission prompt -- those can sit for a long time waiting for user input, and content detection would correctly see the prompt and also return Waiting. So the same 30s window is fine; if the user takes >30s to answer, we fall through and content detection keeps returning Waiting. Answer: no difference needed.
- Should we log at trace-level when fallback triggers, to aid debugging? Yes -- add `tracing::debug!("hook stale, falling through to content detection", instance_id, age_secs)`.
