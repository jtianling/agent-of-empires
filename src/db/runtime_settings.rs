use anyhow::{ensure, Context, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::{AgentSlot, Store};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeSettings {
    pub model: String,
    pub effort: Option<String>,
    pub service_tier: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSettings {
    agent: String,
    identity_key: String,
    settings: RuntimeSettings,
}

impl RuntimeSettings {
    pub fn verify_codex_response(&self, response: &serde_json::Value) -> Result<()> {
        ensure!(
            response.get("model").and_then(|v| v.as_str()) == Some(&self.model),
            "Codex did not restore the pane model"
        );
        let effort_value = optional_string(response, "reasoningEffort")?;
        if let Some(effort) = &self.effort {
            ensure!(
                effort_value == Some(effort),
                "Codex did not restore the pane reasoning effort"
            );
        }
        let tier = optional_string(response, "serviceTier")?.unwrap_or("default");
        ensure!(
            tier == self.service_tier.as_deref().unwrap_or("default"),
            "Codex did not restore the pane service tier"
        );
        Ok(())
    }
    pub fn codex_config(&self) -> serde_json::Value {
        let mut config = serde_json::json!({
            "service_tier": self.service_tier.as_deref().unwrap_or("default")
        });
        if let Some(effort) = &self.effort {
            config["model_reasoning_effort"] = serde_json::json!(effort);
        }
        config
    }
    pub fn validate(&self, agent: &str) -> Result<()> {
        ensure!(
            super::claude_transcript::is_safe_model_id(&self.model),
            "Invalid pane model"
        );
        ensure!(agent == "codex", "Unsupported pane settings agent: {agent}");
        let valid_value = |value: &str| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        };
        ensure!(
            self.effort.as_deref().map_or(true, valid_value),
            "Invalid {agent} reasoning effort"
        );
        ensure!(
            self.service_tier.as_deref().map_or(true, valid_value),
            "Invalid {agent} service tier"
        );
        Ok(())
    }

    pub fn append_command(&self, command: String, agent: &str) -> Result<String> {
        use crate::session::environment::shell_escape;
        self.validate(agent)?;
        let args = shell_words::split(&command).context("Invalid Codex launch command")?;
        let separator = args.iter().position(|arg| arg == "--");
        let tail = separator.map(|index| shell_words::join(&args[index + 1..]));
        let command = separator.map_or(command, |index| shell_words::join(&args[..index]));
        let mut command = format!("{command} --model {}", shell_escape(&self.model));
        if let Some(effort) = &self.effort {
            command = format!(
                "{command} -c {}",
                shell_escape(&format!("model_reasoning_effort=\"{effort}\""))
            );
        }
        if agent == "codex" {
            let tier = self.service_tier.as_deref().unwrap_or("default");
            command = format!(
                "{command} -c {}",
                shell_escape(&format!("service_tier=\"{tier}\""))
            );
        }
        if let Some(tail) = tail {
            command = format!("{command} -- {tail}");
        }
        Ok(command)
    }

    pub fn launch_args(args: &str) -> Result<String> {
        let args = shell_words::split(args).context("Invalid Codex launch arguments")?;
        let mut args = args.into_iter();
        let mut kept = Vec::new();
        while let Some(arg) = args.next() {
            if arg == "--" {
                kept.push(arg);
                kept.extend(args);
                break;
            }
            if matches!(arg.as_str(), "--model" | "-m") {
                args.next().context("Codex model argument has no value")?;
            } else if !(arg.starts_with("--model=")
                || arg.starts_with("-m") && !arg.starts_with("--"))
            {
                kept.push(arg);
            }
        }
        Ok(shell_words::join(kept))
    }
}

fn optional_string<'a>(response: &'a serde_json::Value, field: &str) -> Result<Option<&'a str>> {
    match response.get(field) {
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value)),
        _ => anyhow::bail!("Codex returned an invalid {field}"),
    }
}

impl Store {
    pub(crate) fn save_runtime_settings(
        &self,
        slot: &AgentSlot,
        settings: &RuntimeSettings,
    ) -> Result<()> {
        settings.validate(&slot.agent)?;
        let changed = self.conn.execute(
            "UPDATE agent_slot SET runtime_settings = ?1 WHERE instance_id = ?2 \
             AND slot = ?3 AND agent = ?4 AND native_session_id = ?5 \
             AND xats_runtime_generation = ?6 AND xats_identity_key = ?7",
            rusqlite::params![
                serde_json::to_string(&StoredSettings {
                    agent: slot.agent.clone(),
                    identity_key: slot.xats_identity_key.clone(),
                    settings: settings.clone(),
                })?,
                slot.instance_id,
                slot.slot,
                slot.agent,
                slot.native_session_id,
                slot.xats_runtime_generation,
                slot.xats_identity_key
            ],
        )?;
        ensure!(
            changed == 1,
            "Pane changed while capturing runtime settings"
        );
        Ok(())
    }

    pub(crate) fn runtime_settings(&self, slot: &AgentSlot) -> Result<Option<RuntimeSettings>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT runtime_settings FROM agent_slot WHERE instance_id = ?1 \
             AND slot = ?2 AND agent = ?3 AND xats_identity_key = ?4",
                rusqlite::params![
                    slot.instance_id,
                    slot.slot,
                    slot.agent,
                    slot.xats_identity_key
                ],
                |row| row.get(0),
            )
            .optional()?;
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let stored: StoredSettings =
            serde_json::from_str(&value).context("Invalid stored pane runtime settings")?;
        if stored.agent != slot.agent || stored.identity_key != slot.xats_identity_key {
            return Ok(None);
        }
        let settings = stored.settings;
        settings.validate(&slot.agent)?;
        Ok(Some(settings))
    }
}
