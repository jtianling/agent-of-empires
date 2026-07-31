## Why

A Codex pane has no hook, so AoE learns which conversation it is running by matching Codex's rollout files. The match has exactly three pieces of evidence: the rollout's timestamp is at or after the pane process started (less a 2s slack), its recorded `cwd` equals the instance's project path, and its thread is not already claimed -- earliest wins. Nothing in a rollout names a pane: `session_meta` records `session_id`, `cwd`, `originator`, `cli_version`, `source`, `thread_source`, `model_provider`, `base_instructions`, `history_mode`, `context_window`, and `git`, and in `--remote` mode the file is written by the app-server, so not even an open file descriptor links it to the pane.

Two defects make that already-thin evidence produce wrong answers.

**A restarted pane never re-matches.** `maybe_claim_for_pane` only claims when the pane has no `pane_live` row at all. `pane_live` rows are deleted only when a session is purged or when a pane leaves every managed session -- neither happens on `Shift+C` for a live session, which respawns in place and keeps the pane id. So after every such restart the pane keeps a row describing the conversation it was running *before*, and its new conversation is never claimed. The slot then holds a conversation that is not in that pane, which is what a later `R` would try to resume. With a second Codex pane present it is worse: the new conversation sits unclaimed, and the sibling -- whose own process started long ago -- matches it on time and cwd and takes it. Measured in the lab: a sibling took the primary pane's post-restart conversation across a 15 minute gap, not the 2 second slack the design anticipated.

**Non-interactive Codex runs compete for the same match.** `codex exec` invoked from a script in the same repository writes a rollout with the same `cwd`. AoE does not look at `originator`, so a pane can be bound to a conversation that was never in any pane. This is live today: of the rollouts written on one day on this machine, four were `codex_exec` runs in a repository that also hosts Codex panes.

## What Changes

- Re-claim when the pane's `pane_live` row predates the process currently running in that pane, so a restarted pane picks up its new conversation instead of keeping the old one.
- Skip rollouts whose `originator` marks them as a non-interactive Codex run, so a pane is never bound to a `codex exec` conversation.
- Warn once per unrecognized `originator`, so a value neither known-interactive nor known-non-interactive is visible rather than silently accepted.

## Capabilities

### Modified Capabilities

- `pane-session-capture`: a Codex pane's conversation binding follows the process currently in the pane rather than the first one ever seen there, and only conversations that could have run in a pane are eligible.

## Impact

- `src/db/codex_rollout.rs`: `maybe_claim_for_pane`, `find_rollout`, and the rollout header parser.
- No change to the claim's per-pane evidence requirements, to slot assignment, or to any launch path.
