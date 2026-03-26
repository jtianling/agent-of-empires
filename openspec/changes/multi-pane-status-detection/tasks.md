## 1. Pane Info Cache Expansion

- [ ] 1.1 Add `pane_index: u32` and `pane_id: String` fields to `PaneInfo` struct in `src/tmux/mod.rs`
- [ ] 1.2 Change `parse_pane_info_cache_output()` to store `Vec<PaneInfo>` per session (sorted by pane_index) instead of keeping only the lowest-indexed pane. Add `pane_id` to the `tmux list-panes -a -F` format string (`#{pane_id}`).
- [ ] 1.3 Add `get_all_cached_pane_infos(session_name) -> Option<Vec<PaneInfo>>` function
- [ ] 1.4 Update existing `get_cached_pane_info(session_name)` to return the lowest-indexed pane from the new `Vec<PaneInfo>` structure (backwards compatible)

## 2. Process Comm Name Utility

- [ ] 2.1 Add `get_process_comm(pid: u32) -> Option<String>` to `src/process/macos.rs` using `ps -o comm= -p <pid>`
- [ ] 2.2 Add `get_process_comm(pid: u32) -> Option<String>` to `src/process/linux.rs` using `/proc/<pid>/comm`
- [ ] 2.3 Add cross-platform re-export in `src/process/mod.rs`

## 3. Agent Type Detection

- [ ] 3.1 Add shell name constants list (bash, zsh, fish, sh, dash, ksh, tcsh) and `is_shell_command(cmd: &str) -> bool` helper
- [ ] 3.2 Implement `detect_agent_type_from_command(pane_current_command: &str) -> Option<&'static str>` -- match known agent binary names and shell names from `pane_current_command`
- [ ] 3.3 Implement `detect_agent_type_from_pane(pane_info: &PaneInfo) -> Option<&'static str>` -- full detection chain: command match -> pane_pid comm -> foreground PID comm -> fallback to None
- [ ] 3.4 Add unit tests for agent type detection (known agents, shells, ambiguous commands like version numbers)

## 4. Claude Code Content-Based Detection

- [ ] 4.1 Implement `detect_claude_status(content: &str) -> Status` with real content parsing: Running (spinners, tool-use patterns), Waiting (permission prompts, approval dialogs), Idle (input prompt, default)
- [ ] 4.2 Add unit tests for Claude Code detection patterns (Running, Waiting, Idle scenarios)

## 5. Multi-Pane Status Aggregation

- [ ] 5.1 Implement `aggregate_pane_statuses(statuses: &[Status]) -> Status` with priority Waiting > Running > Idle
- [ ] 5.2 Add a new method `update_multi_pane_status()` on Instance that: enumerates all panes via cache, detects agent type per pane, runs per-pane status detection, aggregates results. For the AoE agent pane (pane 0 / `@aoe_agent_pane`), use existing hook+content detection path. For extra panes, use title spinner + content detection with the detected agent's function.
- [ ] 5.3 Wire `update_multi_pane_status()` into `update_status_with_options()` -- after session existence check, if pane_count > 1, delegate to multi-pane path; otherwise use existing single-pane path
- [ ] 5.4 Apply acknowledged-waiting mapping to the aggregated result
- [ ] 5.5 Add unit tests for aggregation logic

## 6. Integration

- [ ] 6.1 Verify TUI status poller works with multi-pane detection (no changes expected if update_status_with_options is updated)
- [ ] 6.2 Verify notification monitor works with multi-pane detection (same path as TUI poller)
- [ ] 6.3 Run `cargo fmt`, `cargo clippy`, and `cargo test` to ensure no regressions
- [ ] 6.4 Manual testing: create a session, split panes, run different agents, verify aggregated status in TUI and notification bar
