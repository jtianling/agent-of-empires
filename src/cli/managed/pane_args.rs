use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum PaneCommand {
    /// List pane identities, ownership, positions and xats readiness
    List(SessionArgs),
    /// Create an owned agent pane in a running owned session
    Create(CreateArgs),
    /// Close an owned pane, promoting an eligible sibling when necessary
    Remove(TargetArgs),
    /// Send literal input to one exact owned pane
    Send(SendArgs),
    /// Capture output from one exact owned pane
    Capture(CaptureArgs),
}

#[derive(Args)]
pub struct SessionArgs {
    pub session_id: String,
}

#[derive(Args)]
pub struct TargetArgs {
    pub session_id: String,
    pub pane_id: String,
}

#[derive(Args)]
pub struct CreateArgs {
    pub session_id: String,
    #[arg(long)]
    pub tool: String,
    #[arg(long)]
    pub path: Option<PathBuf>,
    #[arg(long, requires = "xats_team")]
    pub xats_name: Option<String>,
    #[arg(long, requires = "xats_name")]
    pub xats_team: Option<String>,
}

#[derive(Args)]
pub struct SendArgs {
    pub session_id: String,
    pub pane_id: String,
    pub message: String,
}

#[derive(Args)]
pub struct CaptureArgs {
    pub session_id: String,
    pub pane_id: String,
    #[arg(short = 'n', long, default_value_t = 50,
        value_parser = clap::value_parser!(u32).range(1..=10000))]
    pub lines: u32,
}

impl PaneCommand {
    pub fn session_id(&self) -> &str {
        match self {
            Self::List(args) => &args.session_id,
            Self::Create(args) => &args.session_id,
            Self::Remove(args) => &args.session_id,
            Self::Send(args) => &args.session_id,
            Self::Capture(args) => &args.session_id,
        }
    }
}
