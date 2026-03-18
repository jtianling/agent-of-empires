## Context

AoE currently provides a "paired terminal" feature: each agent session can have an associated shell (TerminalSession) and, for sandboxed sessions, a container shell (ContainerTerminalSession). The TUI has a ViewMode toggle (`t` key) to switch between agent and terminal views, a TerminalMode toggle (`c` key) for host vs container, preview caches for terminal output, and settings for default terminal mode. This adds significant complexity across tmux lifecycle, TUI rendering, session storage, and configuration.

The user's workflow already provides terminal access through native terminal splitting. The feature is unused.

## Goals / Non-Goals

**Goals:**
- Remove all terminal-related code paths cleanly without breaking agent session functionality
- Simplify the TUI by removing ViewMode (only agent view remains)
- Simplify session data by removing TerminalInfo
- Simplify config by removing default_terminal_mode
- Ensure existing sessions with terminal_info in JSON load without error (field is simply ignored via serde defaults)

**Non-Goals:**
- Do not remove the terminal-category spec (standalone shell tool sessions) -- that is a different feature
- Do not add a migration to strip terminal_info from existing JSON -- serde will ignore unknown/extra fields naturally
- Do not change the agent session attach/detach/cycle behavior

## Decisions

1. **Delete `src/tmux/terminal_session.rs` entirely** rather than gutting it. The TerminalSession and ContainerTerminalSession structs are only used for the paired terminal feature. Rationale: complete removal is cleaner than leaving dead code.

2. **Remove ViewMode enum** from `src/tui/home/mod.rs`. Since only Agent view remains, the view mode concept is unnecessary. The TUI home screen will always show agent sessions. Rationale: simpler than keeping a single-variant enum.

3. **Remove TerminalMode enum** and all toggle/mode logic. With no terminal view, there is no need to track host vs container mode.

4. **Keep `terminal_info` deserialization tolerant** via `#[serde(default)]` on the Instance struct. Old session JSON files may contain `terminal_info` -- serde will ignore it on deserialization if the field is removed and `deny_unknown_fields` is not set. Rationale: avoids needing a data migration. Alternative considered: adding a migration to strip the field, but it's unnecessary since serde handles it.

5. **Kill orphaned terminal tmux sessions** during cleanup. When AoE cleans up on exit or session delete, any `aoe_term_*` / `aoe_cterm_*` tmux sessions should still be killed if they exist. This handles the transition from old sessions. Rationale: prevents zombie tmux sessions. This can be handled by existing cleanup logic or by a one-time check.

6. **Remove `default_terminal_mode` from config** but don't add a migration to remove it from TOML files. The toml parser will ignore unknown keys.

## Risks / Trade-offs

- [Users with existing terminal sessions] -> On next AoE run, orphaned terminal tmux sessions may linger. Mitigation: keep terminal session cleanup in the session deletion path for one release, or document that users should manually kill aoe_term_* sessions. Since the feature is unused, this is low risk.
- [Breaking config] -> Old config files with `default_terminal_mode` will have an unknown key. Mitigation: toml deserialization with `deny_unknown_fields` disabled (current behavior) will silently ignore it.
