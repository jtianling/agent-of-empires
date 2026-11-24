use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use uuid::{Uuid, Variant, Version};

use super::{support, tools, Upstream};
use crate::agents::antigravity::launcher::EXPECTED_AGENT_ENV;

const HANDOFF_RETRIES: usize = 12;
const HANDOFF_RETRY_DELAY: Duration = Duration::from_millis(250);

pub(super) struct Launch {
    id: String,
    name: String,
    team: String,
    expected_agent: Uuid,
}

impl Launch {
    pub(super) fn from_env() -> Result<Self> {
        Self::new(
            std::env::var("XATS_AGY_LAUNCH_ID").context("Reading XATS_AGY_LAUNCH_ID")?,
            std::env::var("XATS_AGY_NAME").context("Reading XATS_AGY_NAME")?,
            std::env::var("XATS_AGY_TEAM").context("Reading XATS_AGY_TEAM")?,
            &std::env::var(EXPECTED_AGENT_ENV).context("Reading expected Antigravity agent ID")?,
        )
    }

    pub(super) fn new(id: String, name: String, team: String, agent_id: &str) -> Result<Self> {
        let uuid = Uuid::parse_str(&id).context("Invalid Antigravity launch UUID")?;
        ensure!(
            uuid.get_version() == Some(Version::Random)
                && uuid.get_variant() == Variant::RFC4122
                && uuid.to_string().eq_ignore_ascii_case(&id),
            "Expected Antigravity launch UUID v4"
        );
        ensure!(
            [&name, &team]
                .iter()
                .all(|s| !s.is_empty() && s.trim() == **s),
            "Expected exact Antigravity launcher name and team"
        );
        ensure!(
            crate::hooks::antigravity::valid_id(agent_id),
            "Invalid expected Antigravity agent UUID"
        );
        Ok(Self {
            id,
            name,
            team,
            expected_agent: Uuid::parse_str(agent_id)?,
        })
    }

    async fn register(
        &self,
        up: &mut Upstream,
        retries: usize,
        retry_delay: Duration,
    ) -> Result<()> {
        for attempt in 0..=retries {
            let current = up
                .registered_agent()
                .await?
                .context("Named Antigravity slot key is no longer registered")?;
            ensure!(
                current.id == self.expected_agent
                    && current.name == self.name
                    && current.team == self.team,
                "Named Antigravity identity changed before registration"
            );
            let result = up
                .rpc(
                    "tools/call",
                    &json!({
                        "name":"register_agent", "arguments":{
                            "identity_key":up.key, "agent_type":"antigravity", "agy_launch_id":self.id,
                            "name":self.name, "team":self.team
                        }
                    }),
                )
                .await?;
            let body = payload(&result).context("Missing named xats registration result")?;
            if result["isError"] == true || body.get("error").is_some() {
                if attempt < retries && is_transient_handoff_error(&body) {
                    tokio::time::sleep(retry_delay).await;
                    continue;
                }
                bail!("Named xats registration failed: {body}");
            }
            let id = body["agent_id"]
                .as_str()
                .filter(|id| crate::hooks::antigravity::valid_id(id))
                .context("Missing or invalid named xats agent UUID")?;
            let id = Uuid::parse_str(id)?;
            ensure!(
                body["team"] == self.team
                    && body.get("name").map_or(true, |n| n == &self.name)
                    && self.expected_agent == id,
                "Named xats registration returned a different identity"
            );
            return Ok(());
        }
        unreachable!("register loop always returns")
    }
}

fn is_transient_handoff_error(body: &Value) -> bool {
    matches!(
        body["error"].as_str(),
        Some("agent_id_collision" | "agy_launcher_not_ready")
    )
}

fn payload(result: &Value) -> Option<Value> {
    if let Some(body) = result.get("structuredContent") {
        return Some(body.clone());
    }
    let text = result["content"][0]["text"].as_str()?;
    Some(serde_json::from_str(text).unwrap_or_else(|_| json!(text)))
}

pub(super) struct Named {
    launch: Launch,
    open: Box<dyn Fn() -> Result<Upstream> + Send + Sync>,
    session: Option<Upstream>,
    discovered_tools: Vec<Value>,
    register_retries: usize,
    register_retry_delay: Duration,
}

impl Named {
    pub(super) fn new(
        launch: Launch,
        open: impl Fn() -> Result<Upstream> + Send + Sync + 'static,
    ) -> Self {
        Self::with_retry(launch, open, HANDOFF_RETRIES, HANDOFF_RETRY_DELAY)
    }

    pub(super) fn with_retry(
        launch: Launch,
        open: impl Fn() -> Result<Upstream> + Send + Sync + 'static,
        register_retries: usize,
        register_retry_delay: Duration,
    ) -> Self {
        Self {
            launch,
            open: Box::new(open),
            session: None,
            discovered_tools: Vec::new(),
            register_retries,
            register_retry_delay,
        }
    }

    pub(super) async fn connect(&mut self, version: &str) -> Result<&mut Upstream> {
        if self.session.is_none() {
            let up = self.session.insert((self.open)()?);
            let retries = self.register_retries;
            let retry_delay = self.register_retry_delay;
            let result = async {
                up.handshake(version).await?;
                let discovered = support::check_schema(up).await?;
                self.launch.register(up, retries, retry_delay).await?;
                Ok::<_, anyhow::Error>(discovered)
            }
            .await;
            match result {
                Ok(discovered) => self.discovered_tools = discovered,
                Err(error) => return self.finish(Err(error)).await,
            }
        }
        self.session.as_mut().context("Missing named xats session")
    }

    pub(super) async fn list_tools(&mut self, version: &str) -> Result<Vec<Value>> {
        self.connect(version).await?;
        if !self.discovered_tools.is_empty() {
            return Ok(self.discovered_tools.clone());
        }
        Ok(tools::list()
            .into_iter()
            .filter(|t| t["name"].as_str().is_some_and(tools::communication))
            .collect())
    }

    pub(super) async fn ping(&mut self, version: &str) -> Result<()> {
        let up = self.connect(version).await?;
        match up.rpc("ping", &json!({})).await {
            Ok(_) => Ok(()),
            Err(e) => self.finish(Err(e)).await,
        }
    }

    pub(super) async fn call(&mut self, version: &str, params: &Value) -> Result<Value> {
        let args = tools::prepare_communication(params)?;
        let up = self.connect(version).await?;
        let result = up.call(args).await;
        match result {
            Ok(reply) if !payload(&reply).is_some_and(|p| p["error"] == "unknown_agent") => {
                Ok(reply)
            }
            result => self.finish(result).await,
        }
    }

    pub(super) async fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        match self.session.take() {
            Some(mut up) => up.finish_session(result).await,
            None => result,
        }
    }
}
