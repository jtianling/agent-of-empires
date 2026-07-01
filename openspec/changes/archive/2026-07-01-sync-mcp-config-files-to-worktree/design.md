## Context

`src/git/mod.rs` already syncs untracked agent config into new worktrees and
cleans it up on deletion, driven by two hardcoded constants:

- `AGENT_DIRS` -- whole directories copied (`.claude`, `.codex`, ...).
- `AGENT_FILES` -- root-level files copied (`CLAUDE.md`, `AGENTS.md`).

`sync_agent_config_to_worktree` and `cleanup_agent_config_from_worktree` iterate
these lists. The file path is missing the per-agent MCP config files, so an agent
in a worktree loses its MCP servers. codex reads project-local
`.codex/config.toml`, which is nested inside an agent directory and today only
arrives if the whole `.codex/` copy runs.

## Goals / Non-Goals

**Goals:**
- Sync + clean up `.mcp.json`, `opencode.json`, `opencode.jsonc` (root files).
- Sync + clean up `.codex/config.toml` (nested path) as a first-class item,
  independent of the `.codex/` directory copy.
- Preserve existing guards: skip when target already exists, skip when tracked by
  git, never delete a tracked file, copy failure is non-fatal.

**Non-Goals:**
- No settings-TUI surface: the list stays a hardcoded constant.
- Not changing the directory-level `is_tracked` whole-dir skip behavior (the
  partial-tracking edge for directories stays as-is; the nested-file entry is the
  targeted fix for codex).
- Not copying secrets (`.env*`) -- out of scope and a leak risk.

## Decisions

**Decision: Represent nested config files in the same `AGENT_FILES` list.**
Add `.codex/config.toml` to `AGENT_FILES` rather than introducing a second
constant. The file-sync loop already computes `worktree_dir.join(file_name)`; the
only gap is that a nested target's parent directory may not exist. Before copying,
create the parent dir (`create_dir_all(target.parent())`). For root-level entries
the parent is the worktree root, which always exists, so this is a no-op there.
Alternative considered: a separate `AGENT_NESTED_FILES` constant -- rejected as
unnecessary duplication; one list with parent-dir creation is general.

**Decision: Keep `.codex` in `AGENT_DIRS` and also list `.codex/config.toml`.**
In the common case (`.codex/` fully untracked) the directory copy brings
`config.toml`, and the explicit file entry then hits the "already exists" guard
and is a no-op. In the edge case where the directory copy is skipped, the explicit
entry still delivers `config.toml`. This is belt-and-suspenders with zero
redundant work at runtime thanks to the existing guard. Alternative considered:
drop `.codex` from `AGENT_DIRS` and rely only on the file entry -- rejected to
avoid changing existing directory-sync behavior for users who keep other content
under `.codex/`.

**Decision: Reuse existing guards verbatim.**
`target_path.exists()` and `is_tracked(...)` already implement "skip if already in
the worktree / already tracked". Adding entries to the constant inherits them; no
new conditional logic is needed for the skip behavior.

## Risks / Trade-offs

- [Nested cleanup could leave an empty `.codex/`] -> The directory-level cleanup
  (`AGENT_DIRS`) removes an untracked `.codex/` wholesale before the file loop
  runs, so the empty-dir case does not occur in the common path; when `.codex/`
  is tracked, its tracked files legitimately remain and `git worktree remove`
  handles them. No extra empty-dir pruning needed.
- [`opencode.jsonc` and `opencode.json` both listed] -> Both are valid opencode
  config names; listing both is cheap and each is independently guarded, so no
  conflict.
