use anyhow::{ensure, Context, Result};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

pub fn run(command: &[OsString]) -> Result<()> {
    let (path, head, args): (&Path, &[&str], &[OsString]) = match command {
        [path, mode, defer, separator, args @ ..]
            if mode == "--agentapi" && defer == "--defer-registration" && separator == "--" =>
        {
            (
                Path::new(path),
                &["--agentapi", "--defer-registration", "--"],
                args,
            )
        }
        [path, defer, separator, args @ ..]
            if defer == "--defer-registration" && separator == "--" =>
        {
            (Path::new(path), &["--defer-registration", "--"], args)
        }
        _ => {
            anyhow::bail!("Expected xats-agy [--agentapi] --defer-registration -- [agy arguments]")
        }
    };
    let installed = crate::agents::executable::resolve("xats-agy")
        .context("Cannot find installed xats-agy launcher")?;
    ensure!(
        path.is_absolute() && path.canonicalize()? == installed.canonicalize()?,
        "Unexpected deferred Antigravity launcher or arguments"
    );
    let key = crate::agents::antigravity::launcher_key()?;
    let error = Command::new(path)
        .args(head)
        .args(args)
        .env_remove("XATS_IDENTITY_KEY")
        .env("XATS_AGY_IDENTITY_KEY", key)
        .exec();
    Err(error).context("Starting deferred Antigravity launcher")
}
