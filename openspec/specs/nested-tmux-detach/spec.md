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

#### Scenario: Detach still returns to AoE after in-scope session cycling
- **WHEN** AoE is running inside a tmux session (TMUX env var is set)
- **AND** the user opens a managed AoE session from the AoE TUI
- **AND** the user uses `Ctrl+b n` or `Ctrl+b p` to cycle to another managed session in the same
  allowed cycle scope
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

#### Scenario: Hook dynamically rebinds on session change
- **WHEN** a tmux `client-session-changed` event fires
- **AND** the new session is an AoE-managed session (name starts with `aoe_`)
- **THEN** the hook SHALL invoke `aoe tmux refresh-bindings` to set `d/n/p` bindings via external process (bypassing tmux's internal command parser)

#### Scenario: Hook restores normal bindings for non-managed sessions
- **WHEN** a tmux `client-session-changed` event fires
- **AND** the new session is NOT an AoE-managed session
- **THEN** the hook SHALL restore `d` to `detach-client` and unbind `n` and `p`

#### Scenario: Correct client is targeted in multi-client environments
- **WHEN** multiple tmux clients are attached to the AoE TUI session
- **AND** the user attaches to a managed session from the TUI
- **THEN** the `switch-client` call SHALL use `-c` to explicitly target the client that initiated the attach
- **AND** the return session SHALL be stored for that specific client

### Requirement: Session cycling via Ctrl+b n/p
While attached to any AoE-managed tmux session, the user SHALL be able to cycle directly between
managed sessions in the same attach-origin profile and the same current session scope using
`Ctrl+b n` (next) and `Ctrl+b p` (previous), without returning to the AoE TUI first. The current
session scope SHALL be the current session's exact `group_path`; sessions with an empty
`group_path` SHALL only cycle among other ungrouped sessions.

#### Scenario: Cycle to next session within the same group
- **WHEN** the user is attached to an AoE-managed session whose `group_path` is `skills-manager`
- **AND** there are multiple managed sessions in profile `work` with `group_path =
  skills-manager`
- **AND** the user presses `Ctrl+b n`
- **THEN** the tmux client switches to the next managed session whose `group_path` is
  `skills-manager`
- **AND** if the current scoped session is the last one, it wraps to the first scoped session

#### Scenario: Cycle to previous session within the same group
- **WHEN** the user is attached to an AoE-managed session whose `group_path` is `skills-manager`
- **AND** there are multiple managed sessions in profile `work` with `group_path =
  skills-manager`
- **AND** the user presses `Ctrl+b p`
- **THEN** the tmux client switches to the previous managed session whose `group_path` is
  `skills-manager`
- **AND** if the current scoped session is the first one, it wraps to the last scoped session

#### Scenario: Ungrouped session cycles only among ungrouped sessions
- **WHEN** the user is attached to an AoE-managed session whose `group_path` is empty
- **AND** there are multiple ungrouped managed sessions in the current profile
- **AND** there are also grouped managed sessions in that same profile
- **AND** the user presses `Ctrl+b n` or `Ctrl+b p`
- **THEN** the tmux client switches only among the ungrouped managed sessions
- **AND** grouped sessions are excluded from the cycle target list

#### Scenario: Grouped session excludes sibling groups and ungrouped sessions
- **WHEN** the user is attached to an AoE-managed session whose `group_path` is `skills-manager`
- **AND** the current profile also contains managed sessions in `blog-workspace`, `main`, or no
  group
- **AND** the user presses `Ctrl+b n` or `Ctrl+b p`
- **THEN** sessions outside `skills-manager` are excluded from the cycle target list

#### Scenario: Single in-scope session does nothing
- **WHEN** the user is attached to an AoE-managed session
- **AND** there is only one managed session in the current session scope
- **AND** the user presses `Ctrl+b n` or `Ctrl+b p`
- **THEN** nothing happens (the user stays in the current session)

#### Scenario: Cycling is scoped to the attach-origin profile
- **WHEN** multiple AoE instances are running with different profiles
- **AND** the user presses `Ctrl+b n` or `Ctrl+b p` inside a managed session entered from profile
  `work`
- **THEN** only sessions belonging to profile `work` are considered for cycling
- **AND** sessions from other profiles are excluded even if the current tmux client previously
  visited them

#### Scenario: Bindings are set before attach
- **WHEN** any session type (agent, terminal, container terminal) is attached via `attach()`
- **THEN** the `n` and `p` key bindings are configured in the tmux server before the attach call
- **AND** the bindings work in both nested tmux mode (TMUX env set) and non-nested mode

#### Scenario: Bindings are cleaned up on exit
- **WHEN** AoE exits normally
- **THEN** the `n` and `p` key bindings are removed from the tmux server

### Requirement: Vi-style pane navigation via Ctrl+b h/j/k/l
While attached to any AoE-managed tmux session, the user SHALL be able to navigate between tmux
panes using vi-style key bindings: `Ctrl+b h` (left), `Ctrl+b j` (down), `Ctrl+b k` (up), and
`Ctrl+b l` (right). These bindings supplement the default arrow key navigation.

#### Scenario: Pane navigation bindings are set when entering a managed session
- **WHEN** the user enters an AoE-managed tmux session
- **THEN** `Ctrl+b h/j/k/l` SHALL be bound to `select-pane -L/-D/-U/-R` respectively

#### Scenario: Pane navigation bindings are removed when leaving a managed session
- **WHEN** the user switches to a non-AoE session
- **THEN** `h`, `j`, `k`, and `l` prefix key bindings SHALL be unbound
