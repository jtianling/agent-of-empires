use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Map, Value};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Intent {
    Fresh,
    Resume(String),
    Fork(String),
}

pub(super) struct Command {
    pub program: String,
    pub arguments: Vec<String>,
    pub intent: Intent,
    pub config: Map<String, Value>,
    pub model: Option<String>,
    pub approval: Option<String>,
    pub sandbox: Option<String>,
    pub reviewer: Option<String>,
    pub profile: Option<String>,
}

impl Command {
    pub fn parse(words: &[String], endpoint: &str, cwd: &std::path::Path) -> Result<Self> {
        let words = super::config::expand_arguments(words);
        ensure!(
            words
                .first()
                .and_then(|word| std::path::Path::new(word).file_name())
                .is_some_and(|name| name == "codex"),
            "Expected a Codex executable"
        );
        let mut command = Self {
            program: words.first().context("Missing Codex executable")?.clone(),
            arguments: Vec::new(),
            intent: Intent::Fresh,
            config: Map::new(),
            model: None,
            approval: None,
            sandbox: None,
            reviewer: None,
            profile: None,
        };
        let mut index = 1;
        while index < words.len() {
            index = command.argument(&words, index, endpoint, cwd)?;
        }
        if let Some(profile) = &command.profile {
            let mut values = super::config::profile(profile, endpoint)?;
            values.extend(command.config);
            command.config = values;
        }
        Ok(command)
    }

    fn argument(
        &mut self,
        words: &[String],
        index: usize,
        endpoint: &str,
        cwd: &std::path::Path,
    ) -> Result<usize> {
        let word = &words[index];
        if self.permission_option(word)? {
            return Ok(index + 1);
        }
        match word.as_str() {
            "resume" | "fork" => {
                self.select_thread(word, value(words, index)?)?;
                return Ok(index + 2);
            }
            "--search" => {
                self.config.insert("web_search".into(), json!("live"));
            }
            "--no-alt-screen" | "--strict-config" | "--no-daemon" => {}
            "--remote-auth-token-env" => bail!(
                "AoE Codex resume does not support app-server auth tokens; launch was canceled"
            ),
            "-c" | "--config" | "-m" | "--model" | "--enable" | "--disable" | "--remote" | "-C"
            | "--cd" | "-p" | "--profile" | "-i" | "--image" | "--add-dir" => {
                self.option(word, value(words, index)?, endpoint, cwd)?;
                self.arguments.extend_from_slice(&words[index..index + 2]);
                return Ok(index + 2);
            }
            "--" => {
                self.arguments.extend_from_slice(&words[index..]);
                return Ok(words.len());
            }
            word if !word.starts_with('-') => {
                self.arguments.push(word.into());
                return Ok(index + 1);
            }
            _ => bail!(
                "Unsupported Codex launch option: {}",
                word.split('=').next().unwrap()
            ),
        }
        self.arguments.push(word.clone());
        Ok(index + 1)
    }

    fn select_thread(&mut self, selector: &str, id: &str) -> Result<()> {
        ensure!(
            self.intent == Intent::Fresh,
            "Multiple Codex session selectors"
        );
        uuid::Uuid::parse_str(id).context("Codex launch requires an exact thread UUID")?;
        self.intent = if selector == "resume" {
            Intent::Resume(id.into())
        } else {
            Intent::Fork(id.into())
        };
        Ok(())
    }

