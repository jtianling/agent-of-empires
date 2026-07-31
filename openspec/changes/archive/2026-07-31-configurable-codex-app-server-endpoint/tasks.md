## 1. Resolve the endpoint from one source

- [x] 1.1 Replace the three `CODEX_XATS_APP_SERVER_*` constants with a single default URL constant and a `CodexAppServerEndpoint { url, host, port }` value carrying all three, so the gate and the `--remote` argument cannot name different servers.
- [x] 1.2 Parse with the `url` crate and accept any `ws`/`wss` URL, path included, matching what xats's `normalizedWebSocketUrl` accepts. Carry the URL as written rather than re-serialized, so an authority-only URL does not gain a trailing slash. Store an IPv6 host unbracketed, which is the form `nc` takes.
- [x] 1.3 Keep the character check on the host the parser extracted, as an injection guard on what reaches the generated `sh -c` script, separate from the question of which endpoints are acceptable.
- [x] 1.4 Resolver: read `CROSS_AGENT_TEAMS_CODEX_WS_URL`; failing that, accept `CROSS_AGENT_TEAMS_CODEX_WS_URLS` when it holds exactly one endpoint; failing that, the default. Return the diagnostic instead of an endpoint when a set value is not acceptable.
- [x] 1.5 A rejected value produces a pane command that prints the diagnostic and exits non-zero, rather than falling back to the default. This is the same shape every other Cross Agent Team precondition in this file already uses.
- [x] 1.6 Make the app-server availability diagnostic a function of the endpoint. It was a constant with the default URL baked into its text, so a configured endpoint would have been probed while the failure message named a different server.
- [x] 1.7 Split the bootstrap builder into a resolving wrapper and `codex_xats_bootstrap_command_for`, which takes the endpoint. Tests assert on a specific endpoint without setting a process-global variable that every other test building a Codex command would also observe.
- [x] 1.8 Add `url` to `Cargo.toml`. It was already in the tree through `reqwest`, so this adds no new compilation.

## 2. Coverage

- [x] 2.1 Parser accepts: the default, an alternate host and port, an IPv6 literal, a `wss` endpoint, a path-bearing endpoint, a scheme's default port, and a value with surrounding whitespace.
- [x] 2.2 Parser rejects: a non-websocket scheme, `file://`, a bare `host:port`, an empty authority, an empty host, a non-numeric port, a port above `u16`, the empty string, and three hosts carrying shell metacharacters.
- [x] 2.3 A configured endpoint is named in the remote argument, the gate host, the gate port, and the failure diagnostic. Uses a host and port that both differ from the default, so neither part of the default can survive unnoticed.
- [x] 2.4 Resolver: default when unset, follows a valid value, returns a diagnostic naming the variable and the value when rejected, honors a single-entry list, refuses a multi-entry one by pointing at the single-URL variable, and prefers the single-URL variable over a list. The resolver takes the two values as arguments, with a thin env-reading wrapper over it, so no test mutates the process environment -- a `#[serial]` test that set the variable would still change what every concurrently running test building a Codex command observed.
- [x] 2.5 Unit test: the bootstrap `exec`s into Codex rather than wrapping it. This is a contract with xats, whose carrier folding takes the process group leader and only finds Codex there because `exec` puts it at the shell's pid. A comment cannot stop that edit.
- [x] 2.5 An aborted pane command carries the diagnostic and `exit 1`, and contains neither `--remote` nor the default endpoint.
- [x] 2.6 `tests/e2e/harness.rs`: every `aoe` spawn site clears `CROSS_AGENT_TEAMS_CODEX_WS_URL`. The e2e Codex assertions name the default endpoint, and this change is what would otherwise let a developer's shell environment decide whether they pass. Placed before `extra_env` so a test can still set it deliberately.

## 3. Verification

- [x] 3.1 `cargo fmt`, `cargo clippy --all-targets`, `cargo check --all-targets` clean. Unit tests run name-filtered only (193 pass in `session::instance::tests`); the full suite and e2e are NOT run, because this machine hosts live AoE tmux sessions.
- [x] 3.2 tester: lab fixture pointed at its private app-server through `CROSS_AGENT_TEAMS_CODEX_WS_URL`. Both panes' actual argv carry `--remote ws://127.0.0.1:8898`, the `nc -z` gate follows, and the run makes no contact with the production app-server. The lab guard also asserts the endpoint is `ws`/`wss`, is not `:8799`, and has something listening, so a misconfiguration cannot be mistaken for another failure.
- [x] 3.3 Accepted by jt.
