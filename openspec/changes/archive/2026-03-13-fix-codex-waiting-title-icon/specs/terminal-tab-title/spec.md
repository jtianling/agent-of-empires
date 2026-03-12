## MODIFIED Requirements

### Requirement: Attached AoE tmux sessions propagate the active pane title
When AoE attaches the user to an AoE-managed tmux session, the outer terminal title SHALL follow
the active pane title for the duration of that attachment. Codex CLI sessions SHALL additionally
surface a raised-hand waiting indicator in the pane title whenever Codex is waiting for user input
or approval, without changing title ownership for other session types.

#### Scenario: Agent session is attached
- **WHEN** AoE attaches to a managed agent or paired-terminal tmux session
- **THEN** AoE SHALL configure that tmux session to enable client terminal title updates
- **AND** tmux SHALL derive the outer terminal title from the active pane title

#### Scenario: Agent updates its own title dynamically
- **WHEN** an attached agent that manages its own title writes a new OSC title during runtime
- **THEN** the pane title SHALL update
- **AND** the outer terminal title SHALL update to match without requiring AoE to poll or redraw

#### Scenario: AoE manages pane titles for the agent
- **WHEN** an attached session uses an agent where AoE manages the pane title
- **THEN** AoE-managed pane title updates SHALL become visible in the outer terminal title while
  attached

#### Scenario: Codex CLI waits for user input
- **WHEN** a running Codex CLI session reaches a waiting-for-input or waiting-for-approval state
- **THEN** the tmux pane title SHALL become `✋ <session title>`
- **AND** the outer terminal title SHALL reflect that waiting title while attached

#### Scenario: Codex CLI resumes from waiting
- **WHEN** a Codex CLI session leaves the waiting state and returns to running or idle
- **THEN** the tmux pane title SHALL revert to `<session title>`
- **AND** the outer terminal title SHALL stop showing the raised-hand prefix while attached

#### Scenario: Non-Codex sessions keep their existing title behavior
- **WHEN** AoE manages or attaches to a session that does not use Codex CLI
- **THEN** this change SHALL NOT alter that session type's title ownership or waiting-title behavior
