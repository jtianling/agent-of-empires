## Context

`maybe_claim_for_pane` is the only thing that ever writes a Codex pane's `pane_live` row, because Codex installs no status hook. Its gate is:

```rust
match store.read_pane_live(pane_id) {
    Ok(None) => {}
    _ => return,
}
```

Read literally: claim once, never again. That was correct while a pane's conversation could not change under a stable pane id. `Shift+C` on a live session breaks that assumption -- it respawns in place (`respawn-pane -k -t <pane>`), so the pane id survives while the conversation does not. (`Shift+C` on a session whose tmux is gone takes the other branch, rebuilds the session, and gets fresh pane ids, so it has no stale row and is unaffected.)

`find_rollout` supplies the other half of the binding. Its evidence is timestamp, `cwd`, and not-already-claimed. A rollout carries no pane, pid, or tty, and under `--remote` the app-server writes it, so there is no stronger signal available to reach for.

## Goals / Non-Goals

**Goals:**

- A pane's recorded conversation describes the process currently in that pane.
- Conversations that could not have been running in a pane are not eligible to be bound to one.
- A `codex` release that changes `originator` is visible rather than silent.

**Non-Goals:**

- Making the pane-to-conversation link hard. There is no such link available; this change makes a heuristic match the right set of candidates, and does not pretend to more.
- Serializing AoE's own Codex pane launches to remove the remaining simultaneous-start ambiguity. Considered and dropped: it costs session startup latency, and once the two defects here are fixed the residual case is a genuine simultaneous start, which is far rarer than either of them.
- Any change to slot assignment, restart, or recovery.

## Decisions

### 1. A `pane_live` row that predates its pane's process is stale

The claim gate becomes: no row, or a row written before the process now in the pane started. Both mean the recorded conversation cannot describe what is running.

This requires the pane's pid and start time before the gate rather than after it, so the checks are reordered. The positive evidence requirements are unchanged and still run first in spirit: a pane whose process tree is not invoking Codex is rejected before any of this matters.

The comparison carries the same `LAUNCH_SLACK_SECS` margin the rollout match uses, applied so that ambiguity resolves to "not stale". `process_start_unix` derives from `ps` elapsed seconds against the current clock, so it is good to about a second; treating a fresh row as stale would send a correctly bound pane looking for another conversation, while treating a stale row as fresh merely defers the re-claim to the next reconcile tick. In practice the two values are tens of seconds apart in either direction -- a live pane's row is written well after its process started, and a stale row predates the respawn entirely -- so the margin is never load-bearing, only a guard.

### 1a. A resumed pane is exempt, on positive evidence rather than on timing

Decision 1 as stated would break resume. `R` respawns the pane too, so a resumed pane's row also predates its process -- while naming exactly the conversation that process was launched to continue. Judged on timestamps alone it would be re-claimed, and since its own rollout is older than the respawn it could not match itself; it would take some other unclaimed conversation instead. The module's previous "claim once" rule was what made resume safe, so removing that rule has to put something in its place.

`codex resume <token>` carries the conversation's id on the command line, and the pane's process tree is already being read to prove it is running Codex. So the exemption is direct evidence -- this pane is running that conversation -- against the circumstantial evidence of two timestamps, and direct evidence wins.

`process_tree_runs_codex` is generalized to a shared `process_tree_any` walk so both checks read the same listing, and the conversation check is deferred behind a closure: it costs a process listing and only matters once the timestamps have already said the row is old.

### 2. Filter on `originator`, as a blacklist rather than a whitelist

A rollout written by `codex exec` carries `originator: "codex_exec"`; a pane's TUI carries `codex-tui` (observed with `source` of both `vscode` and `cli`, so `source` is not the discriminator). Skipping the known non-interactive originator removes the entire class of a pane binding to a scripted run.

A whitelist of `codex-tui` would be stronger, and it is the wrong trade. If a future Codex renames the TUI originator, a whitelist stops every pane from ever being adopted -- no slots, so no restart coverage and no recovery -- and that failure looks exactly like the bugs this change exists to fix. A blacklist that misses a newly introduced non-interactive originator merely leaves today's behavior in place for it. One direction risks a silent total regression, the other risks not having improved.

An originator that is neither known-interactive nor known-non-interactive is accepted and warned about. The warning is the signal that the assumption behind this filter has moved; making it change behavior is what a whitelist would do.

### 3. Parse `originator` alongside `cwd`, from the same line

The rollout header is one JSON line and is already read and capped at 4 MiB for `cwd`. Reading a second field costs nothing and avoids opening the file twice.

## Risks / Trade-offs

- **Two Codex panes that genuinely start within the rollout slack can still take each other's conversation.** -> Not mitigated, and now the only remaining case rather than one of three. Both conversations survive and a user who notices can restart the pane. Removing it means serializing launches, which is a Non-Goal above.
- **A pane bound before this change keeps its stale row until its next restart.** -> Accepted; the row is corrected the first time the pane is respawned, which is exactly when it was going wrong before.
- **The blacklist is a list of one.** -> Intended. It is extended when another non-interactive originator is observed, and the warning in Decision 2 is what surfaces the need.
