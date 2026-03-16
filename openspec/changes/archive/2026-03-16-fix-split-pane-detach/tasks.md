## 1. Store agent pane ID on session creation

- [x] 1.1 Add `get_agent_pane_id(session_name)` helper in `src/tmux/utils.rs` that reads `@aoe_agent_pane` option from a tmux session, returning `Option<String>`
- [x] 1.2 Add `append_store_pane_id_args()` in `src/tmux/utils.rs` that appends tmux commands to capture `#{pane_id}` and store it as `@aoe_agent_pane` on the session, chained atomically with session creation
- [x] 1.3 Call `append_store_pane_id_args()` in `Session::create_with_size()` (`src/tmux/session.rs`)
- [x] 1.4 Call `append_store_pane_id_args()` in `TerminalSession::create()` (`src/tmux/terminal_session.rs`)
- [x] 1.5 Call `append_store_pane_id_args()` in `ContainerTerminalSession::create()` (`src/tmux/terminal_session.rs`)

## 2. Update pane health checks to target stored pane ID

- [x] 2.1 Change `is_pane_dead()` in `src/tmux/utils.rs` to accept an optional pane target override, falling back to session name
- [x] 2.2 Change `pane_current_command()` and `is_pane_running_shell()` in `src/tmux/utils.rs` to accept an optional pane target override
- [x] 2.3 Update `Session::is_pane_dead()`, `is_pane_running_shell()`, `get_pane_pid()` in `src/tmux/session.rs` to read `@aoe_agent_pane` and pass it to the utils functions
- [x] 2.4 Update `TerminalSession::is_pane_dead()`, `get_pane_pid()` in `src/tmux/terminal_session.rs` to use stored pane ID
- [x] 2.5 Update `ContainerTerminalSession::is_pane_dead()`, `get_pane_pid()` in `src/tmux/terminal_session.rs` to use stored pane ID

## 3. Verify and test

- [x] 3.1 Run `cargo clippy` and `cargo fmt` to ensure clean code
- [x] 3.2 Run `cargo test` to verify no regressions
- [x] 3.3 Manual test: create session, split pane, detach from split pane, re-enter -- verify splits preserved
