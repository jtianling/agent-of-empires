## MODIFIED Requirements

### Requirement: Returning from an attached session restores the actual detached session selection
When the user returns from an attached AoE-managed tmux session to the home screen, AoE SHALL restore selection to the session the user actually detached from, even if they switched sessions inside tmux after the initial attach. The client name for per-client tracking SHALL be resolved from the terminal's tty name.

#### Scenario: Detach restores the originally attached session when no cycling occurred
- **WHEN** the user attaches to a session from the home screen
- **AND** the user later returns to the TUI without switching to another managed session first
- **THEN** the home screen SHALL select that same session after the TUI reloads

#### Scenario: Detach restores the cycled-to session
- **WHEN** the user attaches to a session from the home screen
- **AND** the user switches to another AoE-managed session with root-table `Ctrl+.` or `Ctrl+,`
- **AND** the user presses `Ctrl+b d` to return to the TUI
- **THEN** the home screen SHALL select the session the user detached from
- **AND** AoE SHALL NOT force selection back to the originally attached session

#### Scenario: Client name resolved from tty name
- **WHEN** the TUI resolves the attach client name for per-client tracking
- **THEN** the system SHALL use `get_tty_name()` to obtain the terminal's tty path
- **AND** the system SHALL NOT check the TMUX env var for client name resolution

## REMOVED Requirements

### Requirement: FR-003a nested detach binding behavior
**Reason**: Nested tmux mode is removed. The requirement that `Ctrl+b d` inside a managed session switches back to the previous session (rather than fully detaching) when running inside an existing tmux session is no longer applicable. In non-nested mode, `Ctrl+b d` performs standard `detach-client`, and `Ctrl+q` returns to the AoE TUI.
**Migration**: Use `Ctrl+q` to return to the AoE TUI from a managed session. `Ctrl+b d` detaches from tmux entirely.

### Requirement: FR-003b TMUX-gated mouse save/restore
**Reason**: The requirement to save and restore the outer tmux session's mouse setting when the TUI starts and stops is removed. This was only relevant when AoE ran nested inside another tmux session. In non-nested mode, there is no outer tmux session whose mouse settings need preservation.
**Migration**: No user action needed.
