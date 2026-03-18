## REMOVED Requirements

### Requirement: Toggle terminal view
**Reason**: The paired terminal feature is removed. Users can use native terminal splitting for shell access.
**Migration**: The `t` key binding is freed. The TUI always shows the agent view.

### Requirement: Terminal preview
**Reason**: No terminal view means no terminal preview caching or rendering.
**Migration**: None needed. Preview cache code is removed.

## MODIFIED Requirements

### Requirement: Key bindings
The TUI home screen SHALL support the following key bindings:

| Key | Action |
|-----|--------|
| `n` | Open new session dialog |
| `Enter` | Attach to selected agent session |
| `D` | Open diff view for selected session |
| `d` | Delete selected session |
| `r` | Restart selected session |
| `R` | Rename selected session |
| `s` | Open settings |
| `?` | Toggle help overlay |
| `q` | Quit AoE |
| `/` | Start search |
| `Tab` | Toggle group collapse |

#### Scenario: Key t is not bound
- **WHEN** user presses `t` on the home screen
- **THEN** nothing happens (key is unbound)

#### Scenario: Key c is not bound
- **WHEN** user presses `c` on the home screen
- **THEN** nothing happens (key is unbound)

#### Scenario: Enter attaches to agent session
- **WHEN** user presses `Enter` with a session selected
- **THEN** the TUI attaches to the agent's tmux session directly (no view mode check)
