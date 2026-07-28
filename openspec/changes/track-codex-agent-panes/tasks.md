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
- [ ] 5.3 On the real machine, install and then run a Codex session with the hook trusted, and verify a `pane_live` row appears carrying the thread id. This is the only step that can answer whether a hook command Codex spawns inherits `$CODEX_THREAD_ID` (see Open Questions); reading the binary cannot. **Needs the user: the trust prompt is theirs to answer.**
- [ ] 5.4 Verify `~/.codex/config.toml` is byte-identical before and after install, since this change claims not to write it. **Same session as 5.3.**
