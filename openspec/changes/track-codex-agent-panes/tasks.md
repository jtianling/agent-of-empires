## 1. Name the Session-Id Source Per Agent

- [ ] 1.1 Give the registry a way to say where an agent's native session id comes from, and record that Claude's is hook stdin `session_id` and Codex's is `$CODEX_THREAD_ID`.
- [ ] 1.2 Read the id through that declaration in `__record-pane`, so an agent with no declared source writes no row rather than falling back to another agent's source.
- [ ] 1.3 Keep the working-directory chain (stdin `cwd`, then `$PWD`) working for both.

## 2. Declare the Settings Format

- [ ] 2.1 Add the settings format to `AgentHookConfig` and set it on every existing entry.
- [ ] 2.2 Dispatch the installer on the declared format rather than on the path.

## 3. Install Into TOML Without Rewriting It

- [ ] 3.1 Install AoE's Codex hook entries by merging into `~/.codex/config.toml`, preserving every unrelated key, section, and value.
- [ ] 3.2 Replace AoE's own entries on reinstall instead of appending, so repeated installs converge.
- [ ] 3.3 Uninstall only AoE's entries, and leave the file in place when anything else remains.
- [ ] 3.4 Write nothing when Codex is not a detected agent.

## 4. Report the Trust Step

- [ ] 4.1 Tell the user at install time that Codex will ask them to trust the hook once before it runs.
- [ ] 4.2 Confirm no code path passes `--dangerously-bypass-hook-trust`.

## 5. Coverage

- [ ] 5.1 Cover a Codex capture end to end: a Codex pane firing a hook event produces a `pane_live` row whose `native_session_id` is `$CODEX_THREAD_ID` and whose agent is `codex`.
- [ ] 5.2 Cover that an agent whose declared source yields nothing writes no row and does not borrow another agent's source.
- [ ] 5.3 Cover TOML merge fidelity against a file holding unrelated configuration: install, reinstall, uninstall, each asserting the unrelated content byte for byte.
- [ ] 5.4 Cover that a Codex pane reaches a durable slot recording `codex`, which is the behavior the whole change exists for.

## 6. Verification

- [ ] 6.1 Run `cargo fmt`, `cargo clippy`, `cargo test --lib`, and `cargo test --test e2e` with all tmux-touching tests confined to the project harness's private socket.
- [ ] 6.2 Run `openspec validate track-codex-agent-panes --strict`.
- [ ] 6.3 Verify on the real machine that installing does not disturb the existing `notify` entry or any `[projects."..."]` section in `~/.codex/config.toml`, with a backup taken first.
