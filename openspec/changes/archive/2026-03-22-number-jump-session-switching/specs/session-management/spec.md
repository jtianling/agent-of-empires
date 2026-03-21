## MODIFIED Requirements

### Requirement: Session attach configures tmux key bindings
When attaching to any AoE-managed tmux session, the attach operation SHALL configure tmux key bindings for navigation: `Ctrl+b d` for detach/return (nested mode only), `Ctrl+b n/p` for session cycling (all modes), and `Ctrl+b 1-9` for number jump via key tables (all modes). The `attach()` method accepts a `profile` parameter to scope session cycling and number jump to the current profile.

#### Scenario: Agent session attach sets bindings
- **WHEN** `Session::attach(profile)` is called
- **THEN** session cycling bindings (`n`/`p`) are configured scoped to the given profile
- **AND** number jump bindings (`1`-`9` with `aoe-1` through `aoe-9` key tables) are configured
- **AND** if TMUX env var is set and `switch-client` succeeds, the `d` binding is also configured

#### Scenario: Number jump bindings cleaned up on detach
- **WHEN** `cleanup_session_cycle_bindings()` is called
- **THEN** keys `1`-`9` SHALL be unbound from the prefix table
- **AND** all `aoe-N` key table bindings SHALL be unbound
