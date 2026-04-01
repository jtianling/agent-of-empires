## Why

AoE's worktree sync currently only handles agent **directories** (`.claude`, `.codex`, etc.) but ignores root-level agent **files** like `CLAUDE.md` and `AGENTS.md`, as well as the `.agents/` directory. These files are often `.gitignore`'d and contain critical project instructions for AI agents. When a worktree is created, agents working in that worktree have no access to these config files, breaking their workflow.

## What Changes

- Add `CLAUDE.md` and `AGENTS.md` to the set of items synced to worktrees (same rules: exists + gitignored + target doesn't exist)
- Add `.agents/` to the `AGENT_DIRS` list
- Clean up synced files (not just directories) before worktree deletion
- Rename internal functions from `agent_dirs` to `agent_config` to reflect the broader scope

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `worktree-agent-dir-sync`: Extend the sync/cleanup behavior to also handle root-level agent config files (`CLAUDE.md`, `AGENTS.md`) and the `.agents/` directory

## Impact

- `src/git/mod.rs`: Add `AGENT_FILES` constant, extend sync/cleanup functions, update tests
- No config changes, no API changes, no breaking changes
