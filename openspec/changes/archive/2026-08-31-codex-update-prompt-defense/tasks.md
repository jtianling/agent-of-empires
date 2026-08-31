# Tasks: codex-update-prompt-defense

## 1. Launch hardening (agent-registry)

- [x] 1.1 Add a `fixed_args: &'static [&'static str]` field to `AgentDef` in
  `src/agents.rs`, empty for every agent, with
  `--config check_for_update_on_startup=false` on the codex entry
- [x] 1.2 Apply `fixed_args` in `build_base_pane_command`
  (`src/session/instance.rs`) so every built codex command carries the override;
  confirm the command-override path keeps replacing the command verbatim without
  the flag
- [x] 1.3 Unit tests on the command builder: codex commands on primary / extra
  pane / resume fan-out / fresh restart paths contain the override; a
  cross-agent-team wrapped codex command still contains it inside the wrap; a
  command-override instance does not; claude/other agents do not
- [x] 1.4 Manually verify against the installed codex that an unknown or
  known-false `--config check_for_update_on_startup=false` launches cleanly
  (design D1 risk: upstream key rename tolerance)
  (verified against codex-cli 0.151.0: real key and bogus key both exit 0 on
  `--version`; `resume --help` / `fork --help` list `-c, --config`)

## 2. Shell-fallback detection (status-detection)

- [x] 2.1 Add the fallen-agent check to the status poller
  (`src/tui/status_poller.rs`): recorded agent (slot row, else primary tool) is
  non-shell but `#{pane_current_command}` is a shell after the Starting grace
  period -> instance `Error` with a message naming the fallen pane(s) and the
  restart keys
- [x] 2.2 Implement the exemptions from the delta spec: shell tools, shell
  slots, command overrides resolving to a shell, Starting grace window
- [x] 2.3 Ensure the error clears only through the existing start/restart error
  resets, not by the poller flapping the state back to healthy
- [x] 2.4 Unit tests for the detection predicate (pure logic: recorded agent x
  live command x grace/exemption inputs -> verdict), covering every scenario in
  the delta spec
- [x] 2.5 E2E test: stub agent pane that exits after launch and falls back to a
  shell via the pane-died hook is reported as `Error` in the TUI, and a restart
  clears it (route through `TuiTestHarness`; isolated socket rules apply)
  (executed by aoe-tester in an isolated copy under ~/workspace/test:
  1 passed / 0 failed, exit 0; evidence at
  ~/workspace/test/.e2e-test/runs/fallen-agent-20260831-111440/)

## 3. Verification

- [x] 3.1 `cargo fmt` and `cargo clippy` clean
- [x] 3.2 `cargo build` compiles; do NOT run the full `cargo test` suite on a
  machine with live AoE sessions -- run the new/changed tests individually per
  the tmux-safety rules
- [x] 3.3 `openspec validate codex-update-prompt-defense --strict` passes
