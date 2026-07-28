## Why

Recovering a session with adopted agents gives back the wrong thing, and says nothing about it.

A user split a pane and started an agent in each half by hand, so the instance's own tool stayed a shell while both slots recorded `claude`. Recovering it produced a single pane running one of the two agents; the other was gone, the left/right layout was flattened, and the TUI reported success.

Two separate defects.

The relaunch command for slot 0 is built from the instance's tool rather than from the agent the slot actually recorded. For a session whose primary pane was adopted, that means the recovered pane runs a shell where an agent used to be. Non-primary panes already build from their own recorded agent, which is why exactly one of the two came back correctly. The test helper's own comment documents this behavior and works around it by requiring the instance tool to match slot 0's agent, so no test could catch it.

Recovery also reports only the failures it happens to observe while launching. A pane that is created, relaunched, and then disappears is not one of them, so recovery returns success while quietly handing back fewer panes than the user had. Silence is the worst part: the user is left comparing what they see against what they remember.

## What Changes

- Build a recovered or restarted pane's command from the agent its slot recorded. Instance-level launch concepts (command override, pre-allocated session id, pending fork, extra args) apply only when that agent is the instance's own tool, since they describe that agent and nothing else.
- Verify, once the panes have been launched and the layout applied, that every durable slot still has a live pane, and report the ones that do not as per-pane failures.
- Cover both with a session whose tool is a shell and whose slots record a different agent, the shape the existing helper deliberately avoids.

## Capabilities

### Modified Capabilities

- `multi-agent-session`: an adopted slot relaunches as the agent it recorded, not as the instance's tool.
- `session-layout-recovery`: recovery reports slots that did not come back instead of returning success.

## Impact

- `src/session/instance.rs` for per-pane command construction and the post-recovery verification.
- `tests/e2e/cold_start_recovery.rs` and `tests/e2e/harness.rs` for a shell-tool instance with adopted agent slots.
- Does not address the `pane-died` hook running without an explicit target, or the loss of adopted panes to kill-then-respawn. Both are tracked separately; this change makes the second one visible rather than silent.
