# Capability Spec: Nested Tmux Detach

**Capability**: `nested-tmux-detach`
**Created**: 2026-03-12
**Status**: Stable

## Overview

When AoE runs inside an existing tmux session and the user switches into an AoE-managed tmux
session, the detach shortcut should return to the parent tmux session instead of disconnecting the
client entirely.

## Requirements

### Requirement: Detach key returns to parent session when inside managed session
When AoE is running inside an existing tmux session and the user has switched to an AoE-managed
session (prefixed `aoe_`, `aoe_term_`, or `aoe_cterm_`), pressing the tmux detach shortcut
(`Ctrl+b d`) SHALL switch back to the previous tmux session instead of fully detaching the tmux
client.

#### Scenario: Detach from managed agent session while nested in tmux
- **WHEN** AoE is running inside a tmux session (TMUX env var is set)
- **AND** the user opens a managed agent session (switching via `switch-client`)
- **AND** the user presses `Ctrl+b d` inside the managed session
- **THEN** the tmux client switches back to the previous session (the AoE TUI session) instead of
  disconnecting entirely

#### Scenario: Normal detach preserved in non-managed sessions
- **WHEN** the user is in a tmux session whose name does NOT start with `aoe_`
- **AND** the user presses `Ctrl+b d`
- **THEN** `detach-client` executes as normal (the client disconnects from tmux)

#### Scenario: Binding is set after each switch-client call
- **WHEN** AoE calls `switch-client` to attach to any managed session type (agent, terminal, or
  container terminal)
- **THEN** the `d` key binding is configured in the tmux server to use the nested-detach behavior
