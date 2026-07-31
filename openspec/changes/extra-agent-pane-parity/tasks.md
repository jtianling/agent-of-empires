## 1. Launch: one builder for every agent pane

- [x] 1.1 Add `Instance::build_extra_pane_command`, which builds `shell` locally and delegates every other tool to `build_pane_command(tool, None, false, None)`. Both call sites use it, so the right pane and the CLI cannot drift apart again.
- [x] 1.2 In `src/tui/app.rs`, drop `build_right_pane_command` and call the new builder. A tool it cannot launch is reported and no pane is created, rather than splitting an empty pane with nothing saying why.
- [x] 1.3 In `src/cli/session.rs`, `add_agent_pane` builds a non-primary pane instead of `build_agent_command(None)`.
- [x] 1.4 Give `pane_base_command` an `is_primary` argument so the Codex bootstrap for a non-primary pane starts from that agent's own binary rather than the instance's command override (see design Decision 5).
- [x] 1.5 Move the two `build_right_pane_command` unit tests to `src/session/instance.rs` and adjust the non-shell one: the shared builder wraps with a login shell and `exec env`, where the old builder used `bash -c`.

## 2. Launch coverage

- [x] 2.1 Unit test: a Cross Agent Team session's `codex` extra pane carries the pane pre-registration and the app-server connection; a `claude` extra pane carries its channel flag.
- [x] 2.2 Unit test: an instance carrying a command override, a pre-allocated session id, and an identity key produces an extra-pane command containing none of them and no `XATS_IDENTITY_KEY`. Covered for both the plain path (claude) and the bootstrap path (codex).
- [x] 2.3 Unit test: a `shell` right pane still resolves the user's shell and starts in the session's working directory.

## 3. Capture: bind every Codex pane

- [x] 3.1 Give `codex_rollout::maybe_claim_for_pane` an `is_primary` input and apply the instance-level conditions only when it is set, through a pure `instance_permits_claim` predicate. The per-pane evidence checks are unchanged.
- [x] 3.2 In `src/db/reconcile.rs`, attempt the claim for every pane of the session in `list_session_panes` order, passing `is_primary` for the pane that matches `@aoe_agent_pane`.
- [x] 3.3 Unit test: two panes of one session take different conversations, and a later pane does not take an earlier pane's conversation.
- [x] 3.4 Unit test: the instance-level conditions answer for the primary pane only (non-codex tool and command override both stop the primary and neither stops an extra pane).

## 4. Verification

- [x] 4.1 `cargo fmt`, `cargo clippy --all-targets`, `cargo check --all-targets` clean. Unit tests run name-filtered only (`extra_*`, `codex_xats`, `cat_integration`, `identity_key`, `db::reconcile`, `db::codex_rollout`): 9 new pass, 67 related existing pass. The full suite and all e2e tests were NOT run: this machine hosts live AoE tmux sessions.
- [ ] 4.2 Hand off to jt for real-machine acceptance: create a Cross Agent Team session with a Codex right pane, confirm both panes pre-register with xats, then `Shift+C` and confirm both panes restart.
