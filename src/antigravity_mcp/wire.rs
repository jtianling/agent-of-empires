use anyhow::{bail, ensure, Context, Result};
use reqwest::Response;
use serde_json::Value;

use super::{RpcError, LIMIT};

pub(super) async fn read_reply(mut response: Response, id: u64) -> Result<Value> {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    let sse = content_type.eq_ignore_ascii_case("text/event-stream");
    ensure!(
        sse || content_type.eq_ignore_ascii_case("application/json"),
        "Unexpected xats MCP response content type"
    );
    ensure!(
        response.content_length().unwrap_or(0) <= LIMIT as u64,
        "xats MCP response exceeds size limit"
    );
    let mut body = Vec::new();
    let mut data = Vec::new();
    let mut start = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Reading xats MCP response")?
    {
        ensure!(
            chunk.len() <= LIMIT - body.len(),
            "xats MCP response exceeds size limit"
        );
        let previous = body.len();
        body.extend_from_slice(&chunk);
        if sse {
            for i in previous..body.len() {
                if body[i] == b'\n' {
                    let line = &body[start..i];
                    start = i + 1;
                    if let Some(value) = event_line(line, &mut data)? {
                        return unpack(value, id);
                    }
                }
            }
        }
    }
    ensure!(!sse, "xats MCP stream ended without a reply");
    unpack(
        serde_json::from_slice(&body).context("Invalid xats MCP JSON")?,
        id,
    )
}

fn event_line(line: &[u8], data: &mut Vec<u8>) -> Result<Option<Value>> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if let Some(value) = line.strip_prefix(b"data:") {
        data.extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
        data.push(b'\n');
    } else if line.is_empty() {
        if !data.iter().all(u8::is_ascii_whitespace) {
            let value: Value = serde_json::from_slice(data).context("Invalid xats MCP SSE data")?;
            data.clear();
            // Server requests and notifications are outside the pull-only contract.
            if value.get("method").is_none() {
                return Ok(Some(value));
            }
        }
        data.clear();
    }
    Ok(None)
}

fn unpack(value: Value, id: u64) -> Result<Value> {
    ensure!(
        value["jsonrpc"] == "2.0"
            && value["id"].as_u64() == Some(id)
            && value.get("method").is_none(),
        "Unexpected xats MCP response id or envelope"
    );
    match (value.get("error"), value.get("result")) {
        (Some(err), None) => {
            let code = err["code"]
                .as_i64()
                .context("Missing xats MCP error code")?;
            let message = err["message"]
                .as_str()
                .context("Missing xats MCP error message")?;
            Err(RpcError::new(code, message).into())
        }
        (None, Some(result)) if result.is_object() => Ok(result.clone()),
        _ => bail!("Invalid xats MCP result/error envelope"),
    }
}
