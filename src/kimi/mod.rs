//! Kimi panes on a shared, user-owned kimi server.
//!
//! Unlike the OpenCode runtime, AoE owns no server here: it discovers the one
//! the user is already running, mints an exact session on it before the pane
//! starts, and connects. Nothing in this module starts or stops that server.

pub mod mcp_config;
pub mod server;
pub mod session;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub use server::{kimi_home, KimiServer};
pub use session::validate_session_id;

/// Names the kimi TUI and every server-side agent read to find their server.
pub const BASE_URL_ENV: &str = "KIMI_XATS_BASE_URL";
/// This pane's session. Its leakage is the worst of the three: reaching the
/// shared server's environment would make every server-side agent register to
/// one wrong session.
pub const SESSION_ID_ENV: &str = "KIMI_XATS_SESSION_ID";
/// Attaches the TUI to the shared server's engine instead of starting its own.
pub const REMOTE_MODE_ENV: &str = "KIMI_REMOTE";
pub const REMOTE_MODE_VALUE: &str = "auto";
/// The kimi command AoE launches in a Cross Agent Team pane, for panes with no
/// command override of their own.
///
/// Required rather than defaulted to `kimi` on the search path: the remote
/// engine mode lives in the CLI, so a server that mounts the IPC socket paired
/// with a CLI that cannot use it still runs an engine of its own. AoE knows
/// which binary it is about to launch, so it makes the user name it. An extra
/// pane has no per-instance command of its own, so this is the only way to name
/// its binary.
pub const COMMAND_ENV: &str = "AOE_KIMI_COMMAND";

/// What a prepared kimi pane needs in order to launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiLaunch {
    pub base_url: String,
    pub session_id: String,
}

/// Whether this launch resumes the slot's conversation or mints a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    Fresh,
    Resume,
}

/// Everything preparation needs to know about one pane.
#[derive(Debug, Clone)]
pub struct PaneRequest {
    pub working_directory: PathBuf,
    pub cross_agent_team: bool,
    pub mode: SessionMode,
    /// The slot's durable session, required when `mode` is [`SessionMode::Resume`].
    pub durable_session_id: String,
    /// The instance's command override, when this pane launches under one. It
    /// names the binary the same way [`COMMAND_ENV`] does, so the capability
    /// gate has to see it here or it would refuse a pane that is in fact
    /// explicitly configured.
    pub command_override: Option<String>,
}

/// Discover the shared server and produce this pane's exact session.
///
/// Every layer fails closed. There is no fallback to a session picked by
/// directory or recency, and no degraded Cross Agent Team launch: a pane whose
/// server cannot host its session would take poked turns in a conversation the
/// user never sees.
pub fn prepare_session(request: &PaneRequest) -> Result<KimiLaunch> {
    let home = kimi_home()?;
    let server = server::discover_in(&home)?;
    if request.cross_agent_team {
        server.require_capable_engine()?;
        mcp_config::validate(&home)?;
        require_configured_command(request.command_override.as_deref())?;
    }
    let token = server.read_token()?;
    let base_url = server.base_url.clone();
    let working_directory = request.working_directory.clone();
    let mode = request.mode;
    let durable = request.durable_session_id.clone();
    let model = default_model(&home);
    let session_id = crate::xats_control::block_on_control("aoe-kimi-session", {
        let base_url = base_url.clone();
        move || {
            Box::pin(async move {
                let client = session::build_client()?;
                match mode {
                    SessionMode::Resume => {
                        session::verify(&client, &base_url, &token, &working_directory, &durable)
                            .await?;
                        Ok(durable)
                    }
                    SessionMode::Fresh => {
                        session::mint(
                            &client,
                            &base_url,
                            &token,
                            &working_directory,
                            model.as_deref(),
                        )
                        .await
                    }
                }
            })
        }
    })?;
    Ok(KimiLaunch {
        base_url,
        session_id,
    })
}

/// Refresh this pane's xats delivery coordinates. Must be the last xats action
/// before the pane process starts.
///
/// `previous_session_id` is the durable session this slot held before the
/// launch, empty on a slot that never ran. It lets the daemon adopt the
/// identity key onto a row the agent registered without one.
pub fn commit_delivery(
    identity_key: &str,
    previous_session_id: &str,
    launch: &KimiLaunch,
) -> Result<()> {
    let previous = Some(previous_session_id).filter(|previous| !previous.is_empty());
    crate::kimi_xats::commit(identity_key, &launch.base_url, previous, &launch.session_id)
        .map(|_| ())
}

