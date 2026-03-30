## 1. Store @aoe_project_path During Session Creation

- [x] 1.1 Add `append_store_project_path_args(args, target, working_dir)` helper in `src/tmux/utils.rs` that appends `; set-option -t <target> @aoe_project_path <working_dir>` to a tmux argument vector (no `-F` flag needed since the value is a plain path, not a format string)
- [x] 1.2 Call `append_store_project_path_args()` in `create_with_size()` in `src/tmux/session.rs`, right after the existing `append_store_pane_id_args()` call, passing the `working_dir` parameter
- [x] 1.3 Add a unit test in `src/tmux/utils.rs` verifying `append_store_project_path_args` produces the correct argument sequence

## 2. Override % and " Keybindings With if-shell Guard

- [x] 2.1 In `setup_session_cycle_bindings()` in `src/tmux/utils.rs`, add two `bind-key` lines for `%` and `"` using the `if-shell -F "#{m:aoe_*,#{session_name}}"` guard pattern, passing `-c '#{@aoe_project_path}'` to `split-window` in the AoE branch and falling back to plain `split-window` in the else branch
- [x] 2.2 In `cleanup_session_cycle_bindings()` in `src/tmux/utils.rs`, add two lines to restore default tmux bindings: `bind-key % split-window -h` and `bind-key '"' split-window -v` (restore, not unbind)

## 3. Backfill @aoe_project_path for Existing Sessions

- [x] 3.1 In `collect_tag_sessions_with_profile()` in `src/tmux/utils.rs`, add a `set-option` line for each instance that sets `@aoe_project_path` to `instance.project_path` on the generated session name, using `shell_escape()` for the path value

## 4. E2E Test

- [x] 4.1 Create a new e2e test file `tests/e2e/pane_cwd.rs` and register it in `tests/e2e/main.rs`
- [x] 4.2 Write a test that: creates an AoE session via CLI with a known project path, splits the pane using tmux `split-window` with the overridden `%` binding (or directly verifies `@aoe_project_path` is set on the session), and asserts the new pane's `pane_current_path` matches the project path

## 5. Verification

- [x] 5.1 Run `cargo fmt` and `cargo clippy` and fix any warnings
- [x] 5.2 Run `cargo test` (unit + integration tests pass)
- [x] 5.3 Run `cargo test --test e2e -- pane_cwd` to verify the new e2e test passes
