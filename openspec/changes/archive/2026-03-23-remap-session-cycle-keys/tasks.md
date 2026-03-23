## 1. Remove group-scoped cycling and --global flag

- [x] 1.1 Remove `--global` flag from `aoe tmux switch-session` CLI argument definition
- [x] 1.2 Remove `session_cycle_run_shell_cmds()` (non-global variant) and update `session_cycle_run_shell_cmds_with_scope()` to always use global ordering (or inline the global logic directly)
- [x] 1.3 Update the `switch-session` handler to always use global session resolution when `--direction` is specified (remove the conditional branch on `--global`)

## 2. Replace prefix-table n/p/N/P with root-table Ctrl+,/Ctrl+.

- [x] 2.1 In `setup_session_cycle_bindings()`: remove the four `bind-key n/p/N/P` calls and add two `bind-key -T root C-,` / `bind-key -T root C-.` calls with aoe_* session guard (match the pattern used by `C-q` and `C-\;`)
- [x] 2.2 In `apply_managed_session_bindings()`: remove the four `bind-key n/p/N/P` calls and add two `bind-key -T root C-,` / `bind-key -T root C-.` calls using profile-from-option lookup
- [x] 2.3 In `cleanup_session_cycle_bindings()`: remove `n`, `p`, `N`, `P` from the unbind loop and add `C-,` / `C-.` to the root-table unbind loop (alongside `C-\;` and `C-q`)
- [x] 2.4 In `setup_nested_detach_binding()`: update the hardcoded hook command string to remove `unbind-key n ; unbind-key p ; unbind-key N ; unbind-key P` and add `unbind-key -T root C-, ; unbind-key -T root C-.`

## 3. Update specs and documentation

- [x] 3.1 Update `openspec/specs/session-back-toggle/spec.md` to reflect the new keybinding names (apply the delta from the change spec)
- [x] 3.2 Update `AGENTS.md` references from `Ctrl+b N/P` and `Ctrl+b n/p` to `Ctrl+,`/`Ctrl+.`
- [x] 3.3 Update the cross-group-cycling spec if it still lives in `openspec/changes/add-cross-group-session-cycling/` (mark as superseded or update references)

## 4. Verification

- [x] 4.1 Run `cargo fmt` and `cargo clippy` to ensure clean code
- [x] 4.2 Run `cargo test` to verify existing tests pass (update any tests that reference n/p/N/P or --global)
- [x] 4.3 Manually test in non-nested mode: `Ctrl+,` and `Ctrl+.` cycle sessions, `Ctrl+b b` back-toggle works after cycling
- [x] 4.4 Manually test in nested mode: same keybindings work correctly, hook command properly unbinds on session switch away from aoe_*
- [x] 4.5 Verify `Ctrl+,`/`Ctrl+.` pass through in non-aoe sessions (send raw keystrokes to the pane)
