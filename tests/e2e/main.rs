//! End-to-end tests for Agent of Empires.
//!
//! These tests exercise the full `aoe` binary -- both TUI mode (via tmux) and
//! CLI subcommands (via subprocess). They catch startup failures, rendering
//! bugs, config resolution errors, and full-flow regressions that unit and
//! integration tests miss.
//!
//! # Running
//!
//! ```sh
//! cargo test --test e2e              # run all e2e tests
//! cargo test --test e2e -- --nocapture  # with screen dumps on failure
//! ```
//!
//! TUI tests require tmux and are skipped automatically if it is not installed.
//! Docker-dependent tests are `#[ignore]` and require a running Docker daemon.

mod harness;

mod agent_session_store;
mod attach_reconcile;
mod cli;
mod errors;
mod fork;
mod legacy_schema_heal;
mod multi_agent_session;
mod multi_pane_restart;
mod new_session;
mod pane_cwd;
mod pane_session_capture;
mod profile_picker;
mod sandbox;
mod session_rename;
mod tui_launch;
mod unified_view;
