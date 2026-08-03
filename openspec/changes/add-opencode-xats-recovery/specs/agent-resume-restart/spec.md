## ADDED Requirements

### Requirement: Shift+R resumes each OpenCode conversation exactly

`Shift+R` SHALL restart every tracked host OpenCode pane with that slot's persisted `native_session_id`.  Each Cross Agent Team OpenCode slot SHALL advance and reserve its own runtime generation before its old process is replaced.  A missing, invalid or unavailable session id SHALL be a per-pane error, not a fresh fallback.

#### Scenario: Two same-cwd panes resume independently
- **WHEN** two OpenCode slots in the same cwd hold `ses_left` and `ses_right`
- **AND** the user presses `Shift+R`
- **THEN** the left runtime SHALL attach only to `ses_left`
- **AND** the right runtime SHALL attach only to `ses_right`

#### Scenario: Resume preserves xats identity
- **WHEN** a Cross Agent Team OpenCode slot resumes generation N from session S
- **THEN** generation N SHALL use the slot's existing identity key and S
- **AND** SHALL commit only the new endpoint for that exact runtime

#### Scenario: Missing session does not clear context silently
- **WHEN** a tracked OpenCode slot has no valid durable session id during `Shift+R`
- **THEN** that pane SHALL report a resume error
- **AND** SHALL not start a fresh OpenCode conversation under resume mode
