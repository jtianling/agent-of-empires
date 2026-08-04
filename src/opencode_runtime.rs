//! AoE-owned OpenCode server and exact-session attach runtime.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Args;
use serde::Deserialize;
use tokio::process::{Child, Command};

const HOST: &str = "127.0.0.1";
const SERVER_ATTEMPTS: usize = 3;
const HEALTH_ATTEMPTS: usize = 100;
const SLOT_ATTEMPTS: usize = 50;
const POLL_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Args)]
pub struct OpenCodeRuntimeArgs {
    #[arg(long)]
    pub instance_id: String,

    #[arg(long)]
    pub slot: i64,

    #[arg(long)]
    pub generation: i64,

    #[arg(long)]
    pub working_directory: PathBuf,

    #[arg(long)]
    pub resume_session: Option<String>,

    #[arg(long)]
    pub cross_agent_team: bool,

    #[arg(last = true, trailing_var_arg = true)]
    pub extra_args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SessionResponse {
    id: String,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    healthy: bool,
    version: String,
}

pub async fn run(profile: &str, args: OpenCodeRuntimeArgs) -> Result<()> {
    let args = validate_args(profile, args)?;
    let (mut server, base_url) = start_server(&args.working_directory).await?;
    let result = run_with_server(profile, &args, &base_url).await;
    let cleanup = terminate_owned_server(&mut server).await;
    merge_runtime_and_cleanup(result, cleanup)
}

async fn run_with_server(profile: &str, args: &OpenCodeRuntimeArgs, base_url: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("building OpenCode HTTP client")?;
    let session_id = match args.resume_session.as_deref() {
        Some(session_id) => {
            verify_session(&client, base_url, &args.working_directory, session_id).await?;
            session_id.to_string()
        }
        None => create_session(&client, base_url, &args.working_directory).await?,
    };

    record_exact_session(profile, args, &session_id).await?;
    if args.cross_agent_team {
        let identity_key = std::env::var(crate::opencode_xats::IDENTITY_KEY_ENV)
            .context("Cross Agent Team OpenCode runtime is missing XATS_IDENTITY_KEY")?;
        crate::opencode_xats::commit(&identity_key, args.generation, base_url, &session_id)?;
    }

    let status = Command::new("opencode")
        .args(attach_args(base_url, &session_id, &args.extra_args))
        .current_dir(&args.working_directory)
        .env_remove("OPENCODE_SERVER_PASSWORD")
        .env_remove("OPENCODE_SERVER_USERNAME")
        .status()
        .await
        .context("starting OpenCode attach")?;
    if !status.success() {
        bail!("OpenCode attach exited with {}", status);
    }
    Ok(())
}

fn validate_args(profile: &str, args: OpenCodeRuntimeArgs) -> Result<OpenCodeRuntimeArgs> {
    if profile.trim().is_empty() || matches!(profile, "." | "..") || profile.contains(['/', '\\']) {
        bail!("invalid OpenCode runtime profile");
    }
    if args.instance_id.is_empty()
        || args.instance_id.len() > 128
        || !args
            .instance_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        bail!("invalid OpenCode runtime instance id");
    }
    if !(0..=crate::db::MAX_SLOT).contains(&args.slot) {
        bail!("OpenCode runtime slot is out of range");
    }
    if !(0..=crate::db::MAX_XATS_RUNTIME_GENERATION).contains(&args.generation) {
        bail!("OpenCode runtime generation is not a safe integer");
    }
    if args.cross_agent_team && args.generation == 0 {
        bail!("Cross Agent Team OpenCode runtime requires a positive generation");
    }
    if !args.working_directory.is_dir() {
        bail!(
            "OpenCode runtime working directory is not a directory: {}",
            args.working_directory.display()
        );
    }
    if let Some(session_id) = args.resume_session.as_deref() {
        validate_session_id(session_id)?;
    }
    validate_extra_args(&args.extra_args)?;
    Ok(args)
}

