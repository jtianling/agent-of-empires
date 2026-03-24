## 1. Instance struct and serialization

- [x] 1.1 Add `resume_token: Option<String>` field to `Instance` struct in `src/session/instance.rs` with `#[serde(default, skip_serializing_if = "Option::is_none")]`
- [x] 1.2 Initialize `resume_token: None` in `Instance::new()`
- [x] 1.3 Clear `resume_token` in `respawn_agent_pane_with_resume()` after respawn completes (both resume and fresh paths)
- [x] 1.4 Add unit test: old sessions.json without `resume_token` field deserializes to `None`

## 2. Status poller captures token on pane death

- [x] 2.1 Add `resume_token: Option<String>` field to `StatusUpdate` struct in `src/tui/status_poller.rs`
- [x] 2.2 Add `previous_statuses: HashMap<String, Status>` to the polling loop state in `StatusPoller::polling_loop()`
- [x] 2.3 On each poll, detect alive-to-dead transition by comparing current status to previous status; when transitioning to `Error` from a non-`Error` status and the agent has a `ResumeConfig`, capture pane output and extract resume token
- [x] 2.4 Validate extracted token with `is_valid_resume_token()` (expose as `pub(crate)` from `instance.rs`); set to `None` if invalid
- [x] 2.5 Update previous status map after each poll cycle
- [x] 2.6 Set `resume_token: None` in all `StatusUpdate` returns that do not capture a token (container-dead early return, normal polls)

## 3. TUI applies resume token from status updates

- [x] 3.1 In `src/tui/app.rs` where `StatusUpdate` results are applied to instances, store `resume_token` on the Instance when the update carries a non-None token
- [x] 3.2 Do not overwrite an existing stored token when the update's `resume_token` is `None`
- [x] 3.3 Trigger session save after storing a new resume token

## 4. Restart flow uses stored token

- [x] 4.1 Modify `initiate_graceful_restart()` in `src/session/instance.rs`: when pane is dead AND `self.resume_token.is_some()`, call `respawn_agent_pane_with_resume()` with the stored token and return `Ok(true)` instead of `Ok(false)`
- [x] 4.2 In `respawn_agent_pane_with_resume()`, if no `resume_token` argument is provided, fall back to `self.resume_token.as_deref()` before using `None`
- [x] 4.3 Clear `self.resume_token` in `start()` method (new session launch) to prevent stale tokens from a previous lifecycle

## 5. Testing

- [x] 5.1 Unit test: `extract_resume_token` + storage round-trip (serialize Instance with token, deserialize, verify token present)
- [x] 5.2 Unit test: `respawn_agent_pane_with_resume` clears stored token after use
- [x] 5.3 Unit test: dead-pane path in `initiate_graceful_restart` uses stored token when available
- [x] 5.4 Run `cargo fmt`, `cargo clippy`, and `cargo test` to verify no regressions
