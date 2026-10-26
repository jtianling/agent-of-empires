use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args)]
pub struct ManagedArgs {
    /// Persistent manager context file
    #[arg(long, global = true, env = "AOE_MANAGER_CONTEXT")]
    pub context: Option<PathBuf>,
    #[command(subcommand)]
    pub command: ManagedCommand,
}

#[derive(Subcommand)]
pub enum ManagedCommand {
    /// Create a new manager context at --context (never overwrites)
    Init,
    /// Create an owned session, optionally launch it
    Create(CreateArgs),
    /// List session metadata without terminal content
    List,
    /// Show exact session metadata and xats readiness
    Show(TargetArgs),
    /// Start an owned session
    Start(TargetArgs),
    /// Stop an owned session
    Stop(TargetArgs),
    /// Restart an owned session
    Restart(TargetArgs),
    /// Rename an owned session
    Rename(RenameArgs),
    /// Send literal input to an owned session
    Send(SendArgs),
    /// Read terminal output from an owned session
    Capture(CaptureArgs),
    /// Remove an owned session, preserving code and shared resources
    Remove(TargetArgs),
}

#[derive(Args)]
pub struct TargetArgs {
    /// Complete session ID (no titles or prefixes)
    pub id: String,
}

#[derive(Args)]
pub struct CreateArgs {
    /// Existing project directory
    pub path: PathBuf,
    /// Agent tool from the AoE registry
    #[arg(long)]
    pub tool: String,
    #[arg(long)]
    pub title: String,
    #[arg(long, default_value = "")]
    pub group: String,
    #[arg(long)]
    pub launch: bool,
    /// xats collaborator name (requires --xats-team)
    #[arg(long, requires = "xats_team")]
    pub xats_name: Option<String>,
    /// xats team (requires --xats-name)
    #[arg(long, requires = "xats_name")]
    pub xats_team: Option<String>,
}

#[derive(Args)]
pub struct RenameArgs {
    pub id: String,
    #[arg(long)]
    pub title: String,
}

#[derive(Args)]
pub struct SendArgs {
    pub id: String,
    pub message: String,
}

#[derive(Args)]
pub struct CaptureArgs {
    pub id: String,
    #[arg(short = 'n', long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=10000))]
    pub lines: u32,
}

#[derive(Args)]
pub struct SetOwnerArgs {
    /// Complete session ID
    pub id: String,
    /// Transfer to this registered manager context; omit to take human ownership
    #[arg(long)]
    pub context: Option<PathBuf>,
}
