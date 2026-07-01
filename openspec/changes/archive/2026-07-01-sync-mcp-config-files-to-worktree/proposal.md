## Why

When AoE creates a git worktree for a session, it syncs untracked agent config
(`.claude`, `.codex`, `CLAUDE.md`, ...) so the agent keeps its context. But the
per-project MCP config files are not covered: Claude Code's `.mcp.json` and
opencode's `opencode.json` / `opencode.jsonc` are never copied, so an agent
running inside a worktree loses its MCP servers (e.g. cross-agent-teams). codex
reads its config from the project-local `.codex/config.toml`, which today only
rides along by accident when the whole `.codex/` directory happens to be copied.

## What Changes

- Add `.mcp.json` (Claude Code) and `opencode.json` / `opencode.jsonc` (opencode)
  to the well-known agent config file list so they are copied on worktree
  creation and cleaned up on deletion.
- Add `.codex/config.toml` (codex) as a first-class synced config file so it is
  guaranteed independent of whether the `.codex/` directory copy runs. This is a
  nested path, so the file-sync step must create the parent directory before
  copying.
- Reuse the existing sync/cleanup guards unchanged: skip when the target already
  exists in the worktree, skip when the source entry is tracked by git (git
  worktree already populates it), and never delete a tracked file on cleanup.
- No new configuration surface: the list stays a hardcoded constant, so no
  settings-TUI wiring is required.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `worktree-agent-dir-sync`: extend the well-known agent config file list to
  include the per-agent MCP config files (`.mcp.json`, `opencode.json`,
  `opencode.jsonc`, `.codex/config.toml`) and specify nested-path support so a
  file inside an agent directory (e.g. `.codex/config.toml`) can be synced and
  cleaned up on its own, guarded by the existing "already exists" / "tracked"
  checks.

## Impact

- `src/git/mod.rs`: extend the `AGENT_FILES` constant; make the file-sync loop
  create the target parent directory before copying so nested paths work.
- `openspec/specs/worktree-agent-dir-sync/spec.md`: updated requirement list and
  a new scenario for nested-path sync/skip.
- No breaking changes; no data migration; no config schema change.
