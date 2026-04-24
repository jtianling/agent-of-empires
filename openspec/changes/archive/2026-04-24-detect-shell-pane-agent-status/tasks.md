## 1. In-memory session field

- [x] 1.1 Add a non-persistent `detected_inner_agent: Option<String>` field to the in-memory session instance in `src/session/instance.rs`. Mark it `#[serde(skip, default)]` (or equivalent) so it is never serialized to disk.
- [x] 1.2 Initialize the field to `None` in every constructor path (new session, reload from disk, test fixtures).

## 2. Detection trigger on detach-return

- [x] 2.1 In `src/tui/app.rs::attach_session`, after `with_raw_mode_disabled(..., tmux_session.attach())` returns and before `self.home.reload()`, add a branch: if the session's tool is `shell`, look up the primary `PaneInfo` and call `tmux::status_detection::detect_agent_type_from_pane`.
- [x] 2.2 Normalize the result: `Some("shell")` or `None` → set the session's `detected_inner_agent` to `None`; any other `Some(x)` → set to `Some(x.to_string())`. Write the result back through the instance-update channel used by the rest of the TUI (do not mutate the clone in place).
- [x] 2.3 For tools other than `shell`, the new code path MUST be a no-op (no detection, no write).

## 3. Status-detection dispatch in `update_status_with_options`

- [x] 3.1 In `src/session/instance.rs::update_status_with_options`, before the existing `session.detect_status(tool)` call for the primary pane, check whether `self.tool == "shell"` and `self.detected_inner_agent.is_some()`. If both, capture the pane content and call `tmux::status_detection::detect_status_from_content(content, inner_agent, Some(title))` instead.
- [x] 3.2 Ensure the captured primary-pane content uses the same capture parameters and caching path as the existing shell-tool capture, to avoid double-capturing.
- [x] 3.3 Adjust the `Status::Idle → Status::Unknown` rewrite block (currently at `src/session/instance.rs:1218-1228`) so it does NOT rewrite when `detected_inner_agent.is_some()`. A concrete `Idle` from a real agent detector must surface as `Idle`.
- [x] 3.4 Leave all other primary-pane logic (spinner grace period, spike detection, dead-pane → Error, acknowledged mapping) untouched. They apply uniformly to the new path.
- [x] 3.5 Do not touch the `detect_extra_pane_statuses` path — extra panes continue to work as they do today.

## 4. Status-polling non-mutation guarantee

- [x] 4.1 Verify (and add a unit test) that no code path inside `update_status_with_options`, `detect_extra_pane_statuses`, or any status-poller callback writes to `detected_inner_agent`. Only the attach-return path in `src/tui/app.rs` writes to it.

## 5. Tests

- [x] 5.1 Unit test: `detected_inner_agent` default is `None` on construction, survives an `update_status` call without being touched, and is cleared to `None` when a helper simulating the attach-return path is invoked with a shell primary pane.
- [x] 5.2 Unit test: for a shell session with `detected_inner_agent = Some("claude")`, `update_status_with_options` dispatches to the claude content detector. Use a fixture pane content for a known claude state (Running and Idle).
- [x] 5.3 Unit test: for a shell session with `detected_inner_agent = Some("claude")` whose claude detector returns `Idle`, the final status is `Idle` (NOT rewritten to `Unknown`).
- [x] 5.4 Unit test: for a shell session with `detected_inner_agent = None`, status behavior is byte-identical to today's shell-tool path (ends up `Unknown`).
- [x] 5.5 Unit test: `detected_inner_agent` is not serialized — round-trip through JSON/save-load produces `None` after reload.

## 6. Verify & lint

- [x] 6.1 `cargo fmt`
- [x] 6.2 `cargo clippy --all-targets --all-features`
- [x] 6.3 `cargo test`
