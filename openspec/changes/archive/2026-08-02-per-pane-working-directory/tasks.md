## 1. Field-Index Convergence (precondition)

- [x] 1.1 Compute the new session dialog's field layout once and have `handle_key`, `render`, and `current_input_mut` consume that single result instead of each walking the conditional sequence with its own counter. Keep `path_field()` derived from it.
- [x] 1.2 Replace the hardcoded field indices in `new_session/tests.rs` with lookups against the computed layout, so a later conditional field cannot silently invalidate a test's assumption about which field it focused.
- [x] 1.3 Add coverage that focus lands on the intended field for the conditional combinations that already exist (tool selection present or absent, YOLO shown or hidden, Cross Agent Team shown or hidden, worktree branch empty or set, sandbox available or not). This is the regression net for every conditional field added afterwards.

## 2. Reusable Path Field

- [x] 2.1 Add a path field component in `src/tui/components/` bundling the input, its ghost completion, and its invalid-path flash, exposing key handling (segment jumps, ghost acceptance at end of input, Home/Ctrl+A), directory-picker activation, and tilde-expanded resolution.
- [x] 2.2 Move the new session dialog's session path field onto the component with no behavior change, and confirm the existing path tests pass untouched. Do this before any new field exists, so the suite is judging a pure move.
- [x] 2.3 Replace `workspace_repo_dir_picker_active: bool` with an explicit picker target, so a selected directory is routed to the field the picker was opened from. Cover routing for each target.
- [x] 2.4 Confirm the component leaves `PathGhostCompletion` a standalone reusable component in its own right, as its capability spec requires.

## 3. A Launched Pane's Own Working Directory

- [x] 3.1 Move the launched pane's working directory onto `LaunchedPane`, and keep the primary pane's directory a separate parameter of `record_launched_extra_pane` sourced from the instance.
- [x] 3.2 Have the launch path write each directory to its own slot: the launched pane's to its record, the instance's to the primary record it writes when absent.
- [x] 3.3 Cover that a pane launched into a directory other than the instance's records that directory, and that the primary record beside it still records the instance's.
- [x] 3.4 Cover that a restart returns such a pane to its recorded directory while the primary pane returns to the session's. This is the assertion that would have caught the defect: the split alone looks correct without it.
- [x] 3.5 Confirm a capture reporting a different directory still updates the slot and still preserves its identity key.

## 4. New Session Dialog

- [x] 4.1 Add `right_pane_path` to `NewSessionData` and widen the pending right-pane channel in `HomeView` from a tool name to a tool and an optional directory. All three call sites that stage it today (sandboxed/hooked creation, direct creation, fork) must carry it.
- [x] 4.2 Add the Right Pane Path field below the Right Pane field, shown only when a right pane tool other than "none" is selected and the session is not sandboxed, built on the component from task 2.
- [x] 4.3 Resolve an empty value to the session's `project_path` at the moment of the split, not at submit. Cover that a worktree-backed session's right pane follows the resolved worktree path, and that an explicitly named directory is used as given and not worktree-resolved.
- [x] 4.4 Pass the resolved directory to the split in the attach flow, replacing the hardcoded `inst.project_path`.
- [x] 4.5 Surface a split that fails rather than logging it, now that the directory can be one the user typed. A pane that silently does not appear is the failure mode this field introduces.
- [x] 4.6 Make the create-directory confirmation cover every missing directory in one prompt, creating all of them on confirm and none on decline. Cover: both missing, only the right pane path missing, and decline creating nothing.
- [x] 4.7 Cover that toggling sandboxing hides and restores the field, and that a hidden field contributes no value to the submitted data.

## 5. Fork Dialog

- [x] 5.1 Add a right pane path field to the fork dialog on the same component, and host a directory picker there (the dialog has none today).
- [x] 5.2 Add `right_pane_path` to `ForkSessionData` and carry it through the fork creation path.
- [x] 5.3 Cover that an empty value puts the forked right pane in the parent's working directory, and that a named directory is used instead.

## 6. TUI Add-Pane Action

- [x] 6.1 Add an add-pane dialog offering the agent (defaulting to the session's tool) and the working directory (defaulting to the session's), on the component from task 2.
- [x] 6.2 Bind `%` on the home screen to open it for the selected session. Confirm `%` is not added to any tmux key table, so `prefix + %` inside attached sessions is untouched.
- [x] 6.3 Perform the four-slot cap and not-running checks before opening the dialog, matching the CLI's refusals, and surface each rather than working around it.
- [x] 6.4 Attach to the session once the pane is up. Cover that cancelling creates nothing and stays on the home list.
- [x] 6.5 Cover the no-ops: a group header selected, and a session with status `Deleting`.
- [x] 6.6 Add `%` to the help overlay.

## 7. CLI

- [x] 7.1 Accept `--path` and `--tool` on `aoe session add-agent-pane`, defaulting to the session's working directory and the session's own tool.
- [x] 7.2 Confirm the added pane is still built as a non-primary pane, so a named tool cannot pick up the instance's command override, pre-allocated session id, fork token, or identity key.
- [x] 7.3 Re-run `cargo xtask gen-docs` and confirm `docs/cli/reference.md` is in sync.

## 8. Documentation

- [x] 8.1 Document the right pane path in the user-facing session-creation docs, including that an empty value follows the session (worktree included) and that sandboxed sessions do not offer it.
- [x] 8.2 Document `%` alongside the other home-screen keybindings, and state explicitly how it differs from `prefix + %` inside an attached session. Finding those two separately is what makes them read as a contradiction.

## 9. Runtime Acceptance

- [x] 9.1 Add e2e coverage that a right pane created with its own directory actually starts there, read from the pane rather than from the dialog, with the left pane still in the session's directory.
- [x] 9.2 Add e2e coverage that a restart returns both panes to their own directories. Task 3.4 covers this at the store level; this covers it end to end, which is where the original defect was observable.
- [x] 9.3 Add e2e coverage for the `%` flow: dialog opens, pane is created with the chosen agent and directory, AoE attaches.
- [x] 9.4 Route every tmux-touching test through `TuiTestHarness` or `isolate_tmux_socket()`, with `$TMUX` and `$TMUX_PANE` removed, per `AGENTS.md`. No `kill-server` and no prefix-based session sweeps anywhere in this change.
- [x] 9.5 Before finishing: `cargo fmt`, `cargo clippy`, and the test suite. Run the full suite only when no live AoE sessions are at risk; otherwise limit local verification to `cargo build` / `cargo check` and the isolated-socket tests.
