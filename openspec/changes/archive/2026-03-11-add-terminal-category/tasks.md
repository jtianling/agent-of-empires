## 1. Agent Registry

- [x] 1.1 Add `detect_terminal_status` stub to `src/tmux/status_detection.rs` (always returns `Status::Idle`)
- [x] 1.2 Add `terminal` `AgentDef` entry to `AGENTS` array in `src/agents.rs`, after gemini and before cursor, with `yolo: None`, `detection: Which("sh")`, `set_default_command: false`, `supports_host_launch: true`
- [x] 1.3 Update unit tests in `src/agents.rs`: `test_agent_names` (7 entries), `test_get_agent_known`, `test_resolve_tool_name`, `test_settings_index_roundtrip`, and relax `test_all_agents_have_yolo_support` to skip terminal

## 2. Tool Availability

- [x] 2.1 Ensure terminal always appears in `AvailableTools::detect()` -- since `which sh` always succeeds on Unix, no special handling should be needed; verify this

## 3. Session Creation

- [x] 3.1 In the session builder, when tool is "terminal", set the command to launch the user's `$SHELL` (fallback `/bin/sh`) instead of the agent binary

## 4. New Session Dialog UI

- [x] 4.1 Hide YOLO Mode field when "terminal" is selected in the tool picker
- [x] 4.2 Hide Worktree/Branch fields when "terminal" is selected
- [x] 4.3 Verify the tool picker renders "terminal" between "gemini" and "cursor"

## 5. Testing

- [x] 5.1 Add status detection test for `detect_terminal_status` in `src/tmux/status_detection.rs`
- [x] 5.2 Run `cargo test` and `cargo clippy` to ensure all tests pass
- [x] 5.3 Run `cargo fmt` to ensure formatting is clean
