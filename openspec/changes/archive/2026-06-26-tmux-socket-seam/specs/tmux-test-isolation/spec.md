## ADDED Requirements

### Requirement: All AoE tmux invocations go through one builder

Every tmux invocation in AoE production code SHALL be constructed by a single `tmux_command()` builder rather than a bare `Command::new("tmux")`. The builder SHALL apply the process-global AoE socket name as `-L <name>` when one is set, and SHALL produce a bare `tmux` (default socket) when no socket name is set. A static test guard SHALL forbid bare `Command::new("tmux")` outside the builder.

#### Scenario: Builder applies the configured socket name
- **WHEN** the process-global socket name is set to `foo`
- **THEN** `tmux_command()` SHALL produce a command whose arguments begin with `-L foo`

#### Scenario: Builder is bare when no socket name is set (production default)
- **WHEN** no socket name is set (production default)
- **THEN** `tmux_command()` SHALL produce a `tmux` command with no `-L`/`-S` flag (the default socket)

#### Scenario: Static guard forbids bare tmux outside the builder
- **WHEN** the test suite runs the isolation guard
- **THEN** it SHALL fail if any production source file constructs `Command::new("tmux")` outside the `tmux_command()` builder

### Requirement: Tests never touch the default tmux socket

Under unit-test builds, the `tmux_command()` builder SHALL resolve to a private per-process socket (`-L aoe_test_<pid>`) even when no socket name was explicitly set, so a test that forgets to opt in still cannot reach the default socket. Because `-L` overrides `$TMUX` (a `tmux` client reads `$TMUX` only when neither `-L` nor `-S` is given), this holds even when the test runner is itself inside tmux (`$TMUX` set). The full test suite SHALL NOT create, kill, or mutate any session or server option on the default socket.

A test helper SHALL additionally clear `$TMUX` and `$TMUX_PANE`; tests that touch tmux SHALL be `#[serial]`. Integration tests (separate crate, where the seam's test path is inactive) that build tmux commands directly SHALL pass `-L <private>` and clear `$TMUX`/`$TMUX_PANE` themselves.

#### Scenario: Unit-test builder is always private even without opt-in
- **WHEN** unit-test code calls `tmux_command()` without having set any socket name
- **THEN** the produced command SHALL carry `-L aoe_test_<pid>` (a private socket), never the default socket

#### Scenario: Private socket holds even with $TMUX set
- **WHEN** the test runner is inside tmux (`$TMUX` points at the live server)
- **AND** a test tmux command is built via the builder (or an integration test's `-L` builder)
- **THEN** the command SHALL target the private `-L` socket, not the `$TMUX` server

#### Scenario: Integration test command is isolated
- **WHEN** `tests/tui_attach_detach.rs` builds a tmux `new-session`/`kill-session` command
- **THEN** it SHALL carry `-L <private>` and have `$TMUX`/`$TMUX_PANE` removed

#### Scenario: Isolation verified without executing tmux
- **WHEN** isolation is verified in tests
- **THEN** verification SHALL assert on the built command's arguments/env (it contains `-L <private>`), and SHALL NOT run a destructive `tmux` subcommand against any server
