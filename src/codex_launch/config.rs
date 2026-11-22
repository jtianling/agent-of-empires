use anyhow::{ensure, Context, Result};
use serde_json::{Map, Value};

pub(super) fn expand_arguments(words: &[String]) -> Vec<String> {
    let mut expanded = Vec::new();
    let mut literal = false;
    for word in words {
        if word == "--" {
            literal = true;
        }
        if literal {
            expanded.push(word.clone());
            continue;
        }
        if let Some((option, value)) = word.split_once('=') {
            if [
                "--config",
                "--model",
                "--remote",
                "--cd",
                "--enable",
                "--disable",
                "--profile",
                "--image",
                "--add-dir",
                "--remote-auth-token-env",
                "--sandbox",
                "--ask-for-approval",
                "--local-provider",
            ]
            .contains(&option)
            {
                expanded.extend([option.to_string(), value.to_string()]);
                continue;
            }
        }
        if word.starts_with('-')
            && !word.starts_with("--")
            && word.len() > 2
            && word
                .get(..2)
                .is_some_and(|prefix| ["-c", "-m", "-C", "-p", "-i", "-a", "-s"].contains(&prefix))
        {
            expanded.extend([word[..2].to_string(), word[2..].to_string()]);
        } else {
            expanded.push(word.clone());
        }
    }
    expanded
}

pub(super) fn profile(name: &str, endpoint: &str) -> Result<Map<String, Value>> {
    ensure!(
        name.as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')),
        "Invalid Codex profile name"
    );
    let endpoint = url::Url::parse(endpoint)?;
    ensure!(
        matches!(
            endpoint.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        ),
        "Local Codex profiles require a local app-server"
    );
    let home = match std::env::var_os("CODEX_HOME") {
        Some(path) if !path.is_empty() => std::path::PathBuf::from(path),
        Some(_) => anyhow::bail!("CODEX_HOME must not be empty"),
        None => dirs::home_dir()
            .context("Missing Codex home")?
            .join(".codex"),
    };
    let path = home.join(format!("{name}.config.toml"));
    let config: toml::Value =
        toml::from_str(&std::fs::read_to_string(path).context("Reading Codex launch profile")?)
            .map_err(|error: toml::de::Error| {
                anyhow::anyhow!(
                    "Invalid Codex launch profile: {} at {:?}",
                    error.message(),
                    error.span()
                )
            })?;
    let mut values = Map::new();
    flatten("", &config, &mut values)?;
    Ok(values)
}

fn flatten(prefix: &str, value: &toml::Value, output: &mut Map<String, Value>) -> Result<()> {
    if let toml::Value::Table(values) = value {
        for (key, value) in values {
            let key = if key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                key.clone()
            } else {
                serde_json::to_string(key)?
            };
            let path = if prefix.is_empty() {
                key
            } else {
                format!("{prefix}.{key}")
            };
            flatten(&path, value, output)?;
        }
    } else {
        output.insert(prefix.into(), serde_json::to_value(value)?);
    }
    Ok(())
}
