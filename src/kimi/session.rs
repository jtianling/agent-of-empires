//! Exact kimi session lifecycle over the shared server's REST API.
//!
//! Every step runs before the pane process starts, because `kimi --session <id>`
//! attaches to a session that must already exist, already carry a model, and
//! already have its main agent materialized.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// The permission mode of the session itself, which is not the pane's yolo
/// setting. A server driven turn -- an xats poke -- has no one to answer an
/// approval prompt, so a session that can be poked at all has to run unattended;
/// the pane's own yolo setting still decides what the TUI passes on its command
/// line for turns the user drives.
const DEFAULT_PERMISSION_MODE: &str = "yolo";

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct SessionRecord {
    id: String,
    #[serde(default)]
    metadata: SessionMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct SessionMetadata {
    #[serde(default)]
    cwd: Option<String>,
}

/// Session ids are `session_<uuid>`; the shape is fixed by the server and the
/// value reaches a shell command line, so nothing outside this alphabet passes.
pub fn validate_session_id(value: &str) -> Result<()> {
    let valid = value.strip_prefix("session_").is_some_and(|rest| {
        !rest.is_empty()
            && value.len() <= 256
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    });
    if !valid {
        bail!("invalid kimi session id")
    }
    Ok(())
}

pub(crate) fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("building kimi server HTTP client")
}

/// Create a session whose recorded working directory is the pane's, give it a
/// model and permission mode, and materialize its main agent.
///
/// All three steps are mandatory. A session without a model fails every
/// server-driven turn, which is exactly the path an xats poke takes, and the
/// CLI refuses to attach a session whose main agent has never been resolved.
pub(crate) async fn mint(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    working_directory: &Path,
    model: Option<&str>,
) -> Result<String> {
    let cwd = working_directory
        .to_str()
        .context("kimi pane working directory is not valid UTF-8")?;
    let record = create_session(client, base_url, token, cwd).await?;
    validate_session_id(&record.id)?;
    require_recorded_cwd(&record, cwd)?;
    if let Some(model) = model {
        set_profile(client, base_url, token, &record.id, model).await?;
    } else {
        bail!(
            "kimi has no default model configured, so a server driven turn in a \
             new session would fail immediately. Set `default_model` in the kimi \
             config before launching a Cross Agent Team kimi pane."
        );
    }
    materialize_main_agent(client, base_url, token, &record.id).await?;
    Ok(record.id)
}

/// Confirm a durable session still exists and still belongs to this pane's
/// directory. `kimi --session <id>` hard-fails when the two disagree, so the
/// mismatch is worth reporting before the pane is torn down and respawned.
pub(crate) async fn verify(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    working_directory: &Path,
    session_id: &str,
) -> Result<()> {
    validate_session_id(session_id)?;
    let cwd = working_directory
        .to_str()
        .context("kimi pane working directory is not valid UTF-8")?;
    let url = session_url(base_url, session_id, &[])?;
    let record: SessionRecord = get_json(client, url, token, "loading kimi session").await?;
    if record.id != session_id {
        bail!(
            "kimi returned session '{}' while '{}' was requested",
            record.id,
            session_id
        );
    }
    require_recorded_cwd(&record, cwd)
}

fn require_recorded_cwd(record: &SessionRecord, cwd: &str) -> Result<()> {
    match record.metadata.cwd.as_deref() {
        Some(recorded) if recorded == cwd => Ok(()),
        Some(recorded) => bail!(
            "kimi session '{}' records working directory '{}' but the pane runs \
             in '{}'; the TUI refuses to attach across that mismatch",
            record.id,
            recorded,
            cwd
        ),
        None => bail!(
            "kimi session '{}' records no working directory; the TUI refuses to \
             attach a session it cannot match against '{}'",
            record.id,
            cwd
        ),
    }
}

async fn create_session(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    cwd: &str,
) -> Result<SessionRecord> {
    let url = sessions_url(base_url)?;
    let body = serde_json::json!({ "metadata": { "cwd": cwd } });
    post_json(client, url, token, &body, "creating exact kimi session").await
}

async fn set_profile(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    session_id: &str,
    model: &str,
) -> Result<()> {
    let url = session_url(base_url, session_id, &["profile"])?;
    let body = serde_json::json!({
        "agent_config": { "model": model, "permission_mode": DEFAULT_PERMISSION_MODE }
    });
    send(
        client.post(url).json(&body),
        token,
        "setting kimi session profile",
    )
    .await
    .map(|_| ())
}

/// Materialize the main agent through the read path that does it as a side
/// effect: it is synchronous, leaves no message in the transcript, and its
/// success doubles as proof the session is visible to a reader.
async fn materialize_main_agent(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    session_id: &str,
) -> Result<()> {
    let url = session_url(base_url, session_id, &["messages"])?;
    send(
        client.get(url),
        token,
        "materializing the kimi session main agent",
    )
    .await
    .map(|_| ())
}

fn sessions_url(base_url: &str) -> Result<reqwest::Url> {
    reqwest::Url::parse(&format!("{base_url}/api/v1/sessions"))
        .context("building kimi sessions URL")
}

fn session_url(base_url: &str, session_id: &str, tail: &[&str]) -> Result<reqwest::Url> {
    let mut url = sessions_url(base_url)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid kimi base URL"))?
        .push(session_id)
        .extend(tail);
    Ok(url)
}

async fn post_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: reqwest::Url,
    token: &str,
    body: &serde_json::Value,
    what: &str,
) -> Result<T> {
    let bytes = send(client.post(url).json(body), token, what).await?;
    decode(&bytes, what)
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: reqwest::Url,
    token: &str,
    what: &str,
) -> Result<T> {
    let bytes = send(client.get(url), token, what).await?;
    decode(&bytes, what)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8], what: &str) -> Result<T> {
    let envelope: Envelope<T> = serde_json::from_slice(bytes)
        .with_context(|| format!("invalid kimi server response while {what}"))?;
    Ok(envelope.data)
}