    fn permission_option(&mut self, option: &str) -> Result<bool> {
        match option {
            "--dangerously-bypass-approvals-and-sandbox" | "--yolo" => {
                self.permission("never", "danger-full-access", None)?
            }
            "--approve-for-me" => {
                self.permission("on-request", "workspace-write", Some("auto_review"))?
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn option(
        &mut self,
        option: &str,
        value: &str,
        endpoint: &str,
        cwd: &std::path::Path,
    ) -> Result<()> {
        match option {
            "-c" | "--config" => {
                let (key, raw) = value
                    .split_once('=')
                    .context("Invalid Codex config override")?;
                ensure!(
                    !key.is_empty() && !key.chars().any(char::is_control),
                    "Invalid Codex config key"
                );
                let parsed = toml::from_str::<toml::Value>(&format!("value={raw}"))
                    .ok()
                    .and_then(|value| value.get("value").cloned())
                    .map(serde_json::to_value)
                    .transpose()?
                    .unwrap_or_else(|| json!(raw));
                self.config.insert(key.into(), parsed);
            }
            "-m" | "--model" => self.model = Some(value.into()),
            "-p" | "--profile" => self.profile = Some(value.into()),
            "--add-dir" => self.add_directory(value)?,
            "-i" | "--image" => {}
            "--enable" | "--disable" => {
                ensure!(!value.is_empty() && value.bytes().all(|byte|
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
                    "Invalid Codex feature name");
                self.config
                    .insert(format!("features.{value}"), json!(option == "--enable"));
            }
            "--remote" => ensure!(
                url::Url::parse(value)? == url::Url::parse(endpoint)?,
                "Codex launch endpoint changed"
            ),
            "-C" | "--cd" => ensure!(
                std::fs::canonicalize(value)? == cwd,
                "Codex launch directory changed"
            ),
            _ => unreachable!(),
        }
        Ok(())
    }

    fn add_directory(&mut self, value: &str) -> Result<()> {
        let path = std::fs::canonicalize(value).context("Resolving Codex additional directory")?;
        self.config
            .entry("sandbox_workspace_write.writable_roots".to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .context("Invalid Codex writable roots")?
            .push(json!(path));
        Ok(())
    }

    fn permission(&mut self, approval: &str, sandbox: &str, reviewer: Option<&str>) -> Result<()> {
        ensure!(
            self.approval.is_none(),
            "Conflicting Codex permission options"
        );
        self.approval = Some(approval.into());
        self.sandbox = Some(sandbox.into());
        self.reviewer = reviewer.map(str::to_string);
        Ok(())
    }

    pub fn resume_arguments(&self, thread: &str) -> Vec<String> {
        let mut args = vec!["resume".into(), thread.into()];
        let mut index = 0;
        while index < self.arguments.len() {
            if self.arguments[index] == "--" {
                args.extend_from_slice(&self.arguments[index..]);
                break;
            }
            if self.arguments[index] == "--add-dir" && !matches!(self.intent, Intent::Resume(_)) {
                index += 2;
            } else {
                args.push(self.arguments[index].clone());
                index += 1;
            }
        }
        args
    }

    pub fn secrets(&self) -> Vec<String> {
        let mut secrets = Vec::new();
        collect_private(&Value::Object(self.config.clone()), false, &mut secrets);
        secrets
    }
}

fn value(words: &[String], index: usize) -> Result<&str> {
    words
        .get(index + 1)
        .map(String::as_str)
        .context("Missing Codex launch option value")
}

pub(super) fn private_values(words: &[String]) -> Vec<String> {
    let mut secrets = Vec::new();
    for pair in words
        .windows(2)
        .filter(|pair| matches!(pair[0].as_str(), "-c" | "--config"))
    {
        let Some((key, raw)) = pair[1].split_once('=') else {
            continue;
        };
        let parsed = toml::from_str::<toml::Value>(&format!("value={raw}"))
            .ok()
            .and_then(|value| value.get("value").cloned())
            .and_then(|value| serde_json::to_value(value).ok())
            .unwrap_or_else(|| json!(raw));
        collect_private(&parsed, private_key(key), &mut secrets);
    }
    secrets
}

fn private_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "token",
        "password",
        "secret",
        "api_key",
        "authorization",
        "headers",
    ]
    .iter()
    .any(|part| key.contains(part))
}

fn collect_private(value: &Value, private: bool, secrets: &mut Vec<String>) {
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                collect_private(value, private || private_key(key), secrets);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_private(value, private, secrets);
            }
        }
        Value::String(value) if private && !value.is_empty() => {
            secrets.push(value.clone());
            if let Some(token) = value.strip_prefix("Bearer ") {
                secrets.push(token.into());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_remote_launch_moves_permission_flags_to_rpc_and_keeps_model_config() {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path();
        let words = vec![
            "codex".into(),
            "--remote".into(),
            "ws://127.0.0.1:8799".into(),
            "-C".into(),
            cwd.to_str().unwrap().into(),
            "--approve-for-me".into(),
            "--model".into(),
            "gpt-test".into(),
            "-c".into(),
            "service_tier=\"fast\"".into(),
        ];
        let command = Command::parse(&words, "ws://127.0.0.1:8799", cwd).unwrap();
        assert_eq!(command.intent, Intent::Fresh);
        assert_eq!(command.reviewer.as_deref(), Some("auto_review"));
        assert_eq!(command.model.as_deref(), Some("gpt-test"));
        assert_eq!(command.config["service_tier"], "fast");
        assert!(!command
            .resume_arguments("original")
            .contains(&"--approve-for-me".into()));
    }

    #[test]
    fn prompt_does_not_hide_later_flags_and_equals_config_forms() {
        let directory = tempfile::tempdir().unwrap();
        let words = [
            "codex",
            "work on this",
            "--search",
            "--model=custom-model",
            "--config=model_reasoning_effort=\"high\"",
            "--approve-for-me",
        ]
        .map(str::to_string);
        let command = Command::parse(&words, "ws://127.0.0.1:8799", directory.path()).unwrap();
        assert_eq!(command.reviewer.as_deref(), Some("auto_review"));
        assert_eq!(command.model.as_deref(), Some("custom-model"));
        assert_eq!(command.config["model_reasoning_effort"], "high");
        assert_eq!(command.config["web_search"], "live");
        let arguments = command.resume_arguments("original");
        assert!(arguments.contains(&"work on this".to_string()));
        assert!(!arguments.contains(&"--approve-for-me".to_string()));
    }

    #[test]
    fn delimiter_keeps_literal_prompt_and_unicode_is_not_sliced() {
        let directory = tempfile::tempdir().unwrap();
        let words = ["codex", "--", "--model=literal"].map(str::to_string);
        let command = Command::parse(&words, "ws://127.0.0.1:8799", directory.path()).unwrap();
        assert_eq!(command.arguments, ["--", "--model=literal"]);
        assert_eq!(
            command.resume_arguments("original"),
            ["resume", "original", "--", "--model=literal"]
        );
        let literal = Command::parse(
            &["codex".into(), "--".into(), "--add-dir".into()],
            "ws://127.0.0.1:8799",
            directory.path(),
        )
        .unwrap();
        assert_eq!(
            literal.resume_arguments("original"),
            ["resume", "original", "--", "--add-dir"]
        );
        assert!(Command::parse(
            &["codex".into(), "-图".into()],
            "ws://127.0.0.1:8799",
            directory.path()
        )
        .is_err());
    }

    #[test]
    fn unsupported_auth_mode_is_rejected_before_creating_a_thread() {
        let directory = tempfile::tempdir().unwrap();
        let words = ["codex", "--remote-auth-token-env=TOKEN"].map(str::to_string);
        let error = Command::parse(&words, "ws://127.0.0.1:8799", directory.path())
            .err()
            .unwrap();
        assert!(error
            .to_string()
            .contains("does not support app-server auth tokens"));
    }
}