pub fn parse_and_validate_extra_args(value: &str) -> Result<Vec<String>> {
    let args = shell_words::split(value).context("invalid OpenCode extra args quoting")?;
    validate_extra_args(&args)?;
    Ok(args)
}

fn validate_extra_args(args: &[String]) -> Result<()> {
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--print-logs" | "--pure" | "--mini" | "--no-replay" => index += 1,
            "--log-level" => {
                validate_log_level(
                    args.get(index + 1)
                        .context("--log-level requires a value")?,
                )?;
                index += 2;
            }
            "--replay-limit" => {
                validate_replay_limit(
                    args.get(index + 1)
                        .context("--replay-limit requires a value")?,
                )?;
                index += 2;
            }
            _ => {
                if let Some(value) = argument.strip_prefix("--log-level=") {
                    validate_log_level(value)?;
                } else if let Some(value) = argument.strip_prefix("--replay-limit=") {
                    validate_replay_limit(value)?;
                } else {
                    bail!(
                        "OpenCode attach does not support extra argument \
                         '{argument}'"
                    );
                }
                index += 1;
            }
        }
    }
    Ok(())
}

fn validate_log_level(value: &str) -> Result<()> {
    if !matches!(value, "DEBUG" | "INFO" | "WARN" | "ERROR") {
        bail!("invalid OpenCode attach log level '{value}'");
    }
    Ok(())
}

fn validate_replay_limit(value: &str) -> Result<()> {
    value
        .parse::<u64>()
        .with_context(|| format!("invalid OpenCode attach replay limit '{value}'"))?;
    Ok(())
}

fn attach_args(base_url: &str, session_id: &str, extra_args: &[String]) -> Vec<String> {
    let mut args = vec![
        "attach".to_string(),
        base_url.to_string(),
        "--session".to_string(),
        session_id.to_string(),
    ];
    args.extend_from_slice(extra_args);
    args
}

pub fn validate_session_id(value: &str) -> Result<()> {
    let valid = value.strip_prefix("ses_").is_some_and(|rest| {
        !rest.is_empty()
            && value.len() <= 256
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    });
    if !valid {
        bail!("invalid OpenCode session id")
    }
    Ok(())
}

async fn start_server(working_directory: &Path) -> Result<(Child, String)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .context("building OpenCode health client")?;
    let mut last_error = None;
    for _ in 0..SERVER_ATTEMPTS {
        let port = allocate_loopback_port()?;
        let base_url = format!("http://{HOST}:{port}");
        let mut child = Command::new("opencode")
            .arg("serve")
            .arg("--hostname")
            .arg(HOST)
            .arg("--port")
            .arg(port.to_string())
            .current_dir(working_directory)
            .env_remove("OPENCODE_SERVER_PASSWORD")
            .env_remove("OPENCODE_SERVER_USERNAME")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting OpenCode loopback server")?;
        match wait_for_health(&client, &mut child, &base_url).await {
            Ok(()) => return Ok((child, base_url)),
            Err(error) => {
                last_error = Some(error);
                if let Err(cleanup_error) = terminate_owned_server(&mut child).await {
                    let startup_error = last_error.take().unwrap();
                    return Err(anyhow::anyhow!(
                        "{startup_error:#}. Failed to clean up failed OpenCode server startup: {cleanup_error:#}"
                    ));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("OpenCode server failed to bind")))
        .context("starting OpenCode loopback server after bounded port retry")
}

fn allocate_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind((HOST, 0)).context("allocating OpenCode loopback port")?;
    Ok(listener.local_addr()?.port())
}

