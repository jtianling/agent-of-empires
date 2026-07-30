# Tasks: codex-preregister-identity-key

## 1. Bootstrap Script

- [x] 1.1 In `codex_xats_bootstrap_command`, build the pre-registration as a first attempt carrying `--identity-key "$XATS_IDENTITY_KEY"` (only when the variable is non-empty, branched inside the script) plus `--ttl-seconds <constant>`, with a retry on non-zero exit that reproduces the exact pre-change call; only the second failure takes the existing fatal-diagnostic path.
- [x] 1.2 Name the TTL as a constant beside the other `CODEX_XATS_*` bootstrap constants, set to the daemon's documented 600s ceiling.
- [x] 1.3 Confirm the `exec`ed Codex command line is byte-identical to before this change (no key, no new flags).

## 2. Coverage

- [x] 2.1 Unit: a bootstrap command contains the `--identity-key "$XATS_IDENTITY_KEY"` reference and the TTL flag on the first attempt, and a bare retry; the key appears only as a shell expansion, never as an interpolated value.
- [x] 2.2 Unit: the segment after `exec` contains neither `identity-key` nor `XATS_IDENTITY_KEY` nor `ttl`.
- [x] 2.3 Unit: restart/fork/resume plans keep reapplying the bootstrap with the new call shape (extend the existing `test_codex_xats_*` family).

## 3. Verification

- [x] 3.1 Run `cargo fmt`, `cargo clippy`, and the targeted unit tests (`cargo test --lib instance::tests::test_codex_xats`).
- [x] 3.2 Run `openspec validate codex-preregister-identity-key --strict`.
- [ ] 3.3 On the real machine, after xats ships `--identity-key`: launch a Codex Cross Agent Team session, verify the daemon row carries the identity key, restart with C/R, and verify the daemon pokes the pane and the agent re-registers under its previous `(team, name)`. **Blocked on the xats release; the fallback keeps launches working until then.**
