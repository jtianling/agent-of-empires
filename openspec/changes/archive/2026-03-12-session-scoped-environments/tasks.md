## 1. Environment Key Resolution

- [x] 1.1 Add `resolve_profile()` function in `src/session/mod.rs` that implements the priority chain: explicit profile -> tmux session name -> canonical working directory
- [x] 1.2 Implement `get_tmux_session_name()` helper that runs `tmux display-message -p '#S'` when `TMUX` env var is set, returning `None` on failure
- [x] 1.3 Implement `sanitize_profile_name()` that lowercases, replaces non-alphanumeric chars with hyphens, trims leading/trailing hyphens, and collapses consecutive hyphens
- [x] 1.4 Implement `short_hash()` that returns the first 4 hex chars of SHA-256 of a canonical path string
- [x] 1.5 Implement auto-profile naming: `auto-<sanitized_name>` for tmux, `auto-<sanitized_dir>-<hash>` for directory

## 2. Auto-Profile Creation

- [x] 2.1 Add logic in `resolve_profile()` to auto-create the profile directory (with empty `sessions.json` and `groups.json`) when the resolved profile does not exist
- [x] 2.2 Ensure auto-created profiles are valid and appear in `aoe profile list`

## 3. Integration with Main Entry Point

- [x] 3.1 Replace `cli.profile.unwrap_or_default()` in `src/main.rs` with `resolve_profile(cli.profile)` call
- [x] 3.2 Ensure all profile-consuming code paths (CLI subcommands and TUI) receive the resolved profile

## 4. Migration Hint

- [x] 4.1 After profile resolution, check if the `default` profile has sessions while the resolved auto-profile is empty
- [x] 4.2 Display a one-time hint message directing users to `aoe -p default` to access existing sessions

## 5. Testing

- [x] 5.1 Unit tests for `sanitize_profile_name()` covering special characters, spaces, unicode, empty strings
- [x] 5.2 Unit tests for `short_hash()` determinism and uniqueness for different paths
- [x] 5.3 Unit tests for `resolve_profile()` covering all three branches (explicit, tmux, directory)
- [x] 5.4 Integration test verifying auto-profile creation creates valid profile directory
- [x] 5.5 Test that explicit `--profile` flag bypasses environment scoping

## 6. Cleanup and Verification

- [x] 6.1 Run `cargo fmt` and `cargo clippy` to ensure code quality
- [x] 6.2 Run full test suite `cargo test` to verify no regressions
- [ ] 6.3 Manual verification: run `aoe` in different tmux sessions and directories, confirm isolation
