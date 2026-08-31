# Design: codex-update-prompt-defense

## Context

Codex shows a blocking interactive update menu at startup whenever the installed
version lags npm latest (`codex-rs/tui/src/updates.rs`, gated by the config key
`check_for_update_on_startup`). In a managed pane that menu is fatal: any Enter that
reaches the pane (xats codex-recovery injection, a poke, a stray keystroke) selects
the default "Update now", codex exits to run `npm install -g`, and the pane-died hook
(`src/tmux/utils.rs::pane_died_hook_command`) drops the pane into a bare `/bin/zsh`.
The TUI keeps showing the session as healthy; the 2026-08-31 incident took both codex
panes of a session this way with zero visible error.

Current state:

- Every AoE codex launch path funnels through `build_base_pane_command`
  (`src/session/instance.rs:3045`): primary launch, extra panes, resume fan-out,
  cold-start recovery, and the sandbox branch all decorate its output. The codex
  xats bootstrap wraps the final codex invocation, so flags added to the base
  command survive the wrap.
- `AgentDef` (`src/agents.rs`) already carries agent-specific launch knowledge
  (`yolo`, `instruction_flag: Some("--config developer_instructions={}")`), so the
  `--config key=value` style is an established codex pattern.
- The batch pane query (`src/tmux/mod.rs:239`) already fetches
  `#{pane_current_command}` for every pane, and `agent_slot` rows record each
  tracked pane's agent. The incident showed slot rows survive the fallback (the
  shell pane keeps its `%id`), so "slot says codex, pane runs zsh" is observable
  from data AoE already collects.
- Constraint from `agent-registry`: AoE SHALL NOT write anything under `~/.codex/`,
  so the suppression must ride the command line, never the config file.

## Goals / Non-Goals

**Goals:**

- No AoE-managed codex pane ever shows the startup update menu.
- A tracked agent pane that has fallen back to a plain shell is surfaced as an
  error state with a readable message, on every path that displays instance status.

**Non-Goals:**

- Fixing the xats daemon's injection guard (separate repository; reported).
- Auto-answering or dismissing the update menu (suppression makes this moot).
- Auto-respawning fallen panes (visibility only; the user restarts explicitly and
  the persisted slots/identity keys already make that restart correct).
- Managing codex updates for the user (they update on their own schedule).

## Decisions

### D1: Suppression flag declared on `AgentDef`, applied in `build_base_pane_command`

Add a static field to `AgentDef` (e.g. `fixed_args: &'static [&'static str]`,
empty for every agent except codex, which carries
`--config check_for_update_on_startup=false`) and append it in
`build_base_pane_command` right after the binary/model/resume decoration.

- Why not hardcode codex in the command builder: agents.rs is the repo's
  established source of truth for per-agent launch knowledge (cf. `sets_own_title`,
  `yolo`, `instruction_flag`); a codex `if` in instance.rs scatters that.
- Why `build_base_pane_command`: it is the single funnel every launch path shares
  (host, sandbox, resume fan-out, recovery, extra panes), so one insertion point
  covers all of them, including the xats bootstrap wrap which surrounds the
  finished command.
- Command override wins unchanged: an instance with `has_command_override()`
  replaces the base command verbatim today and MUST keep doing so; fixed args
  apply only to commands AoE itself builds.
- Long-form `--config` over `-c` for consistency with `instruction_flag`.

### D2: Shell-fallback detection joins slot agent x live pane command in the poller

The status poller compares, for each tracked pane it already queries, the recorded
agent (`agent_slot.agent`, or the primary pane's tool when no slot row exists)
against `#{pane_current_command}`. When the recorded agent is not `shell` (per the
existing `pane_agent_is_shell` / `is_shell_command` helpers) but the live command is
a shell, the pane is a fallen agent: the instance status becomes `Error` with a
message naming the pane(s), e.g. "agent exited; pane %9 dropped to shell (restart
with r/R or c/C)".

- Why the poller: it already owns status transitions and already fetches
  `pane_current_command` in the batch query, so detection costs no new tmux
  round-trips.
- Why not a pane-died hook marker: writing state from tmux hooks adds a second
  writer with its own lifecycle and version sensitivities; the poller derives the
  same fact from data it already has.

### D3: Guards against false positives

- **Launch window**: the wrapper (`zsh -lc 'stty ...; exec codex ...'`) legitimately
  reports a shell until `exec` replaces the image. Detection is suppressed while
  the instance is `Starting` and within the existing start grace window
  (`last_start_time`), reusing the spinner-grace-period convention rather than
  inventing a new timer.
- **Legitimate shells**: instances with `expects_shell()`, shell slots, and
  instances whose command override names a shell (the `--cmd-override sh` e2e
  fixture pattern) are excluded: for them a shell in the pane is correct.
- **One-way latch until relaunch**: the error clears on the next
  restart/start of the instance (the paths that already reset `last_error`),
  not by the poller flapping the state.

## Risks / Trade-offs

- [Upstream renames `check_for_update_on_startup`] → The flag becomes an unknown
  config key. Implementation MUST verify codex's unknown-key behavior with the
  pinned local version (expected: tolerated, config is open TOML); the e2e/manual
  check in tasks covers it. If codex ever hard-errors on unknown keys, the field
  is one registry line to update.
- [Detection misses non-slot panes] → A pane with no slot row and no primary
  marker is invisible to D2. Accepted: such panes are also invisible to
  restart/recovery today; the capture chain is the existing answer.
- [Very old codex without `--config`] → Out of support; the user tracks recent
  versions, and the same versions are what show the update menu at all.
- [False negative when the fallback shell runs a non-shell child] → The pane
  reports the child's command while it runs, hiding the fallen state. Accepted:
  the user is then actively using that pane, and the error would be noise.

## Migration Plan

No data migration. Purely additive behavior:

1. Land the registry field + command builder change (D1); existing sessions pick
   it up on their next restart.
2. Land detection (D2/D3); the poller starts flagging fallen panes immediately,
   including any that fell before the upgrade.

Rollback is a revert; no stored state depends on either feature.

## Open Questions

- None blocking. The exact error-message wording and whether extra fallen panes
  beyond the first are listed individually can be settled in implementation.
