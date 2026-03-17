# Repository Guidelines

> `CLAUDE.md` is a symlink to this file. Do not edit `CLAUDE.md` directly -- edit `AGENTS.md` instead.

## Project Structure & Module Organization

- `src/main.rs`: binary entrypoint (`aoe`).
- `src/lib.rs`: shared library code used by the CLI/TUI.
- `src/cli/`: clap command handlers (e.g., `src/cli/add.rs`, `src/cli/session.rs`).
- `src/tui/`: ratatui UI and input handling.
- `src/session/`: session storage, configuration, and group management.
- `src/tmux/`: tmux integration and status detection.
- `src/process/`: OS-specific process handling (`macos.rs`, `linux.rs`).
- `src/docker/`: Docker sandboxing and container management.
- `src/git/`: git worktree operations and template resolution.
- `src/update/`: version checking against GitHub releases.
- `src/migrations/`: versioned data migrations for breaking changes (see below).
- `tests/`: integration tests (`tests/*.rs`).
- `tests/e2e/`: end-to-end tests exercising the full `aoe` binary (see E2E Tests below).
- `docs/`: user-facing documentation and guides.
- `scripts/`: installation and utility scripts.
- `xtask/`: build automation workspace.

## Build, Test, and Development Commands

- `cargo build` / `cargo build --release`: compile (release binary at `target/release/aoe`).
- `cargo build --profile dev-release`: faster optimized builds for local development. Skips LTO for quicker compile times while still producing an optimized binary. Use `--release` for final/CI builds.
- `cargo run --release`: run from source; requires `tmux` installed.
- `cargo check`: fast type-checking during development.
- `cargo test`: run unit + integration tests (some tests skip if `tmux` is unavailable).
- `cargo fmt`: format with rustfmt (run before pushing).
- `cargo clippy`: lint (fix warnings unless there’s a strong reason not to).
- Debug logging: `RUST_LOG=agent_of_empires=debug cargo run` (or `AGENT_OF_EMPIRES_DEBUG=1 cargo run`).

## Settings & Configuration

- **Every configurable field must be editable in the settings TUI.** When adding a new config field to `SandboxConfig`, `WorktreeConfig`, etc., you must also:
  1. Add a `FieldKey` variant in `src/tui/settings/fields.rs`
  2. Add a `SettingField` entry in the corresponding `build_*_fields()` function
  3. Wire up `apply_field_to_global()` and `apply_field_to_profile()`
  4. Add a `clear_profile_override()` case in `src/tui/settings/input.rs`
- Profile overrides (`*ConfigOverride` structs in `profile_config.rs`) must also include the new field with merge logic in `merge_configs()`.

## Coding Style & Naming Conventions

- Prefer "let the tools decide": keep code `cargo fmt`-clean and `cargo clippy`-clean.
- **Never use emdashes (—)** in documentation or comments.
- Rust naming: `snake_case` for modules/functions, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Keep OS-specific logic in `src/process/{macos,linux}.rs` rather than sprinkling `cfg` checks.
- Do not be concerned about maintaining backwards compatibility. You should not assume that it needs to be backwards compatible, but you should mention when you make a change that breaks backwards compatibility.
- Add comments where they aid understanding, but remove obvious ones before finishing:
  - **Keep**: comments explaining non-obvious formulas, layout structure documentation, or "why" something is done
  - **Remove**: section headers that just name what the next line does (e.g., `// Render buttons` before `render_buttons()`), or comments restating the code

## Testing Guidelines

- Use unit tests in-module (`#[cfg(test)]`) for pure logic; use `tests/*.rs` for integration tests.
- Tests must be deterministic and clean up after themselves (tmux tests should use unique names like `aoe_test_*` or `aoe_e2e_*`).
- Avoid reading/writing real user state; prefer temp dirs (see `tempfile` usage in `src/session/storage.rs`).
- New features touching TUI rendering, CLI subcommands, or session lifecycle should consider adding an e2e test.

### E2E Tests

Full-binary end-to-end tests live in `tests/e2e/`. They exercise `aoe` through tmux (for TUI tests) and as a subprocess (for CLI tests). Run them with:

