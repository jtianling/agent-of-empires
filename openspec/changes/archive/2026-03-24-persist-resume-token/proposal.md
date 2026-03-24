## Why

When a user presses R to restart an agent whose pane is already dead, the current code skips graceful restart entirely (commit 2ffafc7) because extracting a resume token from stale pane output is unreliable -- the token may have been consumed by a manually started agent. This means agents that crash or exit on their own can never be resumed; they always get a fresh start. By persisting the resume token when the pane first dies (output is still fresh), we can offer resume restart even when the user triggers it later.

## What Changes

- Add a `resume_token: Option<String>` field to the `Instance` struct, serialized to sessions.json.
- Capture the resume token in the status poller when it first detects a pane has died (fresh output, token not yet stale).
- On restart, check for a stored resume token before attempting live extraction from pane output.
- Clear the stored resume token after it is consumed (successful restart) or when it becomes invalid (session recreated, agent changed).
- The graceful restart flow (`initiate_graceful_restart`) continues to work as-is for live panes; the stored token only serves as a fallback for already-dead panes.

## Capabilities

### New Capabilities

(none -- this change enhances existing capabilities)

### Modified Capabilities

- `agent-resume-restart`: Add requirement for persisted resume token capture and consumption, enabling resume restart from dead panes.
- `status-detection`: Add requirement that the poller captures the resume token from pane output when it detects a pane has transitioned to dead.

## Impact

- **`src/session/instance.rs`**: New `resume_token` field on `Instance`, updated `respawn_agent_pane_with_resume` to prefer stored token, updated `initiate_graceful_restart` to use stored token for dead panes.
- **`src/tui/status_poller.rs`**: `StatusUpdate` gains optional `resume_token` field; polling loop captures token on pane death transition.
- **`src/tui/app.rs`**: Apply `resume_token` from status updates to instances; pass stored token through restart flow.
- **`src/session/storage.rs`**: No direct changes needed (serde picks up the new field automatically).
- **sessions.json**: Gains `resume_token` field on serialized instances. Old files without this field deserialize safely via `Option<String>` with serde default.
