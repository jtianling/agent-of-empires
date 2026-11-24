use anyhow::{ensure, Context, Result};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

pub fn run(command: &[OsString]) -> Result<()> {
    let [path, defer, separator, args @ ..] = command else {
        anyhow::bail!("Expected xats-agy --defer-registration -- [agy arguments]");
    };
    let path = Path::new(path);
    let installed = crate::agents::executable::resolve("xats-agy")
        .context("Cannot find installed xats-agy launcher")?;
    ensure!(
        defer == "--defer-registration"
            && separator == "--"
            && path.is_absolute()
            && path.canonicalize()? == installed.canonicalize()?,
        "Unexpected deferred Antigravity launcher or arguments"
    );
    let key = crate::agents::antigravity::launcher_key()?;
    let error = Command::new(path)
        .args([defer, separator])
        .args(args)
        .env_remove("XATS_IDENTITY_KEY")
        .env("XATS_AGY_IDENTITY_KEY", key)
        .exec();
    Err(error).context("Starting deferred Antigravity PTY launcher")
}
