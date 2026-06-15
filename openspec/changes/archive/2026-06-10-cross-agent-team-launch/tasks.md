## 1. Configuration

- [x] 1.1 Add `cross_agent_team_channel: String` (default `server:cross-agent-teams-channel`) and `cross_agent_team_default: bool` (default false) to the session config in `src/session/config.rs`
- [x] 1.2 Add matching `*Override` fields in `src/session/profile_config.rs` and extend `merge_configs()` with their merge logic
- [x] 1.3 Add `FieldKey` variants and `SettingField` entries in `src/tui/settings/fields.rs`, and wire `apply_field_to_global()` + `apply_field_to_profile()`
- [x] 1.4 Add `clear_profile_override()` cases for the new fields in `src/tui/settings/input.rs`

## 2. Data model and command building

- [x] 2.1 Add serde-persisted `cross_agent_team: bool` (default false) to `Instance` in `src/session/instance.rs`
- [x] 2.2 In `build_agent_command`, on the non-sandboxed claude path, append `--dangerously-load-development-channels <channel>` (after the YOLO flag when present) when `cross_agent_team` is set; never on the sandbox path
- [x] 2.3 Confirm no `CROSS_AGENT_TEAMS_MCP_TOKEN` injection is added (inherited from environment)
- [x] 2.4 Add unit tests for command building: enabled/disabled, with/without YOLO, custom channel value

## 3. New Session dialog

- [x] 3.1 Add `cross_agent_team` to `NewSessionData` and thread it through `build_instance` in `src/session/builder.rs`
- [x] 3.2 Add dialog state (`cross_agent_team`, default from `cross_agent_team_default`) and a `has_cross_agent_team_field()` helper (claude-only, hidden when Sandbox enabled) in `src/tui/dialogs/new_session/mod.rs`
- [x] 3.3 Insert the new focusable field immediately after `yolo_mode_field` in tab order; update all dependent field-index calculations in `handle_key`
- [x] 3.4 Render the "Cross Agent Team" checkbox on the YOLO Mode row, to its right, with focus highlight, in `src/tui/dialogs/new_session/render.rs`
- [x] 3.5 Add the field help entry for "Cross Agent Team"
- [x] 3.6 Add dialog tests: visibility (claude vs non-claude, sandbox on/off), default state, focus order with field present and absent

## 4. Auto-confirm startup screens

- [x] 4.1 Add `spawn_auto_confirm` (detached background thread) on `Instance`; chosen over a TUI-loop tick because attach suspends the loop (see design D4)
- [x] 4.2 Thread polls `capture_pane`, sends Enter only while a recognized confirmation marker is present, exits on done-grace or 20s timeout
- [x] 4.3 Define the confirmation markers in a single constant (`AUTO_CONFIRM_MARKERS`)
- [x] 4.4 Spawn auto-confirm on initial launch (`start_with_size_opts`)
- [x] 4.5 Spawn auto-confirm on `R` restart for both graceful-resume and kill-and-recreate (`respawn_agent_pane_with_resume` + `start_with_size_opts`)
- [x] 4.6 Add unit tests for marker detection (positive/negative) and no-op spawn for non-cross-agent-team

## 5. Validation

- [x] 5.1 Run `cargo fmt`, `cargo clippy`, and `cargo test` (fmt clean, clippy clean, 1134 lib + integration tests pass)
- [x] 5.2 Run `cargo xtask gen-docs` if any clap help text changed (N/A: no clap help changed)
- [x] 5.3 Manual tmux/TUI check: New Session dialog shows the Cross Agent Team checkbox next to YOLO and toggles (verified via tmux smoke test)
