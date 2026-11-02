use anyhow::{Context, Result};
use clap::Args;

use crate::session::Storage;

#[derive(Args)]
pub struct CodexRecoveryArgs {
    /// Complete session ID
    identifier: String,
    #[arg(long, value_parser = clap::value_parser!(i64).range(0..=crate::db::MAX_SLOT))]
    slot: i64,
    /// Complete original Codex thread UUID, selected explicitly
    #[arg(long)]
    thread: String,
}

pub(super) fn run(profile: &str, args: CodexRecoveryArgs) -> Result<()> {
    crate::session::managed_pane::validate_pane_id(&args.identifier)?;
    let storage = Storage::new(profile)?;
    let _operation = storage.operation_lock()?;
    let (instances, _) = storage.load_with_groups()?;
    let instance = instances
        .iter()
        .find(|instance| instance.id == args.identifier)
        .context("Session not found; use its complete ID")?;
    instance.recover_codex_thread(profile, args.slot, &args.thread)?;
    println!(
        "Recovered Codex binding: session {}, slot {}, thread {}. Pane was not restarted.",
        args.identifier, args.slot, args.thread
    );
    Ok(())
}
