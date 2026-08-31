# Proposal: codex-update-prompt-defense

## Why

On 2026-08-31 a Shift+C fresh restart of a two-codex session silently produced two bare
shells: codex 0.150.0 blocked on its interactive startup update menu ("Update available",
default item "1. Update now (runs `npm install -g @openai/codex`)"), the xats daemon's
codex-recovery injection landed Enter on that menu, codex exited to run the npm update,
and the pane-died hook dropped both panes into plain `/bin/zsh` with no visible error.
The trigger recurs on every AoE codex launch path (start, restart, cold-start recovery)
whenever the installed codex lags npm latest, and any Enter reaching the pane (xats poke,
recovery injection, a user keystroke) turns the menu into an agent-killing update. AoE
needs two independent defenses: never let the menu appear in managed panes, and never
let a dead agent pane masquerade as a healthy session.

## What Changes

- AoE-built codex launch commands gain a per-invocation config override
  `--config check_for_update_on_startup=false` (key verified against openai/codex
  source; gate lives in `codex-rs/tui/src/updates.rs`), so managed codex panes never
  show the blocking startup update menu. The override rides the command line only;
  nothing under `~/.codex/` is written, per the existing agent-registry constraint.
- Status detection learns to recognize a fallen agent pane: when a tracked pane's
  recorded agent is not `shell` but the pane is actually running a plain shell (the
  pane-died hook's fallback), the instance is surfaced as an error state in the TUI
  with a readable message instead of being displayed as a normal running session.

## Capabilities

### New Capabilities

<!-- none -->

### Modified Capabilities

- `agent-registry`: the codex launch command MUST carry the startup-update-check
  suppression override on every AoE-managed launch path, without writing codex user
  configuration.
- `status-detection`: a tracked agent pane observed running a shell while its slot
  records a non-shell agent MUST be reported as a dead agent (error state with
  message), not as a normal working/waiting session.

## Impact

- `src/session/instance.rs`: codex command construction (`build_pane_command` family)
  gains the config override for every codex launch path (primary, extra pane, resume
  fan-out, cold-start recovery).
- `src/agents.rs`: codex `AgentDef` if the override is declared there rather than in
  the command builder (design decides the single owner).
- `src/tui/status_poller.rs` / `src/session/status_detection`: shell-in-agent-pane
  detection and the error surfacing path.
- Tests: unit coverage for the codex command shape; e2e or unit coverage for the
  dead-agent-pane error state.
- No data migration; no change to stored schemas. The xats daemon's injection guard
  is out of scope (separate repository; already reported to its owners).
