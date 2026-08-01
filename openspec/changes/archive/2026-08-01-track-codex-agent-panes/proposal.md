## Why

Every pane AoE has ever tracked is a Claude pane. Not by choice -- by construction.

Pane tracking is driven entirely by the agent: the agent's own status hook shells out to `aoe __record-pane`, which writes the `pane_live` row that the reconciler later snapshots into a durable `agent_slot`. An agent that installs no hook produces no row, is never adopted into a slot, and therefore does not exist as far as recovery is concerned. `codex` is such an agent: its registry entry carries `hook_config: None`.

The consequence is not that Codex panes recover badly. It is that they recover as something else. A pane that once ran Claude and was later handed to Codex keeps its stale `claude` record forever, because nothing Codex does can correct it; recovery then faithfully relaunches Claude into a pane whose user was running Codex. On the machine that reported this, `agent_slot` held seventeen rows and `pane_live` eight, every one of them `claude`, on a machine that runs Codex daily.

Codex 0.145 has a hooks system that is close to structurally identical to Claude's, so the reason this gap persists is no longer a missing upstream capability.

## What Changes

- Give `codex` a hook configuration so its panes report themselves, the way every other tracked agent's panes do.
- Read a capture's native session id from the agent's own source rather than assuming one shape. Claude supplies it as `session_id` in the hook's stdin JSON; Codex exports `$CODEX_THREAD_ID` into the pane's environment. The requirement becomes "the agent's session id", with the source named per agent.
- Install those hooks into `~/.codex/hooks.json`, the dedicated hooks file Codex reads beside `config.toml`, in the same JSON shape the existing installer already writes. The user's `config.toml` is not written at all.
- Surface Codex's hook trust gate rather than working around it. A newly installed hook does not run until the user trusts it once, and `--dangerously-bypass-hook-trust` is not an acceptable way to avoid that.

## Capabilities

### Modified Capabilities

- `pane-session-capture`: a capture's native session id comes from the source its agent provides, not from hook stdin alone; Codex panes are captured.
- `agent-registry`: `codex` carries a hook configuration.

## Impact

- `src/agents.rs` for the Codex hook configuration and its event set.
- `src/cli/record_pane.rs` for the per-agent session-id source.
- `~/.codex/hooks.json` on the user's machine, a file AoE creates and owns. `~/.codex/config.toml` is not touched.
- `src/hooks/mod.rs` is expected to need no format work: Codex reads the same `{"hooks": {"<Event>": [{"hooks": [{"type": "command", ...}]}]}}` shape the installer already produces.
- Once Codex panes can occupy slots, an adopted slot whose agent is not the instance's tool becomes reachable in practice. Cross Agent Team decoration and adopted-slot identity keys were fixed ahead of this change (`c29ed8be`); Decision 5 records what is left.
