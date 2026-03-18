## MODIFIED Requirements

### Requirement: Dynamic rebinding hook
The `client-session-changed` hook SHALL dynamically adjust key bindings based on the current session:
- For sessions matching `aoe_*` prefix: run `aoe tmux refresh-bindings` to configure managed bindings (d/n/p/h/j/k/l)
- For other sessions: restore default `detach-client` binding and remove cycle bindings

The hook pattern `aoe_*` matches agent sessions (`aoe_` prefix). Terminal session prefixes (`aoe_term_*`, `aoe_cterm_*`) are no longer created, so they do not need to be matched.

#### Scenario: Hook fires on agent session
- **WHEN** the client switches to a session matching `aoe_*`
- **THEN** managed bindings (d, n, p, h, j, k, l) are configured

#### Scenario: Hook fires on non-managed session
- **WHEN** the client switches to a session not matching `aoe_*`
- **THEN** default detach binding is restored and cycle bindings are removed

### Requirement: Session cycling scope
`Ctrl+b n` and `Ctrl+b p` SHALL cycle only through agent sessions (`aoe_` prefix) within the same group path and profile. Terminal sessions (`aoe_term_*`, `aoe_cterm_*`) are no longer created and are excluded from the cycle target list.

#### Scenario: Cycling skips orphaned terminal sessions
- **WHEN** an orphaned `aoe_term_*` tmux session exists from a previous AoE version
- **THEN** session cycling does not include it in the cycle order

### Requirement: Managed session detection
A tmux session is considered "managed by AoE" if its name starts with `aoe_` and it corresponds to a known Instance. The `matches_managed_tmux_name` helper SHALL only match agent session names (`Session::generate_name`), not terminal session names.

#### Scenario: Only agent sessions are matched
- **WHEN** checking if a tmux session name matches a managed session
- **THEN** only the agent session name pattern is checked (not terminal or container terminal patterns)
