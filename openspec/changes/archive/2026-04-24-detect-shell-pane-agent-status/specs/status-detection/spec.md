## ADDED Requirements

### Requirement: Shell session primary-pane agent discovery on detach

When the user detaches from a tmux session whose configured tool is `shell`, AoE SHALL run exactly one agent-type discovery pass against the session's primary pane and cache the result in an in-memory field on the session instance.

The discovery SHALL reuse the existing `detect_agent_type_from_pane` helper. The returned value SHALL be stored in a non-persistent field (e.g. `detected_inner_agent: Option<String>`) on the in-memory session instance. The session's persisted `tool` field MUST NOT be modified.

If `detect_agent_type_from_pane` returns `Some("shell")` or `None`, the field SHALL be set to `None`. Otherwise it SHALL be set to `Some(<agent-name>)`.

#### Scenario: Agent detected on detach

- **WHEN** the user detaches from a shell session back to the AoE TUI and the primary pane's foreground process is a known agent (e.g. `claude`, `codex`, `gemini`)
- **THEN** the session's `detected_inner_agent` field is set to `Some("<agent-name>")` and the session's persisted `tool` remains `"shell"`

#### Scenario: No agent detected on detach

- **WHEN** the user detaches from a shell session back to the AoE TUI and the primary pane's foreground process is a bare shell or cannot be identified
- **THEN** the session's `detected_inner_agent` field is set to `None`

#### Scenario: Detection swap on subsequent detach

- **WHEN** a shell session has `detected_inner_agent = Some("claude")` from a previous detach, the user re-attaches, exits claude, starts codex, and detaches
- **THEN** on the new detach the field is overwritten to `Some("codex")`

#### Scenario: Non-shell session untouched

- **WHEN** the user detaches from a session whose tool is not `shell` (e.g. `claude`)
- **THEN** no shell-pane detection runs and no `detected_inner_agent` field is written for that session

### Requirement: Status polling uses detected inner agent for shell sessions

During status polling, when a session's tool is `shell` AND its in-memory `detected_inner_agent` is `Some(X)`, the primary-pane status detection SHALL dispatch to agent `X`'s content-based detector instead of the shell tool's stub detector.

When `detected_inner_agent` is `None`, primary-pane status detection SHALL fall back to the existing shell behavior (stub detector returning `Idle`, subsequently mapped to `Unknown` by the shell/custom-command heuristic).

When `detected_inner_agent` is `Some(X)` and agent `X`'s content-based detector returns a concrete status (`Running`, `Waiting`, `Idle`, `Error`), that status SHALL NOT be rewritten to `Unknown` by the `Idle → Unknown` heuristic that currently applies to shell sessions.

Status polling MUST NOT mutate `detected_inner_agent`. The field is only written on detach.

#### Scenario: Running status surfaces for detected agent

- **WHEN** a shell session has `detected_inner_agent = Some("claude")` and the primary pane content matches the claude "running" pattern
- **THEN** the TUI displays the session's status as `Running`

#### Scenario: Idle status surfaces for detected agent without override

- **WHEN** a shell session has `detected_inner_agent = Some("claude")` and claude's content detector returns `Idle`
- **THEN** the TUI displays the session's status as `Idle` (not `Unknown`)

#### Scenario: Shell fallback when no inner agent detected

- **WHEN** a shell session has `detected_inner_agent = None`
- **THEN** status polling uses the shell stub detector and the TUI displays `Unknown` (`?`) as it does today

#### Scenario: Detected agent with hook-only detector falls back to Unknown

- **WHEN** a shell session has `detected_inner_agent = Some(X)` where `X` has no usable content-based detector
- **THEN** the TUI displays `Unknown` (`?`) rather than a false signal

#### Scenario: Status poller does not clear detected field

- **WHEN** status polling runs on a shell session while the user is not attached
- **THEN** the session's `detected_inner_agent` field is not modified, regardless of the detected status value