async fn wait_for_health(
    client: &reqwest::Client,
    child: &mut Child,
    base_url: &str,
) -> Result<()> {
    let url = format!("{base_url}/global/health");
    for _ in 0..HEALTH_ATTEMPTS {
        if let Some(status) = child.try_wait().context("checking OpenCode server child")? {
            bail!("OpenCode server exited before health check with {status}");
        }
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                if let Ok(health) = response.json::<HealthResponse>().await {
                    if health.healthy && !health.version.trim().is_empty() {
                        return Ok(());
                    }
                }
            }
        }
        tokio::time::sleep(POLL_DELAY).await;
    }
    bail!("OpenCode server health check timed out")
}

async fn create_session(
    client: &reqwest::Client,
    base_url: &str,
    working_directory: &Path,
) -> Result<String> {
    let mut url = reqwest::Url::parse(&format!("{base_url}/session"))?;
    url.query_pairs_mut()
        .append_pair("directory", &working_directory.to_string_lossy());
    let response = client
        .post(url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .context("creating exact OpenCode session")?
        .error_for_status()
        .context("OpenCode session create returned an error")?
        .json::<SessionResponse>()
        .await
        .context("invalid OpenCode session create response")?;
    validate_session_id(&response.id)?;
    Ok(response.id)
}

async fn verify_session(
    client: &reqwest::Client,
    base_url: &str,
    working_directory: &Path,
    session_id: &str,
) -> Result<()> {
    let mut url = reqwest::Url::parse(base_url)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid OpenCode base URL"))?
        .extend(["session", session_id]);
    url.query_pairs_mut()
        .append_pair("directory", &working_directory.to_string_lossy());
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("loading exact OpenCode session '{session_id}'"))?
        .error_for_status()
        .with_context(|| format!("OpenCode session '{session_id}' is unavailable"))?
        .json::<SessionResponse>()
        .await
        .context("invalid OpenCode session response")?;
    if response.id != session_id {
        bail!(
            "OpenCode returned session '{}' while '{}' was requested",
            response.id,
            session_id
        );
    }
    Ok(())
}

async fn record_exact_session(
    profile: &str,
    args: &OpenCodeRuntimeArgs,
    session_id: &str,
) -> Result<()> {
    let pane = std::env::var("TMUX_PANE").context("OpenCode runtime requires TMUX_PANE")?;
    if crate::cli::record_pane::pane_hosts_this_process(&pane) == Some(false) {
        bail!("OpenCode runtime pane ancestry does not match {pane}");
    }
    let cwd = args.working_directory.to_string_lossy();
    let store = crate::db::Store::open_with_schema(profile)?;
    for _ in 0..SLOT_ATTEMPTS {
        if store.record_opencode_runtime_session(
            &args.instance_id,
            args.slot,
            args.generation,
            &pane,
            session_id,
            &cwd,
            crate::db::now_unix(),
        )? {
            return Ok(());
        }
        tokio::time::sleep(POLL_DELAY).await;
    }
    bail!(
        "OpenCode runtime slot {} for instance '{}' did not materialize at generation {}",
        args.slot,
        args.instance_id,
        args.generation
    )
}

async fn terminate_owned_server(child: &mut Child) -> Result<()> {
    if child
        .try_wait()
        .context("checking owned OpenCode server before cleanup")?
        .is_none()
    {
        child
            .kill()
            .await
            .context("terminating owned OpenCode server")?;
    }
    Ok(())
}

