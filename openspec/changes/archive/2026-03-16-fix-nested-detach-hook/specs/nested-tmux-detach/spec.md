## MODIFIED Requirements

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
- **AND** the user uses `Ctrl+b j` or `Ctrl+b k` to cycle to another managed session in the same
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
- **THEN** the hook SHALL invoke `aoe tmux refresh-bindings` to set `d/j/k` bindings via external process (bypassing tmux's internal command parser)

#### Scenario: Hook restores normal bindings for non-managed sessions
- **WHEN** a tmux `client-session-changed` event fires
- **AND** the new session is NOT an AoE-managed session
- **THEN** the hook SHALL restore `d` to `detach-client` and unbind `j` and `k`

#### Scenario: Correct client is targeted in multi-client environments
- **WHEN** multiple tmux clients are attached to the AoE TUI session
- **AND** the user attaches to a managed session from the TUI
- **THEN** the `switch-client` call SHALL use `-c` to explicitly target the client that initiated the attach
- **AND** the return session SHALL be stored for that specific client
