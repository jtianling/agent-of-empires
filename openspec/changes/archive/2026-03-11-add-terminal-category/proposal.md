## Why

AoE currently only manages AI coding agent sessions (Claude, Codex, Gemini, etc.). Users often need plain terminal sessions alongside their agent sessions for tasks like running servers, monitoring logs, or executing manual commands. Having a "Terminal" category in the tool picker lets users manage all their tmux sessions from one place instead of switching between AoE and raw tmux.

## What Changes

- Add a new `"terminal"` entry to the `AGENTS` registry in `src/agents.rs`, positioned after `gemini` and before `cursor`.
- Terminal is always available (no binary detection needed -- the user's shell is always present).
- Terminal sessions use the existing `TerminalSession` infrastructure (`aoe_term_` prefix) but are now first-class creatable from the new session dialog.
- No YOLO mode, no instruction injection, no status detection for terminal -- it simply launches the user's default shell.
- The tool picker in the new session dialog shows "terminal" alongside agent tools.
- When "terminal" is selected, agent-specific fields (YOLO Mode, Worktree/Branch) are hidden since they don't apply.

## Capabilities

### New Capabilities
- `terminal-category`: Adds "terminal" as a selectable tool category for creating plain shell sessions managed by AoE.

### Modified Capabilities
- `agent-registry`: Add the `terminal` entry to the registry with appropriate defaults (no YOLO, no instruction flag, always available, no status detection).

## Impact

- `src/agents.rs`: New `AgentDef` entry for terminal.
- `src/tmux/status_detection.rs`: Trivial stub function (always Idle).
- `src/tmux/mod.rs`: Terminal is always available -- `AvailableTools::detect()` may need adjustment or terminal can be treated specially.
- `src/tui/dialogs/new_session/`: Hide irrelevant fields when terminal is selected.
- `src/tui/components/preview.rs`: Display terminal sessions appropriately.
- Tests in `src/agents.rs` need updating for the new entry.
