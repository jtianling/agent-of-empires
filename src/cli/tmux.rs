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
    #[arg(long, required_unless_present_any = ["index", "back"])]
    direction: Option<String>,
    #[arg(long, conflicts_with_all = ["direction", "back"])]
    index: Option<usize>,
    #[arg(long, conflicts_with_all = ["direction", "index"])]
    back: bool,
    #[arg(long, default_value = "default")]
    profile: String,
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

pub fn run_switch_session(args: SwitchSessionArgs) -> Result<()> {
    if args.back {
        crate::tmux::utils::switch_aoe_session_back(&args.profile, args.client_name.as_deref())
    } else if let Some(index) = args.index {
        crate::tmux::utils::switch_aoe_session_by_index(
            index,
            &args.profile,
            args.client_name.as_deref(),
        )
    } else {
        crate::tmux::utils::switch_aoe_session(
            args.direction.as_deref().unwrap_or("next"),
            &args.profile,
            args.client_name.as_deref(),
        )
    }
}