```sh
cargo test --test e2e              # all e2e tests
cargo test --test e2e -- --nocapture  # with screen dumps on failure
```

The test harness (`tests/e2e/harness.rs`) provides `TuiTestHarness` with:
- `spawn_tui()` / `spawn(args)` -- launch `aoe` in a detached tmux session with isolated `$HOME`
- `send_keys(keys)` / `type_text(text)` -- send keystrokes or literal text
- `wait_for(text)` -- poll the screen until text appears (10s timeout, panics with screen dump)
- `capture_screen()` / `assert_screen_contains(text)` -- one-shot screen assertions
- `run_cli(args)` -- run `aoe` as a subprocess with the same env isolation

TUI tests auto-skip if tmux is not installed. Docker-dependent tests use `#[ignore]` and require a running daemon. All tests use `#[serial]` for tmux isolation.

#### Recording E2E Tests

E2E tests can produce asciinema recordings (`.cast`) and GIF files automatically. This is useful for PR reviews and documenting TUI behavior.

- **Local**: `RECORD_E2E=1 cargo test --test e2e -- --nocapture` (requires `asciinema` and `agg` on `$PATH`). Outputs go to `target/e2e-recordings/`.
- **CI**: Add the `needs-recording` label to a PR. The `E2E Recordings` workflow will run the tests with recording enabled and upload GIF artifacts.

## Commit & Pull Request Guidelines

- Branch names: `feature/...`, `fix/...`, `docs/...`, `refactor/...`.
- Commit messages: use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`).
- PRs: follow the template in `.github/pull_request_template.md`. When creating PRs via `gh pr create`, read the template first and use its structure for the `--body` argument. Include a clear “what/why”, how you tested (`cargo test`, plus any manual tmux/TUI checks), and screenshots/recordings for UI changes.

## Git Configuration

- Do not modify git configuration (e.g., `.gitconfig`, `.git/config`, `git config` commands) without explicit user approval.
- The one exception: adding a new remote to fetch a contributor's fork during PR code review is allowed without asking.

## Local Data & Configuration Tips

- Runtime config/data location:
  - **Linux**: `$XDG_CONFIG_HOME/agent-of-empires/` (defaults to `~/.config/agent-of-empires/`)
  - **macOS/Windows**: `~/.agent-of-empires/`
- Keep user data out of commits. For repo-local experiments, use ignored paths like `./.agent-of-empires/`, `.env`, and `.mcp.json`.

## Tmux Nested vs Non-Nested Environments

AoE must support two distinct runtime modes for tmux integration, and bugs have repeatedly occurred from only handling one path:

- **Nested mode** (`TMUX` env var is set): AoE is running inside an existing tmux session. Attach uses `switch-client`, and `setup_nested_detach_binding()` configures the custom `d` key and the `client-session-changed` hook.
- **Non-nested mode** (`TMUX` env var is NOT set): AoE attaches via `attach-session`. Only `setup_session_cycle_bindings()` runs; nested detach hooks are not installed.

When adding or modifying tmux key bindings, session options, or attach behavior:

1. **Always verify both paths.** Code in `apply_managed_session_bindings()` only runs in nested mode. Code in `setup_session_cycle_bindings()` runs in both modes. Choose the right location based on whether the feature should work everywhere or only when nested.
2. **Test both modes manually** when feasible: start AoE from a bare terminal (non-nested) and from within an existing tmux session (nested).
3. **Key binding lifecycle**: bindings set on attach must be cleaned up on exit (`cleanup_session_cycle_bindings`) and on session switch (the `client-session-changed` hook's non-managed branch).

## Terminal Title Handling

- Treat outer terminal title handling and tmux pane title handling as separate concerns.
- For the AoE TUI itself, use the stable title `Agent of Empires`. Do not reintroduce per-view or per-dialog dynamic TUI titles unless the spec is updated first.
- When AoE starts, preserve the pre-launch terminal title and restore it on normal exit and panic cleanup. For Alacritty, prefer the title stack sequences already used in `src/tui/tab_title.rs`.
- For AoE-managed tmux sessions, make the outer terminal title follow the active pane title through session-scoped tmux options such as `set-titles` and `set-titles-string=#T`.
- Preserve the existing `sets_own_title` split in `src/agents.rs`: agents that manage their own titles should keep doing so, and AoE should only manage pane titles for agents that do not.
- Do not mutate unrelated global tmux defaults just to make title propagation work. Scope title changes to AoE-managed sessions.
- If you change terminal-title behavior, add or update runtime coverage for tmux title propagation and title restoration, and manually sanity-check the behavior in Alacritty when feasible.

