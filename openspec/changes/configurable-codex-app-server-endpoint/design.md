## Context

`codex_xats_bootstrap_command` builds a `sh -c` script that gates on the app-server being reachable and then execs Codex against it. It reads three separate constants:

```rust
const CODEX_XATS_APP_SERVER_HOST: &str = "127.0.0.1";
const CODEX_XATS_APP_SERVER_PORT: &str = "8799";
const CODEX_XATS_APP_SERVER_URL: &str = "ws://127.0.0.1:8799";
```

The gate uses the first two, the `--remote` argument uses the third. They agree only because someone kept them in sync by hand.

The same file already has the pattern this change needs: `RECOVERY_SETTLE_ENV` reads an environment variable, validates it, warns on a value it will not honor, and falls back to a compiled-in default.

## Goals / Non-Goals

**Goals:**

- One endpoint value, with the gate's host and port derived from it.
- Honor the endpoint xats itself honors, on the same terms xats honors it, so the two cannot end up configured apart.
- A value AoE will not take stops the pane with a diagnostic instead of reaching the generated shell script or being quietly replaced.

**Non-Goals:**

- A settings-TUI field. This mirrors a xats deployment choice that lives in the user's shell environment alongside `CROSS_AGENT_TEAMS_MCP_HOME` and `CROSS_AGENT_TEAMS_MCP_TOKEN`; it is not per-session state, and `AGENTS.md`'s "every configurable field must be editable in the settings TUI" governs `SandboxConfig`-style session config, not process environment.
- Probing several endpoints the way xats does. See Decision 3.
- Any change to what the bootstrap does once connected.

## Decisions

### 1. Reuse `CROSS_AGENT_TEAMS_CODEX_WS_URL` rather than adding an AoE-specific variable

xats resolves the Codex endpoint from this variable already. A separate `AGENT_OF_EMPIRES_CODEX_APP_SERVER_URL` would let the two be set to different values, which reintroduces the exact split this change exists to close -- only now with the user believing they had configured it.

The cost is that AoE takes a dependency on a xats variable name. That is the correct direction of coupling: the endpoint is xats's to define, and AoE is the caller.

### 2. Match xats's acceptance set, and keep the injection guard separate from it

The first draft of this change accepted exactly `ws://<host>:<port>`. Checking xats's implementation showed that is stricter than xats on two axes: `normalizedWebSocketUrl` parses with `new URL()` and admits both `ws:` and `wss:`, preserving any path, and the single-variable form is not validated at all (it is trimmed and used).

Being stricter than xats is not a safe direction. A value xats takes and AoE refuses puts the two on different servers while the user believes they configured one -- which is the same silent split this resolution exists to remove, entering through a different door. So AoE parses with the `url` crate and accepts any URL whose scheme is `ws` or `wss`, path and all.

That leaves the real problem the strictness was aimed at: the host is interpolated into a generated `sh -c` script (`nc -z {host} {port}`), so an unchecked value is a shell injection into a command AoE runs on the user's machine. The character check stays, but it now applies to the host the parser extracted rather than to the raw string, and it is an injection guard rather than a statement about which endpoints are legitimate. Keeping the two questions apart is what lets the acceptance set follow xats without loosening the guard. The host is also passed through `shell_escape`, so neither mechanism is load-bearing alone.

The URL is carried as the user wrote it, not as `Url` re-serializes it. A round trip appends a trailing slash to an authority-only URL, which would silently change the `--remote` argument AoE has always passed.

### 3. A single-element `CROSS_AGENT_TEAMS_CODEX_WS_URLS` is honored; an ambiguous one aborts

xats accepts a JSON array of endpoints and probes them to find the one holding a given `thread_id`. AoE cannot do that: it commits to an endpoint at launch, before any thread exists.

But a one-element array leaves nothing to pick between, and a user whose array holds only the endpoint they run is a working configuration today. So AoE honors that case and refuses the rest, rather than treating the whole variable as unreadable.

An array with several endpoints is not guessed at. AoE reports that it needs one endpoint and points at the single-URL variable.

### 4. A rejected value aborts the pane; it does not fall back to the default

Falling back to the default on a bad value looked like the safe failure direction. It is the opposite. The user who set the variable is the one whose value gets discarded, xats keeps using it, and the two end up on different servers -- with AoE's warning in AoE's debug log while the symptom (a Codex that connected but cannot be resumed) shows up on the xats side, where nobody is reading AoE's warnings.

So a rejected value produces a pane command that prints the diagnostic and exits non-zero, which is how every other Cross Agent Team precondition in this file already fails, and what the existing "Codex xats bootstrap failure is explicit" requirement already demands: AoE must not silently substitute a different launch for the one that was asked for. A pane that refuses to start is loud. Two systems quietly talking to different servers is not.

### 5. Resolve once per command build, not once per process

The resolution runs inside `codex_xats_bootstrap_command`. A `OnceLock` would cache the first value for the life of the process, which is wrong for tests that set the variable per case, and buys nothing -- the function already builds a multi-line shell script on every call.

## Risks / Trade-offs

- **A user with a stale `CROSS_AGENT_TEAMS_CODEX_WS_URL` in their environment now changes AoE's behavior where it previously did not.** -> This is the intended correction, and it aligns AoE with what xats was already doing with that variable. It is a behavior change for anyone whose environment sets the variable to something other than the default.
- **A typo in the variable now stops the pane from starting, where before this change the variable was ignored entirely.** -> Accepted, and chosen deliberately over the alternative in Decision 4. The diagnostic names the variable and the value, and it appears in the pane the user is looking at.
- **AoE's acceptance set tracks xats's, so a future loosening on the xats side silently reopens a gap.** -> Not mitigated in code. The pairing is recorded here and in the spec; the injection guard is written to stay correct regardless, because it constrains the extracted host rather than the accepted URL shape.
- **A multi-endpoint array that used to be ignored (AoE quietly using the default) now aborts unless it holds exactly one endpoint.** -> Intended. The old behavior was the silent split; if the default was the right endpoint it was a coincidence.
