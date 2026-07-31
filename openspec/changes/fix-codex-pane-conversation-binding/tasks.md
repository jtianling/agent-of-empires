## 1. Re-claim when the capture predates the pane's process

- [x] 1.1 In `maybe_claim_for_pane`, resolve the pane's pid, its Codex evidence, and its process start time before consulting `pane_live`, so the gate can compare the two.
- [x] 1.2 Replace the "no row at all" gate with `capture_is_superseded`, carrying the existing `LAUNCH_SLACK_SECS` margin so ambiguity resolves to "not superseded".
- [x] 1.3 Exempt a resumed pane. `R` respawns it, so its capture necessarily predates the new process while naming exactly the conversation it is running -- and `codex resume <token>` carries that conversation's id on the command line, which is direct evidence where the timestamps are circumstantial. Without this the change would break resume: the pane would be judged superseded and could be re-bound to a stranger's conversation. `process_tree_runs_codex` is generalized to `process_tree_any` so the same process listing serves both checks, and the check is deferred behind a closure so it only costs a listing when the timestamps already say otherwise.
- [x] 1.4 Log the re-claim path distinctly from the first claim, naming the conversation being replaced.

## 2. Skip non-interactive rollouts

- [x] 2.1 Replace `rollout_cwd` with `rollout_header`, reading `cwd` and `originator` from the same line.
- [x] 2.2 In `find_rollout`, skip a rollout whose originator is a known non-interactive Codex entry point (`codex_exec`). A missing originator stays eligible.
- [x] 2.3 Warn on an originator that is neither known-interactive (`codex-tui`) nor known-non-interactive, and keep it eligible.

## 3. Coverage

- [x] 3.1 Unit test: a capture older than the pane's process is superseded; one written after it is not.
- [x] 3.2 Unit test: the margin resolves a near-tie toward "not superseded". The conversation check is a closure that panics, so a near-tie decided by anything but the timestamps fails the test.
- [x] 3.3 Unit test: a resumed pane keeps a capture a day older than its process.
- [x] 3.4 Unit test: `find_rollout` skips a `codex_exec` rollout that is the earliest match and returns the interactive one behind it; a scripted run alone leaves the pane unbound.
- [x] 3.5 Unit test: an absent and an unrecognized originator both stay eligible.
- [x] 3.6 Unit test: the header parser reads both fields from one line; an empty conversation id never matches a process tree.

## 4. Verification

- [x] 4.1 `cargo fmt`, `cargo clippy --all-targets`, `cargo check --all-targets` clean. Name-filtered unit tests: 17 in `db::codex_rollout` (8 new), 49 in `db`, 194 in `session::instance`, all passing. The full suite and e2e were NOT run, because this machine hosts live AoE tmux sessions.
- [ ] 4.2 tester: run the existing RED script `discuss/xats-codex-lab/s-stale-pane-live.sh`, whose invariant is that every slot's recorded conversation equals the one running in that slot's pane. It is red today and must go green, with its phase 1 (the same assertion before any restart) still passing. Add a resume case: after `R`, the pane must keep its conversation rather than be rebound.
- [ ] 4.3 jt's real-machine acceptance.
