use std::collections::HashSet;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};

use super::{tools, validate_key, Upstream, VERSIONS};
use crate::xats_control::{block_on_control, discover_control_plane};

pub(crate) fn check_named_support(key: &str) -> Result<()> {
    validate_key(key)?;
    let control = discover_control_plane()?;
    let key = key.to_owned();
    block_on_control("antigravity-launch-probe", move || {
        Box::pin(async move {
            let mut up = Upstream::new(control, key)?;
            let probe = async {
                up.handshake(VERSIONS[0]).await?;
                check_schema(&mut up).await.map(|_| ())
            };
            let result = tokio::time::timeout(Duration::from_secs(10), probe)
                .await
                .context("Antigravity daemon capability probe timed out")
                .and_then(|result| result);
            up.finish_session(result).await
        })
    })
}

pub(super) async fn check_schema(up: &mut Upstream) -> Result<Vec<Value>> {
    let mut seen = HashSet::new();
    let mut params = json!({});
    let mut has_named_register = false;
    let mut discovered = Vec::new();
    loop {
        let page = up.rpc("tools/list", &params).await?;
        let items = page["tools"]
            .as_array()
            .context("Invalid xats tools/list")?;
        for tool in items {
            let Some(name) = tool["name"].as_str() else {
                continue;
            };
            if name == "register_agent" {
                let fields = &tool["inputSchema"]["properties"];
                ensure!(
                    fields["agent_type"]["enum"]
                        .as_array()
                        .is_some_and(|types| types.iter().any(|value| value == "antigravity"))
                        && fields["agy_launch_id"]["type"] == "string",
                    "Upgrade the local xats daemon: register_agent lacks named Antigravity support"
                );
                has_named_register = true;
            } else if tools::communication(name) {
                discovered.push(tool.clone());
            }
        }
        let Some(cursor) = page.get("nextCursor") else {
            break;
        };
        let cursor = cursor
            .as_str()
            .filter(|c| !c.is_empty())
            .context("Invalid xats tools/list cursor")?;
        ensure!(
            seen.insert(cursor.to_owned()),
            "Repeated xats tools/list cursor"
        );
        ensure!(seen.len() <= 128, "Too many xats tools/list pages");
        params = json!({"cursor": cursor});
    }
    if !has_named_register {
        bail!("The local xats daemon does not advertise register_agent");
    }
    Ok(discovered)
}
