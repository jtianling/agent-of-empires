## 1. Tool-Specific Cross Agent Team Runtime

- [x] 1.1 Split the existing Claude-only Cross Agent Team checks into supported-tool, Claude, and Codex helpers without changing persisted instance fields.
- [x] 1.2 Add a pane-local Codex xats bootstrap that validates dependencies, pre-registers `TMUX_PANE` with a fresh UUID, and launches Codex against the local app-server without exposing token values.
- [x] 1.3 Apply the Codex xats bootstrap through the shared command builder so fresh launch, resume restart, fresh restart, and fork preserve native Codex arguments and independent YOLO behavior.
- [x] 1.4 Keep Claude development-channels flags and auto-confirm behavior isolated from the Codex path, and preserve the normal Codex path when Cross Agent Team is disabled.

## 2. New Session UI

- [x] 2.1 Show the Cross Agent Team field for non-sandboxed Claude and Codex primary tools while keeping it hidden for unsupported tools and Sandbox sessions.
- [x] 2.2 Update tool-neutral field help and submission behavior so hidden or unsupported field state cannot enable Cross Agent Team accidentally.

## 3. Automated Coverage

- [x] 3.1 Extend New Session dialog tests for Codex visibility, toggling, default state, Sandbox hiding, unsupported tools, and YOLO independence.
- [x] 3.2 Add command-construction tests for Codex fresh, YOLO, non-YOLO, resume, fork, explicit bootstrap failures, and disabled-mode behavior.
- [x] 3.3 Preserve and run Claude Cross Agent Team tests to prove its flag and auto-confirm paths do not regress.
- [x] 3.4 Add or update isolated tmux E2E coverage for creating a Codex Cross Agent Team session with controlled xats and Codex shims, including failure diagnostics and cleanup by exact session name.

## 4. Verification

- [x] 4.1 Run `cargo fmt -- --check`, `cargo clippy`, and the relevant unit and E2E test suites.
- [x] 4.2 Run OpenSpec verification and confirm every requirement scenario is covered by implementation evidence.