/// The kimi launch command words for a pane, before the session flag.
///
/// A Cross Agent Team pane must name its binary, through the instance's command
/// override or through [`COMMAND_ENV`]; a plain kimi pane falls back to the
/// registry binary, because nothing about it depends on the remote engine.
pub fn command_words(
    cross_agent_team: bool,
    command_override: Option<&str>,
) -> Result<Vec<String>> {
    match resolve_command(command_override)? {
        Some(words) => Ok(words),
        None if cross_agent_team => Err(missing_command_error()),
        None => Ok(vec!["kimi".to_string()]),
    }
}

fn require_configured_command(command_override: Option<&str>) -> Result<()> {
    if resolve_command(command_override)?.is_none() {
        return Err(missing_command_error());
    }
    Ok(())
}

/// The pane's own command override wins over the environment default: it is the
/// more specific statement of which binary this pane runs, and it is what the
/// pane will actually launch.
fn resolve_command(command_override: Option<&str>) -> Result<Option<Vec<String>>> {
    let Some(raw) = command_override.filter(|value| !value.trim().is_empty()) else {
        return configured_command();
    };
    let words = shell_words::split(raw).context("invalid kimi command override quoting")?;
    if words.is_empty() {
        return configured_command();
    }
    Ok(Some(words))
}

fn configured_command() -> Result<Option<Vec<String>>> {
    let raw = match std::env::var(COMMAND_ENV) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            bail!("{COMMAND_ENV} must contain valid Unicode")
        }
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let words =
        shell_words::split(&raw).with_context(|| format!("invalid {COMMAND_ENV} quoting"))?;
    if words.is_empty() {
        return Ok(None);
    }
    Ok(Some(words))
}

fn missing_command_error() -> anyhow::Error {
    anyhow::anyhow!(
        "a Cross Agent Team kimi pane must name the kimi build that can attach \
         to the shared server's engine, through this session's command override \
         or through {COMMAND_ENV}. Resolving `kimi` from the search path would \
         silently run a CLI without remote engine support, which puts every \
         poked turn in a session you cannot see."
    )
}

/// Launch-safe extra arguments for a kimi pane.
///
/// Everything the runtime owns is refused: AoE picked the session, the server
/// and the engine mode for this pane, and an argument that changes any of them
/// changes which conversation the pane is, without the durable slot noticing.
pub fn parse_and_validate_extra_args(value: &str) -> Result<Vec<String>> {
    let args = shell_words::split(value).context("invalid kimi extra args quoting")?;
    for argument in &args {
        let head = argument.split('=').next().unwrap_or(argument);
        if matches!(
            head,
            "--session"
                | "-s"
                | "--continue"
                | "-c"
                | "--resume"
                | "--cwd"
                | "--dir"
                | "--port"
                | "--host"
                | "--prompt"
                | "-p"
                | "--yolo"
        ) {
            bail!("kimi pane does not support extra argument '{argument}'");
        }
        if argument.starts_with('<') || argument.starts_with('>') || argument.contains('\n') {
            bail!("kimi pane does not support extra argument '{argument}'");
        }
    }
    Ok(args)
}

