use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::RpcError;

#[derive(Deserialize, PartialEq)]
pub(super) struct Binding {
    name: String,
    team: String,
}

impl Binding {
    pub(super) fn check(&self, params: &Value) -> Result<()> {
        if params["name"] == "register_agent"
            && (params["arguments"]["name"] != self.name
                || params["arguments"]["team"] != self.team)
        {
            return Err(RpcError::invalid(
                "Antigravity identity is already bound to another name or team",
            )
            .into());
        }
        Ok(())
    }

    pub(super) fn observe(known: &mut Option<Self>, params: &Value, reply: &Value) -> Result<()> {
        let register = params["name"] == "register_agent";
        if (!register && params["name"] != "reconnect") || reply["isError"] == true {
            return Ok(());
        }
        let raw = reply["content"][0]["text"]
            .as_str()
            .context("Missing xats identity result")?;
        let body: Value = serde_json::from_str(raw).context("Invalid xats identity result")?;
        if body.get("error").is_some()
            || body["ambiguous"] == true
            || (!register && body["need_register"] == true)
        {
            return Ok(());
        }
        ensure!(
            body["agent_id"].as_str().is_some_and(|s| !s.is_empty()),
            "Missing agent_id in xats identity result"
        );
        let binding: Self = if register {
            ensure!(
                body["team"] == params["arguments"]["team"]
                    && body
                        .get("name")
                        .map_or(true, |name| name == &params["arguments"]["name"]),
                "xats registered an unexpected name or team"
            );
            serde_json::from_value(params["arguments"].clone())?
        } else {
            ensure!(body["ok"] == true, "Unexpected xats reconnect result");
            serde_json::from_value(body).context("Missing name or team in xats reconnect result")?
        };
        ensure!(
            !binding.name.trim().is_empty() && !binding.team.trim().is_empty(),
            "Empty name or team in xats identity result"
        );
        ensure!(
            known.as_ref().map_or(true, |old| old == &binding),
            "xats identity changed during this bridge session"
        );
        *known = Some(binding);
        Ok(())
    }
}
