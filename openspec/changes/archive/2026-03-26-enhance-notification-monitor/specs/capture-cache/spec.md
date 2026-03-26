## MODIFIED Requirements

### Requirement: Cache capture-pane results with 500ms TTL
The system SHALL cache the result of `capture-pane` calls per session with a 500ms time-to-live. Subsequent requests for the same session's pane content within the TTL SHALL return the cached content without spawning a new tmux subprocess. This cache operates per-process: both the TUI and the notification monitor maintain their own cache instances.

#### Scenario: First capture within TTL window
- **WHEN** `capture_pane()` is called for a session
- **AND** no cached content exists or the cache has expired
- **THEN** the system SHALL execute `tmux capture-pane` and cache the result with a timestamp

#### Scenario: Second capture within TTL reuses cache
- **WHEN** `capture_pane()` is called for a session
- **AND** a cached result exists that is less than 500ms old
- **THEN** the system SHALL return the cached content
- **AND** SHALL NOT spawn a tmux subprocess

#### Scenario: Cache expires after 500ms
- **WHEN** `capture_pane()` is called for a session
- **AND** a cached result exists that is more than 500ms old
- **THEN** the system SHALL execute a fresh `tmux capture-pane`
- **AND** replace the cached content

#### Scenario: Notification monitor uses capture cache
- **WHEN** the notification monitor needs pane content for status detection
- **THEN** it SHALL call `capture_pane_cached()` instead of direct subprocess calls
- **AND** benefit from the cache if the same session was captured within the TTL

### Requirement: Resume token extraction reuses capture cache
When the status poller extracts a resume token after detecting pane death, it SHALL reuse the capture cache if a recent capture is available, instead of calling `capture_pane()` again.

#### Scenario: Resume token uses cached content
- **WHEN** status detection already called `capture_pane(50)` for an instance
- **AND** the poller then needs to extract a resume token
- **AND** the cached content is within TTL
- **THEN** the resume token extraction SHALL use the cached content
- **AND** SHALL NOT call `capture_pane(100)` separately

#### Scenario: Resume token needs more lines than cached
- **WHEN** the cached content has fewer lines than needed for resume token extraction
- **AND** the cache is within TTL
- **THEN** the system SHALL execute a fresh `capture_pane` with the larger line count
- **AND** update the cache with the new content
