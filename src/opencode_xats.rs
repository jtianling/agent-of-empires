//! Paired xats control-plane client for OpenCode runtime recovery.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

pub const IDENTITY_KEY_ENV: &str = "XATS_IDENTITY_KEY";
const CLI_BINARY: &str = "cross-agent-teams-mcp";
const PROTOCOL_VERSION: u32 = 1;
const COMMIT_ATTEMPTS: usize = 3;
const COMMIT_RETRY_DELAY: Duration = Duration::from_millis(200);
const CLI_TIMEOUT: Duration = Duration::from_secs(5);
const CLI_CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);
const CLI_POLL_DELAY: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveStatus {
    Reserved,
    AlreadyReserved,
    NeedRegister,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStatus {
    Committed,
    AlreadyCommitted,
    NeedRegister,
}

#[derive(Debug, Deserialize)]
struct CliResponse {
    protocol_version: u32,
    status: String,
    #[serde(default)]
    detail: Option<String>,
}

pub fn reserve(identity_key: &str, generation: i64) -> Result<ReserveStatus> {
    validate_identity_key(identity_key)?;
    validate_generation(generation)?;
    let args = reserve_args(generation);
    let response = invoke(identity_key, &args).context("reserving OpenCode xats runtime")?;
    match response.status.as_str() {
        "reserved" => Ok(ReserveStatus::Reserved),
        "already_reserved" => Ok(ReserveStatus::AlreadyReserved),
        "need_register" => Ok(ReserveStatus::NeedRegister),
        status => bail!(
            "xats reserve returned fail-closed status '{}': {}",
            status,
            response.detail.as_deref().unwrap_or("no detail")
        ),
    }
}

pub fn commit(
    identity_key: &str,
    generation: i64,
    base_url: &str,
    session_id: &str,
) -> Result<CommitStatus> {
    validate_identity_key(identity_key)?;
    validate_generation(generation)?;
    validate_base_url(base_url)?;
    crate::opencode_runtime::validate_session_id(session_id)?;
    let args = commit_args(generation, base_url, session_id);
    let mut last_error = None;
    for attempt in 1..=COMMIT_ATTEMPTS {
        match invoke(identity_key, &args) {
            Ok(response) => match response.status.as_str() {
                "committed" => return Ok(CommitStatus::Committed),
                "already_committed" => return Ok(CommitStatus::AlreadyCommitted),
                "need_register" => return Ok(CommitStatus::NeedRegister),
                "partial" | "retry" => {
                    last_error = Some(anyhow::anyhow!(
                        "xats commit is partial: {}",
                        response.detail.as_deref().unwrap_or("no detail")
                    ));
                }
                status => bail!(
                    "xats commit returned fail-closed status '{}': {}",
                    status,
                    response.detail.as_deref().unwrap_or("no detail")
                ),
            },
            Err(error) if is_protocol_error(&error) => return Err(error),
            Err(error) => last_error = Some(error),
        }
        if attempt < COMMIT_ATTEMPTS {
            std::thread::sleep(COMMIT_RETRY_DELAY);
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("xats commit failed")))
        .context("committing OpenCode xats runtime after bounded retry")
}

fn invoke(identity_key: &str, args: &[String]) -> Result<CliResponse> {
    invoke_with_binary(Path::new(CLI_BINARY), identity_key, args)
}

fn invoke_with_binary(binary: &Path, identity_key: &str, args: &[String]) -> Result<CliResponse> {
    invoke_with_binary_timeout(binary, identity_key, args, CLI_TIMEOUT)
}

fn invoke_with_binary_timeout(
    binary: &Path,
    identity_key: &str,
    args: &[String],
    timeout: Duration,
) -> Result<CliResponse> {
    let mut command = Command::new(binary);
    crate::process::configure_owned_process_group(&mut command);
    let child = command
        .args(args)
        .env(IDENTITY_KEY_ENV, identity_key)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "launching paired xats CLI '{}': not available",
                binary.display()
            )
        })?;
    let output = wait_with_output_timeout(child, timeout)?;
    parse_output(output).map_err(|error| {
        let diagnostic = format!("{error:#}").replace(identity_key, "***");
        anyhow::anyhow!(diagnostic)
    })
}

