use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Args;

#[derive(Args)]
pub struct CodexProviderArgs {
    #[arg(long)]
    cwd: PathBuf,
}

pub fn run(args: CodexProviderArgs) -> Result<()> {
    if !args.cwd.is_dir() {
        bail!(
            "Codex working directory does not exist: {}",
            args.cwd.display()
        );
    }
    let Some(cwd) = args.cwd.to_str() else {
        bail!("Codex working directory is not valid UTF-8");
    };
    let provider = crate::codex_permissions::current_local_provider(cwd)?;
    println!("{}", serde_json::json!(provider));
    Ok(())
}
