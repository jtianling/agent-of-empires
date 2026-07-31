## 1. Remove the unkeyed retry

- [x] 1.1 In `codex_xats_bootstrap_command_for`, drop the retry that re-runs the pre-registration without the identity key. A failure from either the keyed or the unkeyed first attempt prints the diagnostic and exits non-zero.
- [x] 1.2 Keep the keyed/unkeyed split on the first attempt. A pane with no key yet still pre-registers without one.
- [x] 1.3 Give the diagnostic a named constant (`CODEX_XATS_PREREGISTER_FAILED`) so tests can assert on the exact text, and document there why the failure is terminal.

## 2. Coverage

- [x] 2.1 `test_codex_xats_preregister_first_attempt_carries_key_and_ttl`: exactly two `pre-register-codex-pane` call sites (the keyed and keyless branches of the one attempt), and both carry `--ttl`. A call without a TTL is the retry shape, so this catches a reintroduction by argv rather than by count alone.
- [x] 2.2 `test_codex_xats_keyed_preregister_failure_is_not_retried_without_the_key`: with a live key and a first-attempt failure, exactly one npx call, that call carries `--identity-key-env`, the launch fails, and Codex never execs.
- [x] 2.3 `test_codex_xats_bootstrap_preregister_failure_is_fatal_without_codex`: one attempt, fatal, no Codex.
- [x] 2.4 `test_codex_xats_bootstrap_executes_keyless_preregister` (existing) still covers a pane with no key registering without one.
- [x] 2.5 Kept the errexit reproduction, retargeted: under an inherited `SHELLOPTS=errexit`, the failure must still reach its diagnostic rather than dying silently at the failing `npx`. The hazard outlived the retry it was written for, so the test was rewritten rather than deleted. Required a `run_codex_bootstrap_capturing_stderr` variant; the existing helper delegates to it.

## 3. Verification

- [x] 3.1 `cargo fmt`, `cargo clippy --all-targets` clean. Name-filtered unit tests: 16 in `session::instance::tests::test_codex_xats`, 194 in `session::instance::tests`, all passing. No e2e assertion referenced the retry. The full suite and e2e were NOT run, because this machine hosts live AoE tmux sessions.
- [ ] 3.2 jt's acceptance.
