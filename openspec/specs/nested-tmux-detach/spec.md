# Capability Spec: Nested Tmux Detach

**Capability**: `nested-tmux-detach`
**Created**: 2026-03-12
**Status**: Stable

## Overview

When AoE runs inside an existing tmux session and the user switches into an AoE-managed tmux
session, the detach shortcut should return to the AoE tmux session that initiated the attach flow
instead of disconnecting the client entirely.

## Requirements

### Requirement: Detach key returns to parent session when inside managed session
When AoE is running inside an existing tmux session and the user has switched to an AoE-managed
session (prefixed `aoe_`, `aoe_term_`, or `aoe_cterm_`), pressing the tmux detach shortcut
(`Ctrl+b d`) SHALL switch back to the AoE tmux session that initiated the attach flow instead of
fully detaching the tmux client or merely returning to the most recently visited managed session.

#### Scenario: Detach from managed agent session while nested in tmux
- **WHEN** AoE is running inside a tmux session (TMUX env var is set)
- **AND** the user opens a managed agent session (switching via `switch-client`)
- **AND** the user presses `Ctrl+b d` inside the managed session
- **THEN** the tmux client switches back to the AoE TUI session that initiated the attach instead
  of disconnecting entirely

#### Scenario: Detach still returns to AoE after session cycling
- **WHEN** AoE is running inside a tmux session (TMUX env var is set)
- **AND** the user opens a managed AoE session from the AoE TUI
- **AND** the user uses `Ctrl+b j` or `Ctrl+b k` to cycle between managed sessions
- **AND** the user presses `Ctrl+b d`
- **THEN** the tmux client switches back to the original AoE TUI session that initiated the attach

#### Scenario: Normal detach preserved in non-managed sessions
- **WHEN** the user is in a tmux session whose name does NOT start with `aoe_`
- **AND** the user presses `Ctrl+b d`
- **THEN** `detach-client` executes as normal (the client disconnects from tmux)

#### Scenario: Binding is set after each switch-client call
- **WHEN** AoE calls `switch-client` to attach to any managed session type (agent, terminal, or
  container terminal)
- **THEN** the `d` key binding is configured in the tmux server to use the nested-detach behavior
- **AND** the binding records the AoE session that initiated the attach as the return target

### Requirement: Session cycling via Ctrl+b j/k
While attached to any AoE-managed tmux session, the user SHALL be able to cycle directly between
agent sessions in the same profile that initiated the attach using `Ctrl+b j` (next) and `Ctrl+b
k` (previous), without returning to the AoE TUI first.

#### Scenario: Cycle to next session
- **WHEN** the user is attached to an AoE-managed session
- **AND** there are multiple agent sessions in the current profile
- **AND** the user presses `Ctrl+b j`
- **THEN** the tmux client switches to the next agent session in the same order shown by the AoE
  session list for that profile
- **AND** if the current session is the last one, it wraps to the first

#### Scenario: Cycle to previous session
- **WHEN** the user is attached to an AoE-managed session
- **AND** there are multiple agent sessions in the current profile
- **AND** the user presses `Ctrl+b k`
- **THEN** the tmux client switches to the previous agent session in the same order shown by the
  AoE session list for that profile
- **AND** if the current session is the first one, it wraps to the last

#### Scenario: Single session does nothing
- **WHEN** the user is attached to an AoE-managed session
- **AND** there is only one agent session in the current profile
- **AND** the user presses `Ctrl+b j` or `Ctrl+b k`
- **THEN** nothing happens (the user stays in the current session)

#### Scenario: Cycling is scoped to the attach-origin profile
- **WHEN** multiple AoE instances are running with different profiles
- **AND** the user presses `Ctrl+b j` or `Ctrl+b k` inside a managed session entered from profile
  `work`
- **THEN** only sessions belonging to profile `work` are considered for cycling
- **AND** sessions from other profiles are excluded even if the current tmux client previously
  visited them

#### Scenario: Bindings are set before attach
- **WHEN** any session type (agent, terminal, container terminal) is attached via `attach()`
- **THEN** the `j` and `k` key bindings are configured in the tmux server before the attach call
- **AND** the bindings work in both nested tmux mode (TMUX env set) and non-nested mode

#### Scenario: Bindings are cleaned up on exit
- **WHEN** AoE exits normally
- **THEN** the `j` and `k` key bindings are removed from the tmux server