### Code-Level Implementation Notes

- Keep TUI title-writing helpers in `src/tui/tab_title.rs`. Add low-level escape-sequence writes there instead of scattering raw OSC/CSI strings through the TUI.
- `src/tui/mod.rs` owns the TUI title lifecycle. Startup should save the incoming title and set the stable AoE title; normal teardown and panic cleanup should restore the saved title.
- `src/tui/app.rs` should only reapply the stable AoE title when returning from attach flows. Use `terminal.backend_mut()` for title writes in attach/detach paths, not direct `std::io::stdout()` calls.
- `src/tmux/status_bar.rs` owns tmux session-scoped title passthrough. If attached sessions need to affect the outer terminal title, wire that through tmux options there rather than adding new TUI polling logic.
- `src/tui/status_poller.rs` owns AoE-managed pane-title refresh for agents with `sets_own_title = false`. If waiting/running states should affect pane titles, change that file instead of teaching the TUI to infer agent titles.
- `src/agents.rs` is the source of truth for whether an agent sets its own title. Update `sets_own_title` there before adding any special-case title logic elsewhere.
- If existing sessions need new tmux title behavior without being recreated, refresh session options through `src/session/instance.rs` attach helpers and the CLI attach path in `src/cli/session.rs`.
- Prefer session-scoped tmux option changes such as `set-option -t <session> ...` or `set-window-option -t <session> ...`. Avoid `-g` unless the behavior is intentionally global.
- Add focused unit tests for title helpers near the implementation, and add integration or e2e coverage in `tests/e2e/` for tmux title passthrough or restoration behavior.

### Reference Docs

- Alacritty escape sequence support: `https://raw.githubusercontent.com/alacritty/alacritty/master/docs/escape_support.md`
- tmux title and pane-title behavior: `https://github.com/tmux/tmux/wiki/Advanced-Use#pane-titles-and-the-terminal-title`
- tmux manual reference for `set-titles`, `set-titles-string`, and `allow-set-title`: `https://man7.org/linux/man-pages/man1/tmux.1.html`
- xterm control sequences for OSC title setting and CSI `22`/`23` title stack save/restore: `https://invisible-island.net/xterm/ctlseqs/ctlseqs.pdf`

## Data Migrations

When making breaking changes to stored data (file locations, config schema, etc.), use the migration system in `src/migrations/` instead of adding fallback/compatibility logic to the main code.

**Why**: Keeps the main codebase clean. Legacy transition logic is isolated and clearly marked as such.

**How it works**:
1. A `.schema_version` file in the app directory tracks the current version
2. On startup, `migrations::run_migrations()` runs any pending migrations in order
3. Each migration bumps the version after completion

**Adding a new migration**:

1. Create `src/migrations/vNNN_description.rs`:
   ```rust
   use anyhow::Result;

   pub fn run() -> Result<()> {
       // Migration logic here
       Ok(())
   }
   ```

2. Update `src/migrations/mod.rs`:
   ```rust
   mod vNNN_description;

   const CURRENT_VERSION: u32 = NNN;  // bump this

   const MIGRATIONS: &[Migration] = &[
       // ... existing migrations ...
       Migration { version: NNN, name: "description", run: vNNN_description::run },
   ];
   ```

**Guidelines**:
- Migrations must be idempotent (safe to run multiple times)
- Use `tracing::info!` to log what's happening
- Platform-specific migrations should use `#[cfg(target_os = "...")]`
- Test migrations by creating the old state manually and verifying the transition
- Before finishing any feature request, make sure that you have run cargo fmt, clippy, and tests.
