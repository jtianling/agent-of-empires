//! Terminal User Interface module

mod app;
mod components;
mod creation_poller;
mod deletion_poller;
pub mod dialogs;
pub mod diff;
mod home;
pub mod settings;
mod status_poller;
mod styles;
mod tab_title;

pub use app::*;

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io::{self, Write};

use crate::migrations;
use crate::session::get_update_settings;
use crate::update::check_for_update;

pub async fn run(profile: &str) -> Result<()> {
    // Capture the directory where the user launched aoe, before anything changes cwd
    let launch_dir = std::env::current_dir().unwrap_or_default();

    // Run pending migrations with a spinner so users see progress
    if migrations::has_pending_migrations() {
        const SPINNER_FRAMES: &[char] = &['◐', '◓', '◑', '◒'];
        let migration_handle = tokio::task::spawn_blocking(migrations::run_migrations);
        tokio::pin!(migration_handle);
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(120));
        let mut frame = 0usize;
        loop {
            tokio::select! {
                result = &mut migration_handle => {
                    print!("\r\x1b[2K");
                    let _ = io::stdout().flush();
                    result??;
                    break;
                }
                _ = tick.tick() => {
                    print!("\r  {} Running data migrations...", SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]);
                    let _ = io::stdout().flush();
                    frame += 1;
                }
            }
        }
    }

    // Check for tmux
    if !crate::tmux::is_tmux_available() {
        eprintln!("Error: tmux not found in PATH");
        eprintln!();
        eprintln!("Agent of Empires requires tmux. Install with:");
        eprintln!("  brew install tmux     # macOS");
        eprintln!("  apt install tmux      # Debian/Ubuntu");
        eprintln!("  pacman -S tmux        # Arch");
        std::process::exit(1);
    }

    // Check for coding tools
    let available_tools = crate::tmux::AvailableTools::detect();
    if !available_tools.any_available() {
        eprintln!("Error: No coding tools found in PATH");
        eprintln!();
        eprintln!("Agent of Empires requires at least one of:");
        eprintln!("  claude    - Anthropic's Claude CLI");
        eprintln!("  opencode  - OpenCode CLI");
        eprintln!("  cursor    - Cursor's Agent CLI");
        eprintln!();
        eprintln!("Install one of these tools and ensure it's in your PATH.");
        std::process::exit(1);
    }

    // If version changed, refresh the update cache before showing TUI.
    // This ensures we have release notes for the changelog dialog.
    if check_version_change()?.is_some() {
        let settings = get_update_settings();
        if settings.check_enabled {
            let current_version = env!("CARGO_PKG_VERSION");
            // Don't let a network issue block startup
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                check_for_update(current_version, true),
            )
            .await;
        }
    }

    // If running inside tmux, temporarily enable mouse so crossterm receives
    // proper mouse events instead of tmux converting scroll to arrow keys.
    let saved_tmux_mouse = if std::env::var("TMUX").is_ok() {
        enable_tmux_mouse()
    } else {
        None
    };

    // Install panic hook that restores the terminal and the pre-launch title.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = tab_title::pop_terminal_title(&mut io::stdout());
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            crossterm::cursor::Show
        );
        original_hook(info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run
    let mut app = App::new(profile, available_tools, launch_dir)?;
    let _ = tab_title::push_terminal_title(&mut io::stdout());
    let _ = tab_title::set_tui_title(&mut io::stdout());
    let result = app.run(&mut terminal).await;

    // Clean up the nested-detach tmux hook if we set one up during this run.
    if std::env::var("TMUX").is_ok() {
        crate::tmux::utils::cleanup_nested_detach_binding();
    }

    // Restore tmux mouse setting
    if let Some(original) = saved_tmux_mouse {
        restore_tmux_mouse(&original);
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    let _ = tab_title::pop_terminal_title(&mut io::stdout());

    result
}

/// Enable tmux mouse mode for the current session, returning the previous value
/// so it can be restored on exit.
fn enable_tmux_mouse() -> Option<String> {
    use std::process::Command;

    // Query current mouse setting
    let original = Command::new("tmux")
        .args(["show-option", "-gv", "mouse"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // Enable mouse
    let _ = Command::new("tmux")
        .args(["set-option", "-g", "mouse", "on"])
        .output();

    Some(original)
}

/// Restore the original tmux mouse setting.
fn restore_tmux_mouse(original: &str) {
    use std::process::Command;

    if original == "on" {
        // Already was on, nothing to restore
        return;
    }

    let _ = Command::new("tmux")
        .args(["set-option", "-g", "mouse", original])
        .output();
}
