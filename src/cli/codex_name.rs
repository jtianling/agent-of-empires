use anyhow::{ensure, Result};
use clap::Args;

#[derive(Args)]
pub struct CodexNameArgs {
    #[arg(long)]
    launch_id: String,
    #[arg(long)]
    endpoint: url::Url,
}

pub async fn run(args: CodexNameArgs) -> Result<()> {
    ensure!(
        std::env::var("TMUX").is_ok_and(|value| !value.is_empty()),
        "Codex naming requires the launcher's tmux connection"
    );
    crate::tmux::init_tmux_socket_name(None);
    crate::codex_name::run(&args.launch_id, args.endpoint.as_str()).await
}