/// `default_model` from the kimi config, the same value the TUI would pick.
///
/// A server-created session carries no model, and a server driven turn -- which
/// is what an xats poke is -- fails instantly without one.
fn default_model(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join("config.toml"))
        .map_err(|error| tracing::debug!("no kimi config.toml: {error}"))
        .ok()?;
    let parsed = toml::from_str::<toml::Table>(&text)
        .map_err(|error| tracing::warn!("kimi config.toml is not valid TOML: {error}"))
        .ok()?;
    let model = parsed.get("default_model")?.as_str()?.trim().to_string();
    (!model.is_empty()).then_some(model)
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kimi::test_support::{spawn_fake_kimi, FakeReply};

    /// A kimi home whose registry advertises `port`, with a mounted IPC socket,
    /// a token, a default model and a conforming MCP config.
    fn capable_home(port: u16) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let server = home.path().join("server");
        std::fs::create_dir_all(server.join("instances")).unwrap();
        std::fs::write(
            server.join("instances").join("only.json"),
            format!(
                r#"{{"server_id":"only","pid":{},"host":"127.0.0.1","port":{port},
                     "started_at":1,"heartbeat_at":2,"host_version":"0.34.0"}}"#,
                std::process::id()
            ),
        )
        .unwrap();
        std::fs::write(server.join(format!("klient-{port}.sock")), "").unwrap();
        std::fs::write(home.path().join("server.token"), "sk-token\n").unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "default_model = \"kimi-code/k3\"\n",
        )
        .unwrap();
        std::fs::write(
            home.path().join("mcp.json"),
            r#"{"mcpServers":{"cross-agent-teams":{"url":"http://127.0.0.1:9100/mcp",
                "scope":"session","headers":{"X-Kimi-Session-Id":"${KIMI_XATS_SESSION_ID}"}}}}"#,
        )
        .unwrap();
        home
    }

    fn created(id: &str, cwd: &str) -> String {
        format!(r#"{{"data":{{"id":"{id}","metadata":{{"cwd":"{cwd}"}}}}}}"#)
    }

    struct EnvGuard(Vec<(&'static str, Option<String>)>);

    impl EnvGuard {
        fn set(pairs: &[(&'static str, &str)]) -> Self {
            let saved = pairs
                .iter()
                .map(|(name, value)| {
                    let previous = std::env::var(name).ok();
                    std::env::set_var(name, value);
                    (*name, previous)
                })
                .collect();
            Self(saved)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, previous) in &self.0 {
                match previous {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    fn fresh_request(cwd: &str) -> PaneRequest {
        PaneRequest {
            working_directory: PathBuf::from(cwd),
            cross_agent_team: true,
            mode: SessionMode::Fresh,
            durable_session_id: String::new(),
            command_override: None,
        }
    }

    /// Two panes in one directory each mint their own conversation, and neither
    /// asks the server for a session list to pick one.
    #[test]
    #[serial_test::serial]
    fn same_directory_panes_each_mint_their_own_session() {
        let cwd = "/tmp/kimi-pane";
        let server = spawn_fake_kimi(vec![
            FakeReply::ok(&created("session_left", cwd)),
            FakeReply::ok(r#"{"data":{}}"#),
            FakeReply::ok(r#"{"data":{"messages":[]}}"#),
            FakeReply::ok(&created("session_right", cwd)),
            FakeReply::ok(r#"{"data":{}}"#),
            FakeReply::ok(r#"{"data":{"messages":[]}}"#),
        ]);
        let port = server.base_url.rsplit(':').next().unwrap().parse().unwrap();
        let home = capable_home(port);
        let _env = EnvGuard::set(&[
            (
                server::KIMI_HOME_ENV_FOR_TESTS,
                home.path().to_str().unwrap(),
            ),
            (COMMAND_ENV, "/opt/kimi-dev/kimi"),
        ]);

        let left = prepare_session(&fresh_request(cwd)).unwrap();
        let right = prepare_session(&fresh_request(cwd)).unwrap();
        assert_eq!(left.session_id, "session_left");
        assert_eq!(right.session_id, "session_right");
        assert_eq!(left.base_url, right.base_url);
        for _ in 0..6 {
            let request = server.requests.recv().unwrap();
            assert!(!request.contains("GET /api/v1/sessions HTTP"), "{request}");
        }
        server.worker.join().unwrap();
    }

    /// Resume reuses the durable id and never falls through to minting.
    #[test]
    #[serial_test::serial]
    fn resume_reuses_the_durable_session_and_mints_nothing() {
        let cwd = "/tmp/kimi-pane";
        let server = spawn_fake_kimi(vec![FakeReply::ok(&created("session_keep", cwd))]);
        let port = server.base_url.rsplit(':').next().unwrap().parse().unwrap();
        let home = capable_home(port);
        let _env = EnvGuard::set(&[
            (
                server::KIMI_HOME_ENV_FOR_TESTS,
                home.path().to_str().unwrap(),
            ),
            (COMMAND_ENV, "/opt/kimi-dev/kimi"),
        ]);

        let launch = prepare_session(&PaneRequest {
            working_directory: PathBuf::from(cwd),
            cross_agent_team: true,
            mode: SessionMode::Resume,
            durable_session_id: "session_keep".to_string(),
            command_override: None,
        })
        .unwrap();
        assert_eq!(launch.session_id, "session_keep");
        let request = server.requests.recv().unwrap();
        assert!(request.starts_with("GET /api/v1/sessions/session_keep HTTP/1.1"));
        assert!(server.requests.try_recv().is_err());
        server.worker.join().unwrap();
    }

    /// Each Cross Agent Team gate refuses on its own, before any session is
    /// minted, and none of them degrades into a plain launch.
    #[test]
    #[serial_test::serial]
    fn every_cross_agent_team_gate_fails_closed_before_minting() {
        let cwd = "/tmp/kimi-pane";
        let server = spawn_fake_kimi(Vec::new());
        let port: u16 = server.base_url.rsplit(':').next().unwrap().parse().unwrap();
        let home = capable_home(port);
        let _env = EnvGuard::set(&[
            (
                server::KIMI_HOME_ENV_FOR_TESTS,
                home.path().to_str().unwrap(),
            ),
            (COMMAND_ENV, "/opt/kimi-dev/kimi"),
        ]);

        std::fs::remove_file(
            home.path()
                .join("server")
                .join(format!("klient-{port}.sock")),
        )
        .unwrap();
        assert!(
            format!("{:#}", prepare_session(&fresh_request(cwd)).unwrap_err())
                .contains("klient IPC socket")
        );
        std::fs::write(
            home.path()
                .join("server")
                .join(format!("klient-{port}.sock")),
            "",
        )
        .unwrap();

        std::fs::remove_file(home.path().join("mcp.json")).unwrap();
        assert!(
            format!("{:#}", prepare_session(&fresh_request(cwd)).unwrap_err()).contains("mcp.json")
        );
        std::fs::write(
            home.path().join("mcp.json"),
            r#"{"mcpServers":{"cross-agent-teams":{"url":"http://127.0.0.1:9100/mcp",
                "scope":"session","headers":{"X-Kimi-Session-Id":"${KIMI_XATS_SESSION_ID}"}}}}"#,
        )
        .unwrap();

        std::env::remove_var(COMMAND_ENV);
        assert!(
            format!("{:#}", prepare_session(&fresh_request(cwd)).unwrap_err())
                .contains(COMMAND_ENV)
        );
        std::env::set_var(COMMAND_ENV, "/opt/kimi-dev/kimi");

        std::fs::remove_file(home.path().join("server.token")).unwrap();
        assert!(
            format!("{:#}", prepare_session(&fresh_request(cwd)).unwrap_err())
                .contains("server.token")
        );

        // The session's own command override names the binary just as well, so
        // the gate lets it through: with no COMMAND_ENV at all this reaches the
        // token step rather than being refused for an unnamed binary.
        std::env::remove_var(COMMAND_ENV);
        let overridden = PaneRequest {
            command_override: Some("/opt/kimi-dev/kimi".to_string()),
            ..fresh_request(cwd)
        };
        assert!(format!("{:#}", prepare_session(&overridden).unwrap_err()).contains("server.token"));

        // Nothing above reached the server.
        assert!(server.requests.try_recv().is_err());
        server.worker.join().unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn a_plain_kimi_pane_needs_no_explicit_command_but_a_team_pane_does() {
        let _env = EnvGuard::set(&[(COMMAND_ENV, "")]);
        std::env::remove_var(COMMAND_ENV);
        assert_eq!(command_words(false, None).unwrap(), ["kimi"]);
        assert!(command_words(true, None).is_err());
        // A pane's own command override names the binary on its own, and wins
        // over the environment default when both are present.
        assert_eq!(
            command_words(true, Some("/opt/kimi-dev/kimi")).unwrap(),
            ["/opt/kimi-dev/kimi"]
        );
        std::env::set_var(COMMAND_ENV, "tsx --tsconfig dev.json main.ts");
        assert_eq!(
            command_words(true, None).unwrap(),
            ["tsx", "--tsconfig", "dev.json", "main.ts"]
        );
        assert_eq!(
            command_words(true, Some("/opt/kimi-dev/kimi")).unwrap(),
            ["/opt/kimi-dev/kimi"]
        );
        assert_eq!(
            command_words(true, Some("   ")).unwrap(),
            ["tsx", "--tsconfig", "dev.json", "main.ts"]
        );
    }

    #[test]
    fn the_default_model_comes_from_the_users_kimi_config() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(default_model(home.path()), None);
        std::fs::write(
            home.path().join("config.toml"),
            "default_model = \"kimi-code/k3\"\n[thinking]\nenabled = true\n",
        )
        .unwrap();
        assert_eq!(default_model(home.path()).as_deref(), Some("kimi-code/k3"));
    }

    #[test]
    fn extra_args_reject_everything_the_runtime_already_decided() {
        for value in [
            "--session session_other",
            "-s session_other",
            "--continue",
            "--resume",
            "--cwd /tmp/other",
            "--port=1234",
            "--prompt hello",
            "--yolo",
        ] {
            assert!(
                parse_and_validate_extra_args(value).is_err(),
                "{value} should be rejected"
            );
        }
        assert_eq!(
            parse_and_validate_extra_args("--theme dark").unwrap(),
            ["--theme", "dark"]
        );
        assert!(parse_and_validate_extra_args("").unwrap().is_empty());
    }
}