fn wait_with_output_timeout(mut child: Child, timeout: Duration) -> Result<Output> {
    let stdout = child
        .stdout
        .take()
        .context("capturing paired xats CLI stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("capturing paired xats CLI stderr")?;
    let (stdout_rx, stdout_reader) = spawn_output_reader(stdout, "xats-cli-stdout");
    let (stderr_rx, stderr_reader) = spawn_output_reader(stderr, "xats-cli-stderr");
    let mut state = ChildOutputState::default();
    if let Err(error) = stdout_reader.and(stderr_reader) {
        let error = anyhow::Error::new(error).context("spawning paired xats CLI output reader");
        let cleanup = cleanup_owned_child(&mut child, &mut state, &stdout_rx, &stderr_rx);
        return append_cleanup_error(error, cleanup);
    }
    let started = Instant::now();
    loop {
        if let Err(error) = state.poll(&mut child, &stdout_rx, &stderr_rx) {
            let cleanup = cleanup_owned_child(&mut child, &mut state, &stdout_rx, &stderr_rx);
            return append_cleanup_error(error, cleanup);
        }
        if state.is_complete() {
            return state.into_output();
        }
        if started.elapsed() >= timeout {
            let error =
                anyhow::anyhow!("paired xats CLI timed out after {} ms", timeout.as_millis());
            let cleanup = cleanup_owned_child(&mut child, &mut state, &stdout_rx, &stderr_rx);
            return append_cleanup_error(error, cleanup);
        }
        std::thread::sleep(CLI_POLL_DELAY);
    }
}

type OutputReceiver = Receiver<io::Result<Vec<u8>>>;

#[derive(Default)]
struct ChildOutputState {
    status: Option<ExitStatus>,
    stdout: Option<io::Result<Vec<u8>>>,
    stderr: Option<io::Result<Vec<u8>>>,
}

impl ChildOutputState {
    fn poll(
        &mut self,
        child: &mut Child,
        stdout_rx: &OutputReceiver,
        stderr_rx: &OutputReceiver,
    ) -> Result<()> {
        if self.status.is_none() {
            self.status = child.try_wait().context("checking paired xats CLI")?;
        }
        poll_output_reader(stdout_rx, &mut self.stdout, "stdout");
        poll_output_reader(stderr_rx, &mut self.stderr, "stderr");
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.status.is_some() && self.stdout.is_some() && self.stderr.is_some()
    }

    fn into_output(self) -> Result<Output> {
        Ok(Output {
            status: self.status.context("paired xats CLI status is missing")?,
            stdout: self
                .stdout
                .context("paired xats CLI stdout is missing")?
                .context("reading paired xats CLI stdout")?,
            stderr: self
                .stderr
                .context("paired xats CLI stderr is missing")?
                .context("reading paired xats CLI stderr")?,
        })
    }
}

fn spawn_output_reader(
    mut reader: impl Read + Send + 'static,
    name: &str,
) -> (OutputReceiver, io::Result<()>) {
    let (sender, receiver) = mpsc::sync_channel(1);
    let spawn = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let mut output = Vec::new();
            let result = reader.read_to_end(&mut output).map(|_| output);
            if sender.send(result).is_err() {
                tracing::debug!("paired xats CLI output receiver was already dropped");
            }
        })
        .map(|_| ());
    (receiver, spawn)
}

fn poll_output_reader(
    receiver: &OutputReceiver,
    output: &mut Option<io::Result<Vec<u8>>>,
    stream: &str,
) {
    if output.is_some() {
        return;
    }
    match receiver.try_recv() {
        Ok(result) => *output = Some(result),
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            *output = Some(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("paired xats CLI {stream} reader disconnected"),
            )));
        }
    }
}

fn cleanup_owned_child(
    child: &mut Child,
    state: &mut ChildOutputState,
    stdout_rx: &OutputReceiver,
    stderr_rx: &OutputReceiver,
) -> Result<()> {
    let group_kill = crate::process::kill_owned_process_group(child.id());
    let direct_kill = child.kill();
    let started = Instant::now();
    let mut poll_error = None;
    while !state.is_complete() && started.elapsed() < CLI_CLEANUP_TIMEOUT {
        if let Err(error) = state.poll(child, stdout_rx, stderr_rx) {
            poll_error = Some(error);
            break;
        }
        std::thread::sleep(CLI_POLL_DELAY);
    }
    if state.is_complete() {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "paired xats CLI cleanup did not complete \
         (group kill: {}; direct kill: {}; poll: {})",
        format_result(group_kill),
        format_result(direct_kill),
        poll_error.map_or_else(|| "ok".to_string(), |error| format!("{error:#}")),
    ))
}

fn format_result(result: io::Result<()>) -> String {
    result.map_or_else(|error| error.to_string(), |()| "ok".to_string())
}

fn append_cleanup_error(error: anyhow::Error, cleanup: Result<()>) -> Result<Output> {
    match cleanup {
        Ok(()) => Err(error),
        Err(cleanup_error) => Err(anyhow::anyhow!(
            "{error:#}. Cleanup also failed: {cleanup_error:#}"
        )),
    }
}

fn parse_output(output: Output) -> Result<CliResponse> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let diagnostic = stderr.trim();
        if diagnostic.to_ascii_lowercase().contains("protocol") {
            bail!("xats protocol mismatch: {}", diagnostic);
        }
        bail!(
            "xats CLI exited with {}: {}",
            output.status,
            if diagnostic.is_empty() {
                "no diagnostic"
            } else {
                diagnostic
            }
        );
    }
    let response: CliResponse = serde_json::from_slice(&output.stdout)
        .context("xats protocol mismatch: invalid JSON response")?;
    if response.protocol_version != PROTOCOL_VERSION {
        bail!(
            "xats protocol mismatch: expected {}, got {}",
            PROTOCOL_VERSION,
            response.protocol_version
        );
    }
    Ok(response)
}

