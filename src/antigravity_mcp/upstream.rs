use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use reqwest::{header::HeaderValue, redirect::Policy, Client, Method, RequestBuilder, Url};
use serde_json::{json, Value};

use super::{binding::Binding, wire, RpcError};
use crate::xats_control::ControlPlane;

pub(super) struct Upstream {
    client: Client,
    control: ControlPlane,
    url: Url,
    pub(super) key: String,
    sid: Option<HeaderValue>,
    version: Option<String>,
    id: u64,
    pub(super) binding: Option<Binding>,
    ready: bool,
    pub(super) lost: bool,
}

impl Upstream {
    pub(super) fn new(control: ControlPlane, key: String) -> Result<Self> {
        ensure!(
            control.base_url.scheme() == "http" && control.base_url.host_str() == Some("127.0.0.1"),
            "Expected loopback xats daemon"
        );
        let url = control.base_url.join("/mcp")?;
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            control,
            client,
            url,
            key,
            sid: None,
            version: None,
            id: 0,
            binding: None,
            ready: false,
            lost: false,
        })
    }

    pub(super) async fn registered_agent(
        &self,
    ) -> Result<Option<crate::agents::antigravity::launcher::RegisteredAgent>> {
        crate::agents::antigravity::launcher::registered_agent(
            &self.control,
            &self.client,
            &self.key,
        )
        .await
    }

    pub(super) fn active(&self) -> bool {
        self.ready
    }

    fn request(&self, method: Method) -> RequestBuilder {
        let mut req = self
            .client
            .request(method, self.url.clone())
            .header("accept", "application/json, text/event-stream");
        if let Some(token) = &self.control.token {
            req = req.bearer_auth(token.expose());
        }
        if let Some(sid) = &self.sid {
            req = req.header("mcp-session-id", sid.clone());
        }
        if let Some(version) = &self.version {
            req = req.header("mcp-protocol-version", version);
        }
        req
    }

    pub(super) async fn rpc(&mut self, method: &str, params: &Value) -> Result<Value> {
        self.id += 1;
        let mut req = self.request(Method::POST);
        if method == "tools/call" {
            req = req.timeout(Duration::from_secs(120));
        }
        let response = req
            .json(&json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}))
            .send()
            .await
            .with_context(|| format!("Sending xats {method}"))?;
        if response.status().as_u16() == 404 && self.sid.is_some() {
            let hint = if self.ready { ". Use reconnect" } else { "" };
            self.sid = None;
            self.version = None;
            self.ready = false;
            self.lost = true;
            bail!("xats {method}: HTTP 404, session expired{hint}");
        }
        ensure!(
            response.status().as_u16() == 200,
            "xats {method}: HTTP {}",
            response.status()
        );
        if method == "initialize" {
            let mut sid = response
                .headers()
                .get("mcp-session-id")
                .filter(|h| !h.is_empty() && h.len() <= 256)
                .context("Missing or invalid xats Mcp-Session-Id")?
                .clone();
            sid.set_sensitive(true);
            self.sid = Some(sid);
        } else if let Some(sid) = response.headers().get("mcp-session-id") {
            ensure!(
                Some(sid) == self.sid.as_ref(),
                "xats changed Mcp-Session-Id"
            );
        }
        wire::read_reply(response, self.id).await
    }

    pub(super) async fn connect(&mut self, requested: &str) -> Result<(String, Value)> {
        self.close().await?;
        match self.open(requested).await {
            Ok(pair) => {
                self.ready = true;
                self.lost = false;
                Ok(pair)
            }
            Err(e) => {
                let reason = self.redact(&format!("{e:#}"));
                let code = e.downcast_ref::<RpcError>().map_or(-32000, |e| e.code);
                self.close()
                    .await
                    .with_context(|| format!("xats handshake failed: {reason}"))?;
                Err(RpcError::new(code, reason).into())
            }
        }
    }

    async fn open(&mut self, requested: &str) -> Result<(String, Value)> {
        let version = self.handshake(requested).await?;
        let result = self
            .call(json!({"name":"reconnect","arguments":{"identity_key":self.key}}))
            .await?;
        check_reconnect(&result)?;
        Ok((version, result))
    }

    pub(super) async fn handshake(&mut self, requested: &str) -> Result<String> {
        let init = self
            .rpc(
                "initialize",
                &json!({
                    "protocolVersion":requested,"capabilities":{},
                    "clientInfo":{"name":"Antigravity","version":env!("CARGO_PKG_VERSION")}
                }),
            )
            .await?;
        let version = init["protocolVersion"]
            .as_str()
            .filter(|v| super::VERSIONS.contains(v))
            .context("Unsupported xats protocol version")?;
        ensure!(
            init["capabilities"]["tools"].is_object(),
            "xats tools unavailable"
        );
        self.version = Some(version.to_owned());
        let response = self
            .request(Method::POST)
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await
            .context("Sending xats initialized notification")?;
        ensure!(
            response.status().as_u16() == 202,
            "xats notifications/initialized: HTTP {}",
            response.status()
        );
        Ok(version.to_owned())
    }

    pub(super) async fn call(&mut self, params: Value) -> Result<Value> {
        if let Some(binding) = &self.binding {
            binding.check(&params)?;
        }
        let mut result = self.rpc("tools/call", &params).await?;
        if let Err(error) = Binding::observe(&mut self.binding, &params, &result) {
            self.lost = true;
            self.close()
                .await
                .with_context(|| format!("Invalid xats identity result: {error:#}"))?;
            return Err(error);
        }
        let plain_registration = params["name"] == "register_agent"
            && params["arguments"]["agent_type"] == "custom"
            && result["isError"] != true;
        let set_delivery_hint = |body: &mut Value| {
            if plain_registration && body["agent_id"].is_string() && body.get("error").is_none() {
                body["hint"] = json!("xats 身份已注册, 当前可按需收信. 要启用空闲唤醒, 请在 AoE 首页选中这个会话并按小写 r 恢复重启, 原身份与对话会保留.");
            }
        };
        let is_comm_ok = params["name"]
            .as_str()
            .is_some_and(super::tools::communication)
            && result["isError"] != true;
        let redact_outer_token = !is_comm_ok
            || result
                .get("structuredContent")
                .is_some_and(|b| b.get("error").is_some());
        if let Some(body) = result.get_mut("structuredContent") {
            set_delivery_hint(body);
        }
        let content = result["content"]
            .as_array_mut()
            .context("Missing xats tool content")?;
        let blocks = std::mem::take(content);
        self.sanitize(&mut result, redact_outer_token);
        let mut clean = Vec::with_capacity(blocks.len());
        for block in blocks {
            ensure!(block["type"] == "text", "Unsupported xats tool content");
            let text = block["text"].as_str().context("Missing xats tool text")?;
            let text = match serde_json::from_str::<Value>(text) {
                Ok(mut inner) => {
                    set_delivery_hint(&mut inner);
                    let redact_token = !is_comm_ok || inner.get("error").is_some();
                    self.sanitize(&mut inner, redact_token);
                    inner.to_string()
                }
                Err(_) => self.redact_with(text, !is_comm_ok),
            };
            clean.push(json!({"type":"text", "text":text}));
        }
        result["content"] = Value::Array(clean);
        Ok(result)
    }

    pub(super) async fn close(&mut self) -> Result<()> {
        self.ready = false;
        if self.sid.is_some() {
            let response = self
                .request(Method::DELETE)
                .send()
                .await
                .context("Closing xats session")?;
            ensure!(
                response.status().is_success() || response.status().as_u16() == 404,
                "Closing xats session: HTTP {}",
                response.status()
            );
            self.sid = None;
            self.version = None;
        }
        Ok(())
    }

    pub(super) async fn finish_session<T>(&mut self, result: Result<T>) -> Result<T> {
        let result = result.map_err(|e| anyhow::anyhow!(self.redact(&format!("{e:#}"))));
        let close = self.close().await;
        match (result, close) {
            (Err(error), Err(close)) => {
                Err(error.context(format!("Closing xats also failed: {close:#}")))
            }
            (result, Ok(())) => result,
            (Ok(_), Err(close)) => Err(close),
        }
    }

    pub(super) fn redact(&self, text: &str) -> String {
        self.redact_with(text, true)
    }

    fn redact_with(&self, text: &str, include_token: bool) -> String {
        let mut text = text.replace(&self.key, "[redacted]");
        let token = include_token
            .then(|| self.control.token.as_ref().map(|s| s.expose()))
            .flatten();
        for secret in [token, self.sid.as_ref().and_then(|s| s.to_str().ok())]
            .into_iter()
            .flatten()
        {
            text = text.replace(secret, "[redacted]");
            let quoted = json!(secret).to_string();
            text = text.replace(&quoted[1..quoted.len() - 1], "[redacted]");
        }
        text
    }

    fn sanitize(&self, value: &mut Value, include_token: bool) {
        match value {
            Value::String(s) => *s = self.redact_with(s, include_token),
            Value::Array(a) => a.iter_mut().for_each(|v| self.sanitize(v, include_token)),
            Value::Object(o) => {
                for (key, mut v) in std::mem::take(o) {
                    if matches!(key.as_str(), "identity_key" | "token" | "bearer_token")
                        && v.is_string()
                    {
                        v = json!("[redacted]");
                    } else {
                        self.sanitize(&mut v, include_token);
                    }
                    o.insert(self.redact_with(&key, include_token), v);
                }
            }
            _ => {}
        }
    }
}

fn check_reconnect(result: &Value) -> Result<()> {
    let text = result["content"][0]["text"]
        .as_str()
        .context("Missing xats reconnect result")?;
    if result["isError"] == true {
        bail!("xats reconnect failed: {text}");
    }
    let body: Value = serde_json::from_str(text).context("Invalid xats reconnect result")?;
    if body.get("error").is_some() || body.get("ambiguous").is_some() {
        return Err(RpcError::new(-32000, format!("xats reconnect failed: {text}")).into());
    }
    ensure!(
        body["need_register"] == true
            || (body["ok"] == true
                && body["agent_id"].is_string()
                && body["name"].is_string()
                && body["team"].is_string()),
        "Unrecognized xats reconnect result"
    );
    Ok(())
}
