## MODIFIED Requirements

### Requirement: Hook captures native session id keyed by tmux pane

The installed agent status hook SHALL, in addition to its existing status-file write, capture the agent's native session id into the SQLite store keyed by `$TMUX_PANE`. The native session id SHALL be read from the source that agent provides, named per agent rather than assumed: Claude supplies it as `.session_id` in the hook's **stdin JSON**, and Codex exports it as the `$CODEX_THREAD_ID` **environment variable** in the pane. An agent that declares no source SHALL NOT be captured, rather than have one guessed for it. The capture SHALL also record the working directory (`.cwd` from stdin or `$PWD`).

#### Scenario: Claude session id captured from stdin
- **WHEN** a Claude agent fires a hook event inside a tmux pane
- **AND** the hook stdin JSON contains `session_id`
- **THEN** the store SHALL hold a `pane_live` row for that pane's `$TMUX_PANE`
- **AND** the row's `native_session_id` SHALL equal the stdin `session_id`
- **AND** the row's `cwd` SHALL equal the agent's working directory

#### Scenario: Codex session id captured from its environment
- **WHEN** a Codex agent fires a hook event inside a tmux pane
- **AND** `$CODEX_THREAD_ID` is set in that pane's environment
- **THEN** the store SHALL hold a `pane_live` row for that pane's `$TMUX_PANE`
- **AND** the row's `native_session_id` SHALL equal `$CODEX_THREAD_ID`
- **AND** the row's `agent` SHALL be `codex`

#### Scenario: A capture with no session id from its agent's source is not written
- **WHEN** an agent fires a hook event inside a tmux pane
- **AND** its declared source yields no session id
- **THEN** the hook SHALL NOT write a capture row
- **AND** the hook SHALL NOT fall back to another agent's source

#### Scenario: Hand-launched agent without AOE_INSTANCE_ID is still captured
- **WHEN** a user manually runs an agent inside a shell pane (no `$AOE_INSTANCE_ID` in the environment)
- **AND** the pane has a `$TMUX_PANE` value
- **THEN** the hook SHALL still write the `pane_live` capture row
- **AND** the capture SHALL NOT depend on `$AOE_INSTANCE_ID`

#### Scenario: Capture no-ops outside tmux
- **WHEN** an agent fires a hook event but `$TMUX_PANE` is empty (not running inside tmux)
- **THEN** the hook SHALL NOT write a capture row
- **AND** the hook SHALL exit successfully without error

## ADDED Requirements

### Requirement: Codex hooks are installed by merging into the user's configuration

AoE SHALL install Codex's status hooks into `~/.codex/config.toml` by merging: it SHALL add or replace only the hook entries it recognizes as its own, and SHALL preserve every other key, section, and value in the file exactly as written. Uninstall SHALL remove only AoE's own entries and SHALL leave the file in place when anything else remains in it.

The hook configuration SHALL declare the settings format it is written in, and the installer SHALL dispatch on that declaration rather than inferring the format from the file's path.

Nothing SHALL be written to `~/.codex/config.toml` unless Codex is a detected agent on the machine.

#### Scenario: Unrelated configuration survives installation
- **WHEN** `~/.codex/config.toml` holds user configuration AoE did not write
- **AND** Codex hooks are installed
- **THEN** every unrelated key, section, and value SHALL be unchanged
- **AND** the AoE hook entries SHALL be present

#### Scenario: Uninstall removes only AoE's entries
- **WHEN** `~/.codex/config.toml` holds both AoE hook entries and unrelated configuration
- **AND** hooks are uninstalled
- **THEN** the AoE hook entries SHALL be gone
- **AND** the unrelated configuration SHALL remain
- **AND** the file SHALL NOT be deleted

#### Scenario: Reinstalling does not duplicate entries
- **WHEN** AoE hook entries are already present
- **AND** installation runs again
- **THEN** the entries SHALL be replaced rather than appended
- **AND** the file SHALL hold exactly one AoE entry per hook event

### Requirement: The Codex hook trust step is reported rather than bypassed

Codex does not run a newly installed hook until the user has trusted it. AoE SHALL NOT pass `--dangerously-bypass-hook-trust` or otherwise arrange for its hooks to run without that review. Installation SHALL tell the user that Codex will ask them to trust the hook once, so that a Codex pane not being captured before that step is understood as the pending step rather than as a failure.

#### Scenario: Installation states the pending trust step
- **WHEN** Codex hooks are installed
- **THEN** the user SHALL be told that Codex requires trusting the hook once before it runs

#### Scenario: Trust is never bypassed
- **WHEN** AoE installs or invokes Codex hooks
- **THEN** it SHALL NOT use a flag that bypasses Codex's hook trust
