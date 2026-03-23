## 1. Data Model

- [x] 1.1 Add `ResumeConfig` struct to `src/agents.rs` with fields: `exit_sequence`, `resume_pattern`, `resume_flag`, `timeout_secs`
- [x] 1.2 Add `resume: Option<ResumeConfig>` field to `AgentDef` and set `None` for all existing agents
- [x] 1.3 Configure `ResumeConfig` for Claude (exit: `[["C-c"], ["C-c"]]`, pattern: `claude --resume\s+([0-9a-f-]+)`, flag: `--resume {}`, timeout: 10)
- [x] 1.4 Configure `ResumeConfig` for Codex (exit: `[["C-c"], ["C-c"]]`, pattern: `codex resume\s+([0-9a-f-]+)`, flag: `resume {}`, timeout: 10)

## 2. Status and State

- [x] 2.1 Add `Restarting` variant to `Status` enum in `src/session/mod.rs` and handle it in display/serialization/status-bar logic
- [x] 2.2 Add `PendingResume` struct and `RestartPhase` enum to `src/session/instance.rs` (fields: phase, config reference, started_at, timeout)
- [x] 2.3 Add `pending_resume: Option<PendingResume>` field to `Instance`

## 3. Tmux Primitives

- [x] 3.1 Add `send_keys_to_agent_pane(&self, keys: &[&str])` method to `Session` in `src/tmux/session.rs`

## 4. Core Logic

- [x] 4.1 Modify `build_agent_command()` in `src/session/instance.rs` to accept `resume_token: Option<&str>` and insert `resume_flag` after the binary name when present
- [x] 4.2 Add `initiate_graceful_restart(&mut self)` method on Instance: checks preconditions (non-custom command, has ResumeConfig, no pending_resume), sends first exit key group, sets status to Restarting, creates PendingResume
- [x] 4.3 Add `tick_pending_resume(&mut self) -> Option<Action>` method on Instance: advances the state machine per tick (send next key group / check pane dead / capture + parse / respawn / handle timeout + fallback)

## 5. TUI Integration

- [x] 5.1 Modify `RespawnAgentPane` action handler in `src/tui/app.rs`: call `initiate_graceful_restart()` when eligible, fall back to current `respawn_agent_pane()` otherwise
- [x] 5.2 Add tick-driven call to `tick_pending_resume()` in the TUI tick handler for instances with `Restarting` status
- [x] 5.3 Handle the `Restarting` status in status bar rendering (display "Restarting..." with spinner)

## 6. Testing

- [x] 6.1 Unit test: `build_agent_command()` with resume token inserts flag correctly for Claude and Codex
- [x] 6.2 Unit test: `ResumeConfig` regex patterns match actual Claude and Codex output formats
- [x] 6.3 Unit test: graceful restart skipped for custom commands and agents without ResumeConfig
- [x] 6.4 Run `cargo fmt`, `cargo clippy`, `cargo test` and fix any issues
