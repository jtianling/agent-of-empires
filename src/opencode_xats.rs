//! Paired xats control-plane client for OpenCode runtime recovery.

use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

pub const IDENTITY_KEY_ENV: &str = "XATS_IDENTITY_KEY";
const CLI_BINARY: &str = "cross-agent-teams-mcp";
const PROTOCOL_VERSION: u32 = 1;
const COMMIT_ATTEMPTS: usize = 3;
const COMMIT_RETRY_DELAY: Duration = Duration::from_millis(200);

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
    let output = Command::new(binary)
        .args(args)
        .env(IDENTITY_KEY_ENV, identity_key)
        .output()
        .with_context(|| {
            format!(
                "launching paired xats CLI '{}': not available",
                binary.display()
            )
        })?;
    parse_output(output).map_err(|error| {
        let diagnostic = format!("{error:#}").replace(identity_key, "***");
        anyhow::anyhow!(diagnostic)
    })
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
}
