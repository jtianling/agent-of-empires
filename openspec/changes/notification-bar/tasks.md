## 1. Status Bar Format Update

- [x] 1.1 Modify `STATUS_LEFT_FORMAT` in `src/tmux/status_bar.rs` to include tmux conditional `#{?#{@aoe_waiting}, | #[fg=colour220]#{@aoe_waiting}#[fg=colour245],}` after the detach hint
- [x] 1.2 Increase `status-left-length` from 80 to 160 in `apply_status_bar()`

## 2. CLI Subcommand

- [x] 2.1 Add `MonitorNotifications` variant to `TmuxCommands` enum in `src/cli/tmux.rs` with `--profile` argument
- [x] 2.2 Add `run_monitor_notifications()` handler that delegates to `tmux::status_bar::run_notification_monitor()`
- [x] 2.3 Wire the new subcommand in `src/main.rs` match arm

## 3. Notification Monitor Daemon

- [x] 3.1 Add `ensure_notification_monitor()` in `src/tmux/status_bar.rs` following the `ensure_codex_title_monitor` pattern: check PID in tmux server option `@aoe_notification_monitor_pid`, spawn `aoe tmux monitor-notifications --profile <profile>` if not running
- [x] 3.2 Implement `run_notification_monitor()` main loop: every 2 seconds, load instances and group tree from disk, list all `aoe_*` tmux sessions, compute which are Waiting or Idle (using persisted status from sessions.json), apply group collapse filtering (Idle in collapsed group excluded, Waiting always included), compute indices using the same sort order as `update_session_index()`, format `[index] title` text per session (excluding self), set `@aoe_waiting` on each session (empty string if no notifications for that session), exit when no aoe sessions remain or PID ownership lost
- [x] 3.3 Add cleanup: unset `@aoe_waiting` on all sessions when monitor exits, unset `@aoe_notification_monitor_pid` server option

## 4. Lifecycle Integration

- [x] 4.1 Call `ensure_notification_monitor()` from `apply_all_tmux_options()` in `src/tmux/status_bar.rs` (so the monitor is started when any session is created)
- [x] 4.2 Add cleanup of `@aoe_waiting` in TUI exit path (alongside existing keybinding cleanup)

## 5. Testing

- [x] 5.1 Add unit tests for notification formatting logic: format with multiple sessions, format excluding self, format with empty list
- [x] 5.2 Add unit tests for Idle/Waiting filtering with collapsed group logic