/// Every kimi REST call is bounded twice: the client's deadline covers the wall
/// clock, and the body is read in chunks against a hard ceiling so a server
/// that omits `Content-Length` cannot make AoE buffer without limit.
async fn send(request: reqwest::RequestBuilder, token: &str, what: &str) -> Result<Vec<u8>> {
    let mut response = request
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("{what} on the kimi server"))?;
    let status = response.status();
    if !status.is_success() {
        bail!("{what} failed: kimi server returned HTTP {status}");
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
    {
        bail!("{what} failed: kimi server response exceeds size limit");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("reading the kimi server response while {what}"))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            bail!("{what} failed: kimi server response exceeds size limit");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kimi::test_support::{spawn_fake_kimi, FakeReply};

    #[test]
    fn session_ids_are_accepted_only_in_the_servers_own_shape() {
        assert!(validate_session_id("session_e96b6682-6967-4e2c-ab5d-f7591b5a5a9a").is_ok());
        assert!(validate_session_id("ses_left").is_err());
        assert!(validate_session_id("session_").is_err());
        assert!(validate_session_id("session_a;rm -rf /").is_err());
        assert!(validate_session_id("").is_err());
    }

    #[tokio::test]
    async fn mint_creates_sets_profile_and_materializes_without_sending_a_message() {
        let created = r#"{"data":{"id":"session_new","metadata":{"cwd":"/tmp/pane"}}}"#;
        let server = spawn_fake_kimi(vec![
            FakeReply::ok(created),
            FakeReply::ok(r#"{"data":{}}"#),
            FakeReply::ok(r#"{"data":{"messages":[]}}"#),
        ]);
        let session = mint(
            &build_client().unwrap(),
            &server.base_url,
            "sk-token",
            Path::new("/tmp/pane"),
            Some("kimi-code/k3"),
        )
        .await
        .unwrap();
        assert_eq!(session, "session_new");

        let create = server.requests.recv().unwrap();
        assert!(create.starts_with("POST /api/v1/sessions HTTP/1.1"));
        assert!(create
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-token"));
        assert!(create.contains(r#""cwd":"/tmp/pane""#));

        let profile = server.requests.recv().unwrap();
        assert!(profile.starts_with("POST /api/v1/sessions/session_new/profile HTTP/1.1"));
        assert!(profile.contains(r#""model":"kimi-code/k3""#));

        let materialize = server.requests.recv().unwrap();
        assert!(materialize.starts_with("GET /api/v1/sessions/session_new/messages HTTP/1.1"));
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn a_failed_profile_step_aborts_before_materialization() {
        let created = r#"{"data":{"id":"session_new","metadata":{"cwd":"/tmp/pane"}}}"#;
        let server = spawn_fake_kimi(vec![
            FakeReply::ok(created),
            FakeReply::status(500, r#"{"error":"model.not_configured"}"#),
        ]);
        let error = mint(
            &build_client().unwrap(),
            &server.base_url,
            "sk-token",
            Path::new("/tmp/pane"),
            Some("kimi-code/k3"),
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("setting kimi session profile"));
        assert!(server.requests.recv().is_ok());
        assert!(server.requests.recv().is_ok());
        assert!(server.requests.try_recv().is_err());
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn a_session_recorded_in_another_directory_is_rejected() {
        let created = r#"{"data":{"id":"session_new","metadata":{"cwd":"/tmp/elsewhere"}}}"#;
        let server = spawn_fake_kimi(vec![FakeReply::ok(created)]);
        let error = mint(
            &build_client().unwrap(),
            &server.base_url,
            "sk-token",
            Path::new("/tmp/pane"),
            Some("kimi-code/k3"),
        )
        .await
        .unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(diagnostic.contains("/tmp/elsewhere"));
        assert!(diagnostic.contains("/tmp/pane"));
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn minting_without_a_configured_model_fails_before_the_profile_call() {
        let created = r#"{"data":{"id":"session_new","metadata":{"cwd":"/tmp/pane"}}}"#;
        let server = spawn_fake_kimi(vec![FakeReply::ok(created)]);
        let error = mint(
            &build_client().unwrap(),
            &server.base_url,
            "sk-token",
            Path::new("/tmp/pane"),
            None,
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("default_model"));
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn verify_reads_the_exact_session_and_rejects_a_substitute() {
        let server = spawn_fake_kimi(vec![
            FakeReply::ok(r#"{"data":{"id":"session_keep","metadata":{"cwd":"/tmp/pane"}}}"#),
            FakeReply::ok(r#"{"data":{"id":"session_other","metadata":{"cwd":"/tmp/pane"}}}"#),
        ]);
        let client = build_client().unwrap();
        verify(
            &client,
            &server.base_url,
            "sk-token",
            Path::new("/tmp/pane"),
            "session_keep",
        )
        .await
        .unwrap();
        let request = server.requests.recv().unwrap();
        assert!(request.starts_with("GET /api/v1/sessions/session_keep HTTP/1.1"));

        let error = verify(
            &client,
            &server.base_url,
            "sk-token",
            Path::new("/tmp/pane"),
            "session_keep",
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("session_other"));
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn an_unbounded_response_body_is_cut_off_at_the_limit() {
        let server = crate::kimi::test_support::spawn_fake_kimi_without_content_length(
            MAX_RESPONSE_BYTES + 1,
        );
        let error = verify(
            &build_client().unwrap(),
            &server.base_url,
            "sk-token",
            Path::new("/tmp/pane"),
            "session_keep",
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("exceeds size limit"));
        server.worker.join().unwrap();
    }
}