fn reserve_args(generation: i64) -> Vec<String> {
    vec![
        "reserve-opencode-runtime".to_string(),
        "--identity-key-env".to_string(),
        IDENTITY_KEY_ENV.to_string(),
        "--runtime-generation".to_string(),
        generation.to_string(),
    ]
}

fn commit_args(generation: i64, base_url: &str, session_id: &str) -> Vec<String> {
    vec![
        "commit-opencode-runtime".to_string(),
        "--identity-key-env".to_string(),
        IDENTITY_KEY_ENV.to_string(),
        "--runtime-generation".to_string(),
        generation.to_string(),
        "--base-url".to_string(),
        base_url.to_string(),
        "--session-id".to_string(),
        session_id.to_string(),
    ]
}

fn validate_identity_key(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_whitespace) {
        bail!("invalid xats identity key")
    }
    Ok(())
}

fn validate_generation(value: i64) -> Result<()> {
    if !(1..=crate::db::MAX_XATS_RUNTIME_GENERATION).contains(&value) {
        bail!("xats runtime generation must be a positive safe integer")
    }
    Ok(())
}

fn validate_base_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).context("invalid OpenCode base URL")?;
    let loopback = url
        .host_str()
        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"));
    if url.scheme() != "http" || !loopback || url.port().is_none() {
        bail!("OpenCode base URL must be an explicit loopback HTTP endpoint")
    }
    Ok(())
}

fn is_protocol_error(error: &anyhow::Error) -> bool {
    format!("{error:#}").contains("protocol mismatch")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ExitStatus;

    #[cfg(unix)]
    fn status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[test]
    fn reserve_argv_never_contains_identity_key() {
        let args = reserve_args(7);
        assert!(!args.iter().any(|arg| arg.contains("secret-key")));
        assert_eq!(
            args,
            [
                "reserve-opencode-runtime",
                "--identity-key-env",
                IDENTITY_KEY_ENV,
                "--runtime-generation",
                "7",
            ]
        );
    }

    #[test]
    fn commit_argv_carries_exact_runtime_tuple_without_key() {
        let args = commit_args(9, "http://127.0.0.1:8123", "ses_left");
        assert!(!args.iter().any(|arg| arg == "secret-key"));
        assert_eq!(
            args,
            [
                "commit-opencode-runtime",
                "--identity-key-env",
                IDENTITY_KEY_ENV,
                "--runtime-generation",
                "9",
                "--base-url",
                "http://127.0.0.1:8123",
                "--session-id",
                "ses_left",
            ]
        );
    }

    #[test]
    #[cfg(unix)]
    fn parser_accepts_reserved_and_rejects_protocol_drift() {
        let parsed = parse_output(Output {
            status: status(0),
            stdout: br#"{"protocol_version":1,"status":"reserved"}"#.to_vec(),
            stderr: Vec::new(),
        })
        .unwrap();
        assert_eq!(parsed.status, "reserved");

        let error = parse_output(Output {
            status: status(0),
            stdout: br#"{"protocol_version":2,"status":"reserved"}"#.to_vec(),
            stderr: Vec::new(),
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("protocol mismatch"));
    }

    #[test]
    fn endpoint_validation_is_loopback_only() {
        assert!(validate_base_url("http://127.0.0.1:8123").is_ok());
        assert!(validate_base_url("https://127.0.0.1:8123").is_err());
        assert!(validate_base_url("http://example.com:8123").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn paired_cli_receives_identity_only_through_environment() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("fake-xats");
        std::fs::write(
            &binary,
            r#"#!/bin/sh
case " $* " in
  *secret-key*) exit 9 ;;
esac
test "$XATS_IDENTITY_KEY" = "secret-key" || exit 8
printf '{"protocol_version":1,"status":"reserved"}'
"#,
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary, permissions).unwrap();

        let response = invoke_with_binary(&binary, "secret-key", &reserve_args(7)).unwrap();
        assert_eq!(response.status, "reserved");
    }

    #[test]
    #[cfg(unix)]
    fn paired_cli_timeout_terminates_and_reaps_the_process() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("slow-xats");
        std::fs::write(&binary, "#!/bin/sh\nsleep 30\n").unwrap();
        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary, permissions).unwrap();

        let error = invoke_with_binary_timeout(
            &binary,
            "secret-key",
            &reserve_args(7),
            Duration::from_millis(20),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("timed out after 20 ms"));
    }

    #[test]
    #[cfg(unix)]
    fn paired_cli_timeout_covers_output_held_by_a_background_child() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("background-xats");
        std::fs::write(
            &binary,
            concat!(
                "#!/bin/sh\n",
                "sleep 30 &\n",
                "printf '{\"protocol_version\":1,\"status\":\"reserved\"}'\n",
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary, permissions).unwrap();

        let started = Instant::now();
        let error = invoke_with_binary_timeout(
            &binary,
            "secret-key",
            &reserve_args(7),
            Duration::from_millis(20),
        )
        .unwrap_err();
        let diagnostic = format!("{error:#}");

        assert!(diagnostic.contains("timed out after 20 ms"));
        assert!(!diagnostic.contains("Cleanup also failed"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