fn merge_runtime_and_cleanup(runtime: Result<()>, cleanup: Result<()>) -> Result<()> {
    match (runtime, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{error:#}. Failed to clean up owned OpenCode server: {cleanup_error:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn serve_once(body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener as TokioTcpListener;

        let listener = TokioTcpListener::bind((HOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let count = socket.read(&mut request).await.unwrap();
            request.truncate(count);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn exact_session_schema_accepts_only_opencode_ids() {
        assert!(validate_session_id("ses_left-123").is_ok());
        assert!(validate_session_id("left-123").is_err());
        assert!(validate_session_id("ses_a;rm").is_err());
    }

    #[test]
    fn only_attach_safe_options_are_accepted() {
        for value in [
            "--hostname 0.0.0.0",
            "--port=4096",
            "--session ses_other",
            "-sses_other",
            "--continue",
            "--fork",
            "--dir /tmp/other",
            "--password secret",
            "--username other",
            "--model anthropic/test",
            "--agent build",
            "--prompt hello",
        ] {
            assert!(
                parse_and_validate_extra_args(value).is_err(),
                "{value} should be rejected"
            );
        }
        assert_eq!(
            parse_and_validate_extra_args("--mini --log-level DEBUG --replay-limit=50",).unwrap(),
            ["--mini", "--log-level", "DEBUG", "--replay-limit=50"]
        );
    }

    #[test]
    fn attach_argv_keeps_runtime_tuple_before_validated_options() {
        let extra = parse_and_validate_extra_args("--mini --log-level DEBUG").unwrap();
        assert_eq!(
            attach_args("http://127.0.0.1:8123", "ses_exact", &extra),
            [
                "attach",
                "http://127.0.0.1:8123",
                "--session",
                "ses_exact",
                "--mini",
                "--log-level",
                "DEBUG",
            ]
        );
    }

    #[test]
    fn runtime_input_validation_rejects_missing_resume_and_bad_generation() {
        let args = OpenCodeRuntimeArgs {
            instance_id: "instance-1".to_string(),
            slot: 0,
            generation: 0,
            working_directory: std::env::temp_dir(),
            resume_session: Some("not-a-session".to_string()),
            cross_agent_team: false,
            extra_args: Vec::new(),
        };
        assert!(validate_args("default", args).is_err());
    }

    #[test]
    fn runtime_input_validation_rejects_profile_path_traversal() {
        for profile in ["", ".", "..", "parent/child", "parent\\child"] {
            let args = OpenCodeRuntimeArgs {
                instance_id: "instance-1".to_string(),
                slot: 0,
                generation: 0,
                working_directory: std::env::temp_dir(),
                resume_session: None,
                cross_agent_team: false,
                extra_args: Vec::new(),
            };
            assert!(validate_args(profile, args).is_err());
        }
    }

    #[test]
    fn cleanup_failure_is_not_hidden_by_runtime_failure() {
        let error = merge_runtime_and_cleanup(
            Err(anyhow::anyhow!("runtime failed")),
            Err(anyhow::anyhow!("cleanup failed")),
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("runtime failed"));
        assert!(message.contains("cleanup failed"));
    }

    #[tokio::test]
    async fn session_api_uses_exact_create_and_resume_endpoints() {
        let directory = std::env::temp_dir();
        let client = reqwest::Client::new();

        let (create_url, create_request) = serve_once(r#"{"id":"ses_created"}"#).await;
        let created = create_session(&client, &create_url, &directory)
            .await
            .unwrap();
        assert_eq!(created, "ses_created");
        let create_request = create_request.await.unwrap();
        assert!(create_request.starts_with("POST /session?directory="));

        let (resume_url, resume_request) = serve_once(r#"{"id":"ses_resume"}"#).await;
        verify_session(&client, &resume_url, &directory, "ses_resume")
            .await
            .unwrap();
        let resume_request = resume_request.await.unwrap();
        assert!(resume_request.starts_with("GET /session/ses_resume?directory="));
    }

    #[tokio::test]
    async fn exact_resume_rejects_a_different_session_response() {
        let (base_url, request) = serve_once(r#"{"id":"ses_other"}"#).await;
        let error = verify_session(
            &reqwest::Client::new(),
            &base_url,
            &std::env::temp_dir(),
            "ses_requested",
        )
        .await
        .unwrap_err();
        request.await.unwrap();
        assert!(format!("{error:#}").contains("ses_other"));
    }

    #[tokio::test]
    async fn owned_server_cleanup_waits_for_child_exit() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        terminate_owned_server(&mut child).await.unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }
}
