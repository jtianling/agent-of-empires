## 1. Extend the well-known agent config file list

- [x] 1.1 In `src/git/mod.rs`, add `.mcp.json`, `opencode.json`, `opencode.jsonc`, and `.codex/config.toml` to the `AGENT_FILES` constant
- [x] 1.2 Update the doc comment on `AGENT_FILES` to note entries may be nested relative paths (a file inside an agent directory), not only repo-root files

## 2. Support nested paths in file sync

- [x] 2.1 In `sync_agent_config_to_worktree`, before copying an agent config file, create the target's parent directory (`create_dir_all`) so nested paths like `.codex/config.toml` land; keep this a no-op for root-level files
- [x] 2.2 Confirm the existing guards still apply unchanged to nested entries: skip when `target_path.exists()`, skip when `is_tracked(source, path)`, and treat copy failure as non-fatal (log warning, continue)
- [x] 2.3 Confirm `cleanup_agent_config_from_worktree` removes a nested untracked file via `remove_file` and still honors the `is_tracked` skip guard

## 3. Tests

- [x] 3.1 Add a test that `.mcp.json` (gitignored, untracked) is copied into a new worktree by the sync
- [x] 3.2 Add a test that `opencode.json` is copied into a new worktree by the sync
- [x] 3.3 Add a test that nested `.codex/config.toml` is copied into a new worktree, creating `.codex/` when absent
- [x] 3.4 Add a test that an already-present config file in the worktree is NOT overwritten (skip on `target_path.exists()`)
- [x] 3.5 Add a test that a tracked config file is NOT copied by the sync (git worktree already provides it)
- [x] 3.6 Add a test that `cleanup_agent_config_from_worktree` removes an untracked `.mcp.json` (and nested `.codex/config.toml`) but leaves tracked files

## 4. Validate

- [x] 4.1 Run `cargo fmt`, `cargo clippy`, and the git module tests; ensure all pass
- [x] 4.2 Run `openspec validate sync-mcp-config-files-to-worktree --strict` and fix any issues
