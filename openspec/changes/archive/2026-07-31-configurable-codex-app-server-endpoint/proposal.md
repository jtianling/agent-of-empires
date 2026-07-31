## Why

The `cross-agent-team` spec says a Codex pane connects to "the **configured** local app-server" in five separate scenarios. Nothing is configured: `src/session/instance.rs` hardcodes `CODEX_XATS_APP_SERVER_HOST/PORT/URL` to `127.0.0.1` and `8799`.

xats itself does not treat 8799 as a constant. Its `register_agent` resolves the Codex endpoint by an explicit precedence chain -- an explicit `ws_url`, then `CROSS_AGENT_TEAMS_CODEX_WS_URL`, then `CROSS_AGENT_TEAMS_CODEX_WS_URLS`, and only then `ws://127.0.0.1:8799` (`src/mcp/tools.ts:1601`). A user who moved their app-server off the default port therefore gets a silent split: xats registers against the endpoint they configured, while AoE launches Codex against 8799. The pane either fails the `nc -z` gate with a diagnostic that names a port the user never chose, or connects to whatever else is listening there.

Hardcoding also makes the endpoint untestable in isolation. A test fixture cannot point AoE at a private app-server, so an isolated Codex pane connects to the developer's real one -- executing tools in that process, against the real `CODEX_HOME`, writing rollouts into the real session store.

## What Changes

- Resolve the Codex app-server endpoint from `CROSS_AGENT_TEAMS_CODEX_WS_URL`, falling back to `ws://127.0.0.1:8799`. This is xats's own variable, not a new AoE-specific one.
- Derive the `nc -z` availability gate's host and port from that one endpoint, so the gate and the launch argument cannot disagree.
- Reject a malformed value with a warning and fall back to the default, rather than splicing it into the generated shell script.
- Warn when only `CROSS_AGENT_TEAMS_CODEX_WS_URLS` (the multi-endpoint form) is set, because AoE launches against exactly one endpoint and would otherwise silently use the default.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `cross-agent-team`: the app-server endpoint the Codex bootstrap connects to and gates on becomes configurable through xats's own environment variable. The spec already describes it as "configured"; this makes that true.

## Impact

- `src/session/instance.rs`: `CODEX_XATS_APP_SERVER_*` constants and `codex_xats_bootstrap_command`.
- No change to any other launch path, to the pre-registration protocol, or to xats.
