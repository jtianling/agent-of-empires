## Why

Codex CLI sessions currently only get the raised-hand waiting indicator through AoE's background
status poller, which does not cover the live attached tmux session where users actually watch the
Codex UI. Prior attempts also widened the waiting-title behavior beyond Codex, which does not match
the requested scope.

## What Changes

- Make Codex CLI sessions surface a raised-hand icon in the tmux pane title when Codex is waiting
  for user input or approval, and restore the plain session title when Codex resumes running or
  returns to idle.
- Ensure the Codex-specific waiting title behavior applies to the actual Codex tmux session lifecycle
  rather than only the AoE TUI background poller.
- Preserve the existing title behavior for all non-Codex agents and paired terminal sessions.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `terminal-tab-title`: Clarify that Codex CLI sessions must surface a raised-hand waiting title in
  the attached tmux title path without changing how other session types manage titles.

## Impact

- Affected code is expected in the Codex session launch and attach path, tmux title propagation, and
  Codex waiting-state detection reuse.
- No user-facing config changes are expected.
- No breaking API or storage changes are expected.
