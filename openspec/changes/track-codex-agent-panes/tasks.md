## 1. Name the Session-Id Source Per Agent

- [x] 1.1 Give the registry a way to say where an agent's native session id comes from, and record that Claude's is hook stdin `session_id` and Codex's is `$CODEX_THREAD_ID`.
- [x] 1.2 Read the id through that declaration in `__record-pane`, so an agent with no declared source writes no row rather than falling back to another agent's source.
- [x] 1.3 Keep the working-directory chain (stdin `cwd`, then `$PWD`) working for both.

## 2. Give Codex a Hook Configuration

- [x] 2.1 Add a `hook_config` to the `codex` registry entry targeting `.codex/hooks.json`.
- [x] 2.2 Map Codex's events to AoE statuses: `PreToolUse` and `UserPromptSubmit` to running, `Stop` to idle, `PermissionRequest` to waiting. Codex has no `Notification` or `ElicitationResult` event.
- [x] 2.3 Confirm the existing JSON installer needs no change: same `{"hooks": {...}}` shape, same merge-and-preserve behavior, only a different path.

## 3. Report the Trust Step

- [x] 3.1 Tell the user at install time that Codex will ask them to trust the hook once before it runs. Documented in `docs/guides/configuration.md`, and added to `HooksInstallDialog`. **The dialog is currently dead code** -- it is exported but never constructed, so hooks install silently on session start for every agent. That is a pre-existing gap, not one this change introduced, and the documentation is what actually reaches the user today.
- [x] 3.2 Confirm no code path passes `--dangerously-bypass-hook-trust`.

## 4. Coverage

- [x] 4.1 Cover a Codex capture end to end: a Codex pane firing a hook event produces a `pane_live` row whose `native_session_id` is `$CODEX_THREAD_ID` and whose agent is `codex`.
- [x] 4.2 Cover that an agent whose declared source yields nothing writes no row and does not borrow another agent's source.
- [x] 4.3 Cover that a user's own entries in `~/.codex/hooks.json` survive install, reinstall, and uninstall.
- [x] 4.4 Cover that a Codex pane reaches a durable slot recording `codex`, which is the behavior the whole change exists for.

## 5. Verification

- [x] 5.1 Run `cargo fmt`, `cargo clippy`, `cargo test --lib`, and `cargo test --test e2e` with all tmux-touching tests confined to the project harness's private socket.
- [x] 5.2 Run `openspec validate track-codex-agent-panes --strict`.
- [x] 5.3 On the real machine, install and then run a Codex session with the hook trusted, and verify a `pane_live` row appears carrying the thread id. This is the only step that can answer whether a hook command Codex spawns inherits `$CODEX_THREAD_ID` (see Open Questions); reading the binary cannot. **Needs the user: the trust prompt is theirs to answer.** Run on 2026-07-30: the hooks fire and the thread id is inherited and correct, but the row carried the app-server's pane rather than the agent's. That is section 6.
- [x] 5.4 Verify `~/.codex/config.toml` gains no AoE hook entry. **Same session as 5.3.** Not a whole-file comparison: Codex rewrites that file on its own -- observed on 2026-07-29, plugin paths and a `last_updated` stamp changing with no AoE involvement -- so a hash check reports a difference whatever AoE does. The invariant is that no `[hooks]` section and no `aoe-hooks` marker ever appears in it. Held: zero `aoe-hooks` markers, and the only hook content Codex added was its own `[hooks.state]` trust hashes.

## 6. Carry the Pane Into Codex's Hooks (Decision 6)

- [x] 6.1 Add the pane variable a hook reads when `$TMUX_PANE` cannot be trusted, and prefer it over `$TMUX_PANE` in `__record-pane`.
- [x] 6.2 Gate the hook's capture branch on either pane variable.
- [x] 6.3 Give a Codex launch `shell_environment_policy.set` overrides for its pane and instance id, expanding `$TMUX_PANE` in the pane's own shell. Not applied to a command override, which is the user's own program.
- [x] 6.4 Cover that a Codex launch carries both overrides and that an agent whose hooks run in its own process does not.
- [x] 6.5 On the real machine, confirm the hook reaches the status file. It does: a Codex session AoE launched wrote `/tmp/aoe-hooks/<instance>/status` on its first turn. That is what disproved "the hook never runs" and, by elimination, located the failure in the session-id source (section 7).
- [ ] 6.6 Delete the `pane_live` row 5.3 wrote against the unrelated shell pane.

## 7. Correct the Session-Id Source (Decision 1, revised)

- [x] 7.1 Move Codex's declared source to hook stdin, now that a live session has shown `$CODEX_THREAD_ID` is absent from hook environments.
- [x] 7.2 Rewrite the coverage that encoded the disproved premise.
- [x] 7.3 On the real machine, confirm a Codex pane finally produces a `pane_live` row. It did -- and the row carried the app-server daemon's pane despite the section-6 overrides on the client's command line, which is what falsified that remedy (Decision 7). The `SessionIdSource` enum was retired along with it: with Codex out of the hook registry every remaining agent reads stdin, so the source is no longer a per-agent property.

## 8. Replace Codex Hooks with Rollout Binding (Decision 7)

- [x] 8.1 Drop the `codex` hook configuration from the registry; nothing is written under `~/.codex/` anymore, and the trust step disappears with it.
- [x] 8.2 Bind a Codex instance's primary pane to the earliest unclaimed rollout created after the pane's process started in the instance's project directory, from the reconciler tick, writing the `pane_live` row the snapshot path already consumes.
- [x] 8.3 Guard the capture subcommand against a `$TMUX_PANE` that names a pane whose root process is not among its ancestors, so a stale hook in a shared daemon skips rather than claims another session's pane. Positive-only: an unanswerable check is accepted.
- [x] 8.4 Coverage: rollout parsing and claim ordering (unit); a Codex pane reaching a durable slot through a rollout file with no hook involved, a foreign pane refused, and an own-pane capture accepted (e2e). Test helpers that simulate hooks from outside a pane point `$TMUX_TMPDIR` at an existing empty directory -- tmux silently falls back to the real default socket when the directory does not exist.
- [ ] 8.5 On the real machine: install, restart the TUI, start a Codex session, and confirm its pane and slot rows carry its own pane id and thread id.
- [ ] 8.6 Clean up the poisoned rows the hook era left behind (`pane_live` and `agent_slot` rows claiming the daemon's panes), and remove `~/.codex/hooks.json`, which nothing reads on AoE's behalf anymore.
