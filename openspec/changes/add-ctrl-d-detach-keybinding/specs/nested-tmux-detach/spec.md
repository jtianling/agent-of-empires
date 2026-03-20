## MODIFIED Requirements

### Requirement: Detach key returns to parent session when inside managed session
When AoE is running inside an existing tmux session and the user has switched to an AoE-managed
session (prefixed `aoe_`), pressing `Ctrl+d` or the tmux detach shortcut
(`Ctrl+b d`) SHALL switch back to the AoE tmux session that initiated the attach flow instead of
fully detaching the tmux client or merely returning to the most recently visited managed session.

#### Scenario: Detach from managed agent session while nested in tmux
- **WHEN** AoE is running inside a tmux session (TMUX env var is set)
- **AND** the user opens a managed agent session (switching via `switch-client`)
- **AND** the user presses `Ctrl+d` inside the managed session
- **THEN** the tmux client switches back to the AoE TUI session that initiated the attach instead
  of disconnecting entirely

#### Scenario: Ctrl+b d still works as before
- **WHEN** AoE is running inside a tmux session (TMUX env var is set)
- **AND** the user opens a managed agent session (switching via `switch-client`)
- **AND** the user presses `Ctrl+b d` inside the managed session
- **THEN** the tmux client switches back to the AoE TUI session (existing behavior unchanged)

#### Scenario: Detach still returns to AoE after in-scope session cycling
- **WHEN** AoE is running inside a tmux session (TMUX env var is set)
- **AND** the user opens a managed AoE session from the AoE TUI
- **AND** the user uses `Ctrl+b n` or `Ctrl+b p` to cycle to another managed session in the same
  allowed cycle scope
- **AND** the user presses `Ctrl+d`
- **THEN** the tmux client switches back to the original AoE TUI session that initiated the attach

#### Scenario: Normal detach preserved in non-managed sessions
- **WHEN** the user is in a tmux session whose name does NOT start with `aoe_`
- **AND** the user presses `Ctrl+d`
- **THEN** the keypress SHALL be sent through to the application as a normal `C-d` (EOF, etc.)

#### Scenario: Ctrl+d passthrough in non-managed sessions
- **WHEN** the user is in a tmux session whose name does NOT start with `aoe_`
- **AND** the user presses `Ctrl+d`
- **THEN** `send-keys C-d` executes so the underlying application receives the EOF signal

#### Scenario: Binding is set after each switch-client call
- **WHEN** AoE calls `switch-client` to attach to a managed agent session
- **THEN** the `C-d` root-table key binding is configured alongside the existing prefix `d` binding
- **AND** the binding uses `if-shell` to check session name before executing

#### Scenario: Hook dynamically rebinds on session change
- **WHEN** a tmux `client-session-changed` event fires
- **AND** the new session is an AoE-managed session (name starts with `aoe_`)
- **THEN** the hook SHALL invoke `aoe tmux refresh-bindings` which sets the root `C-d` binding alongside `d/n/p` prefix bindings

#### Scenario: Hook restores normal bindings for non-managed sessions
- **WHEN** a tmux `client-session-changed` event fires
- **AND** the new session is NOT an AoE-managed session
- **THEN** the hook SHALL unbind `C-d` from the root table and restore `d` to `detach-client`

#### Scenario: Cleanup removes root C-d binding
- **WHEN** AoE exits normally
- **THEN** the `C-d` root-table binding SHALL be removed (`unbind-key -T root C-d`)
- **AND** existing cleanup of prefix bindings continues as before

#### Scenario: Correct client is targeted in multi-client environments
- **WHEN** multiple tmux clients are attached to the AoE TUI session
- **AND** the user attaches to a managed session from the TUI
- **THEN** the `switch-client` call SHALL use `-c` to explicitly target the client that initiated the attach
- **AND** the return session SHALL be stored for that specific client
