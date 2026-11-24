mod binding;
mod deferred;
pub(crate) use deferred::{probe_capabilities, require_deferred_bridge};
mod lease;
mod named;
mod stdio;
mod support;
pub(crate) use support::check_named_support;
mod tools;
mod upstream;
mod wire;

#[cfg(test)]
mod tests;

use std::time::Duration;

use anyhow::{ensure, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::{FramedRead, LinesCodec};
use upstream::Upstream;
use uuid::{Uuid, Variant, Version};

use crate::xats_control::discover_control_plane;

const LIMIT: usize = 1024 * 1024;
const IDLE: Duration = Duration::from_secs(300);
const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }
}

fn validate_key(key: &str) -> Result<()> {
    let uuid = Uuid::parse_str(key)
        .ok()
        .context("Invalid Antigravity pane UUID")?;
    ensure!(
        uuid.get_version() == Some(Version::Random)
            && uuid.get_variant() == Variant::RFC4122
            && uuid.hyphenated().to_string() == key,
        "Expected a canonical random Antigravity pane UUID"
    );
    Ok(())
}

pub async fn run() -> Result<()> {
    if deferred::requested(std::env::var_os("AOE_ANTIGRAVITY_DEFERRED").as_deref())? {
        return deferred::run();
    }
    let key = crate::agents::antigravity::bridge_key()?;
    if let Some(key) = &key {
        validate_key(key)?;
    }
    let _lease = key
        .as_deref()
        .map(|key| lease::acquire(&crate::session::get_app_dir()?, key))
        .transpose()?;
    let mode = match key {
        Some(key) if std::env::var("AOE_ANTIGRAVITY_NAMED").as_deref() == Ok("1") => {
            let launch = named::Launch::from_env()?;
            Mode::Named(named::Named::new(launch, move || {
                ensure!(
                    crate::agents::antigravity::bridge_key()?.as_ref() == Some(&key),
                    "Named Antigravity no longer owns its slot"
                );
                let cwd = std::env::current_dir()?;
                crate::hooks::antigravity_xats::check_named_mcp(
                    cwd.to_str()
                        .context("Invalid Antigravity working directory")?,
                )?;
                Upstream::new(discover_control_plane()?, key.clone())
            }))
        }
        Some(key) => Mode::Plain(Upstream::new(discover_control_plane()?, key)?),
        None => Mode::Disabled,
    };
    let mut bridge = Bridge::with_mode(mode);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = tokio::select! {
        result = stdio::serve(&mut bridge) => result,
        _ = term.recv() => Ok(()),
    };
    bridge.finish(result).await
}

enum Mode {
    Disabled,
    Plain(Upstream),
    Named(named::Named),
}

struct Bridge {
    mode: Mode,
    version: Option<String>,
    initialized: bool,
}

impl Bridge {
    #[cfg(test)]
    fn new(upstream: Option<Upstream>) -> Self {
        Self::with_mode(upstream.map_or(Mode::Disabled, Mode::Plain))
    }

    fn with_mode(mode: Mode) -> Self {
        Self {
            mode,
            version: None,
            initialized: false,
        }
    }

    async fn close(&mut self) -> Result<()> {
        match &mut self.mode {
            Mode::Plain(up) => up.close().await,
            Mode::Named(named) => named.finish(Ok(())).await,
            Mode::Disabled => Ok(()),
        }
    }

    async fn finish(&mut self, result: Result<()>) -> Result<()> {
        match (result, self.close().await) {
            (Err(error), Err(close)) => {
                Err(error.context(format!("Closing xats also failed: {close:#}")))
            }
            (result, close) => result.and(close),
        }
    }

