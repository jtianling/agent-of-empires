//! `aoe tmux` command implementation

use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum TmuxCommands {
    /// Output session info for use in custom tmux status bar
    ///
    /// Add this to your ~/.tmux.conf:
    ///   set -g status-right "#(aoe tmux status)"
    Status(TmuxStatusArgs),
    #[command(hide = true, name = "monitor-codex-title")]
    CodexTitleMonitor(CodexTitleMonitorArgs),
    #[command(hide = true, name = "switch-session")]
    SwitchSession(SwitchSessionArgs),
    #[command(hide = true, name = "refresh-bindings")]
    RefreshBindings(RefreshBindingsArgs),
}

#[derive(Args)]
pub struct TmuxStatusArgs {
    /// Output format (text or json)
    #[arg(short, long, default_value = "text")]
    format: String,
}

#[derive(Args)]
pub struct CodexTitleMonitorArgs {
    #[arg(long)]
    session: String,
}

#[derive(Args)]
pub struct SwitchSessionArgs {
    #[arg(long)]
    direction: String,
    #[arg(long, default_value = "default")]
    profile: String,
    #[arg(long)]
    client_name: Option<String>,
}

#[derive(Args)]
pub struct RefreshBindingsArgs {
    #[arg(long)]
    client_name: Option<String>,
}

pub fn run_status(args: TmuxStatusArgs) -> Result<()> {
    use crate::tmux::status_bar::get_session_info_for_current;

    match get_session_info_for_current() {
        Some(info) => {
            if args.format == "json" {
                let json = serde_json::json!({
                    "title": info.title,
                    "branch": info.branch,
                    "sandbox": info.sandbox,
                });
                println!("{}", serde_json::to_string(&json)?);
            } else {
                let mut output = format!("aoe: {}", info.title);
                if let Some(b) = &info.branch {
                    output.push_str(" | ");
                    output.push_str(b);
                }
                if let Some(s) = &info.sandbox {
                    output.push_str(" [");
                    output.push_str(s);
                    output.push(']');
                }
                print!("{}", output);
            }
        }
        None => {
            // Not in an aoe session - output nothing (cleaner for tmux status bar)
            if args.format == "json" {
                println!("null");
            }
        }
    }

    Ok(())
}

pub fn run_codex_title_monitor(args: CodexTitleMonitorArgs) -> Result<()> {
    crate::tmux::status_bar::run_codex_title_monitor(&args.session)
}

pub fn run_refresh_bindings(args: RefreshBindingsArgs) -> Result<()> {
    crate::tmux::utils::refresh_bindings(args.client_name.as_deref())
}

pub fn run_switch_session(args: SwitchSessionArgs) -> Result<()> {
    crate::tmux::utils::switch_aoe_session(
        &args.direction,
        &args.profile,
        args.client_name.as_deref(),
    )
}