    async fn serve(
        &mut self,
        reader: impl AsyncRead + Unpin,
        mut writer: impl AsyncWrite + Unpin,
        idle: Duration,
    ) -> Result<()> {
        let mut lines = FramedRead::new(reader, LinesCodec::new_with_max_length(LIMIT));
        let mut heartbeat = tokio::time::interval(idle.min(Duration::from_secs(30)));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let line = tokio::select! {
                next = lines.next() => match next.transpose().context("Reading Antigravity MCP line")? {
                    Some(line) => line,
                    None => return Ok(()),
                },
                _ = tokio::time::sleep(idle), if matches!(&self.mode, Mode::Plain(up) if up.active()) => {
                    self.close().await?;
                    continue;
                }
                _ = heartbeat.tick(), if self.initialized && matches!(self.mode, Mode::Named(_)) => {
                    if let (Mode::Named(named), Some(version)) = (&mut self.mode, &self.version) {
                        if let Err(e) = named.ping(version).await {
                            tracing::warn!("Named Antigravity heartbeat failed: {e:#}");
                            eprintln!("Named Antigravity heartbeat failed: {e:#}");
                        }
                    }
                    continue;
                }
            };
            let Some(reply) = self.line(&line).await else {
                continue;
            };
            let mut bytes = serde_json::to_vec(&reply)?;
            if bytes.len() > LIMIT {
                bytes = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":reply["id"],
                    "error":{"code":-32000,"message":"MCP response exceeds size limit"}}))?;
            }
            bytes.push(b'\n');
            tokio::time::timeout(Duration::from_secs(10), async {
                writer.write_all(&bytes).await?;
                writer.flush().await
            })
            .await
            .context("Writing Antigravity MCP response timed out")??;
        }
    }

    async fn line(&mut self, line: &str) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(msg) => msg,
            Err(_) => return Some(error_reply(Value::Null, -32700, "Invalid JSON")),
        };
        let method = msg["method"].as_str();
        if msg["jsonrpc"] != "2.0" || method.is_none() {
            return Some(error_reply(Value::Null, -32600, "Invalid MCP request"));
        }
        let method = method?;
        let Some(id) = msg.get("id") else {
            if method == "notifications/initialized" && self.version.is_some() {
                self.initialized = true;
            }
            return None;
        };
        if !id.as_str().is_some_and(|s| s.len() <= 1024) && !id.is_i64() && !id.is_u64() {
            return Some(error_reply(Value::Null, -32600, "Invalid MCP request id"));
        }
        Some(match self.dispatch(method, &msg["params"]).await {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(e) => {
                let code = e.downcast_ref::<RpcError>().map_or(-32000, |e| e.code);
                let mut reason = format!("{e:#}");
                if let Mode::Plain(up) = &self.mode {
                    reason = up.redact(&reason);
                }
                error_reply(id.clone(), code, &reason)
            }
        })
    }

    async fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value> {
        if method == "initialize" {
            ensure!(self.version.is_none(), "MCP session already initialized");
            let requested = params["protocolVersion"]
                .as_str()
                .ok_or_else(|| RpcError::invalid("Missing protocolVersion"))?;
            let version = if let Mode::Plain(up) = &mut self.mode {
                up.connect(requested).await?.0
            } else if VERSIONS.contains(&requested) {
                requested.to_owned()
            } else {
                VERSIONS[0].to_owned()
            };
            self.version = Some(version.clone());
            return Ok(
                json!({"protocolVersion":version,"capabilities":{"tools":{}},
                "serverInfo":{"name":"aoe-antigravity-xats","version":env!("CARGO_PKG_VERSION")}}),
            );
        }
        if method == "ping" {
            return Ok(json!({}));
        }
        ensure!(self.initialized, "MCP handshake incomplete");
        let version = self.version.as_deref().context("Missing MCP version")?;
        match method {
            "tools/list" => {
                let available = match &mut self.mode {
                    Mode::Disabled => Vec::new(),
                    Mode::Plain(_) => tools::list(),
                    Mode::Named(named) => named.list_tools(version).await?,
                };
                Ok(json!({"tools":available}))
            }
            "tools/call" => {
                let up = match &mut self.mode {
                    Mode::Disabled => return Err(RpcError::invalid("No tools enabled").into()),
                    Mode::Named(named) => return named.call(version, params).await,
                    Mode::Plain(up) => up,
                };
                let args = tools::prepare(params, &up.key)?;
                if !up.active() {
                    if let Some(binding) = &up.binding {
                        binding.check(&args)?;
                    }
                    let reconnect = args["name"] == "reconnect";
                    ensure!(!up.lost || reconnect, "xats session expired. Use reconnect");
                    let (_, restored) = up.connect(version).await?;
                    if reconnect {
                        return Ok(restored);
                    }
                }
                up.call(args).await
            }
            _ => Err(RpcError::new(-32601, "Method not found").into()),
        }
    }
}

fn error_reply(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
