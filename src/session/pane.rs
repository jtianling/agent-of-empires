use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use super::{WorkspaceInfo, WorktreeInfo};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneWorktreeRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default = "default_true")]
    pub create_new_branch: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_repo_paths: Vec<String>,
    #[serde(default)]
    pub reuse_existing: bool,
}

fn default_true() -> bool {
    true
}

impl PaneWorktreeRequest {
    pub fn is_requested(&self) -> bool {
        self.branch
            .as_ref()
            .is_some_and(|branch| !branch.trim().is_empty())
    }

    pub fn normalized(mut self) -> Self {
        self.branch = self
            .branch
            .map(|branch| branch.trim().to_string())
            .filter(|branch| !branch.is_empty());
        self.extra_repo_paths = self
            .extra_repo_paths
            .into_iter()
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect();
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneDraft {
    pub tool: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub yolo_mode: bool,
    #[serde(default)]
    pub cross_agent_team: bool,
    /// User-declared xats team, empty when undeclared.
    #[serde(default)]
    pub xats_team: String,
    /// User-declared xats agent name, empty when undeclared.
    #[serde(default)]
    pub xats_agent_name: String,
    #[serde(default)]
    pub worktree: PaneWorktreeRequest,
}

impl PaneDraft {
    pub fn working_dir<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.path.trim().is_empty() {
            fallback
        } else {
            self.path.trim()
        }
    }

    pub fn validate(&self, path_fallback: Option<&str>) -> Result<PathBuf> {
        if crate::agents::get_agent(&self.tool).is_none() {
            bail!("Unknown pane tool '{}'", self.tool);
        }
        let path = if self.path.trim().is_empty() {
            path_fallback.ok_or_else(|| anyhow::anyhow!("Pane path cannot be empty"))?
        } else {
            self.path.trim()
        };
        let path = PathBuf::from(path);
        if !path.exists() {
            bail!("Pane path does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("Pane path is not a directory: {}", path.display());
        }
        Ok(path)
    }

    pub fn cross_agent_team_enabled(&self) -> bool {
        self.cross_agent_team
            && crate::session::Instance::supports_cross_agent_team_tool(&self.tool)
    }

    pub fn yolo_mode_enabled(&self) -> bool {
        crate::agents::get_agent(&self.tool)
            .and_then(|agent| agent.yolo.as_ref())
            .is_some_and(|mode| {
                self.yolo_mode || matches!(mode, crate::agents::YoloMode::AlwaysYolo)
            })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneWorktreeInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceInfo>,
}

impl PaneWorktreeInfo {
    pub fn is_empty(&self) -> bool {
        self.worktree.is_none() && self.workspace.is_none()
    }

    pub fn worktree_path(&self) -> Option<&Path> {
        self.worktree_path.as_deref().map(Path::new)
    }
}

/// Longest xats team or agent name AoE will store for a pane. The values are
/// role names typed by a user, not payloads; the bound exists so a stray paste
/// cannot reach a launch command line.
pub const MAX_DECLARED_XATS_IDENTITY_LEN: usize = 128;

/// Characters xats reads as addressing syntax rather than as part of a name:
/// `:` separates a name from a device, and parentheses are how a team is written
/// beside a name. A declaration containing them is refused by the daemon.
const XATS_NAME_RESERVED: [char; 3] = [':', '(', ')'];
const XATS_TEAM_RESERVED: [char; 2] = ['(', ')'];

/// Whether a declared xats agent name is safe to store and carry.
///
/// Two different reasons to refuse, and they are worth telling apart. AoE's own
/// reason is that the value travels through command lines and log lines, so
/// control characters and unbounded length are out. The daemon's reason is that
/// it reads `:` and `()` as addressing syntax -- refusing those here is not AoE
/// interpreting what a name MEANS, it is AoE refusing to store something the
/// channel cannot carry.
///
/// Checking the daemon's rule at entry is what keeps a rejected declaration from
/// degrading into a silent success: the Codex bootstrap's retry cannot tell "you
/// used a flag I do not know" from "you used a flag correctly but its value is
/// invalid", so it would drop the declaration and launch a healthy-looking pane
/// that is quietly nameless. The fix is to make the second case unreachable.
pub fn is_valid_declared_xats_agent_name(value: &str) -> bool {
    is_storable_declared_value(value) && !value.contains(XATS_NAME_RESERVED)
}

/// Whether a declared xats team is safe to store and carry. Teams reserve fewer
/// characters than names do: a team is never the left side of `name:device`.
pub fn is_valid_declared_xats_team(value: &str) -> bool {
    is_storable_declared_value(value) && !value.contains(XATS_TEAM_RESERVED)
}

fn is_storable_declared_value(value: &str) -> bool {
    value.len() <= MAX_DECLARED_XATS_IDENTITY_LEN && !value.chars().any(char::is_control)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneConfig {
    pub tool: String,
    pub working_dir: String,
    #[serde(default)]
    pub yolo_mode: bool,
    #[serde(default)]
    pub cross_agent_team: bool,
    /// User-declared xats team for this pane, empty when undeclared.
    ///
    /// Opaque to AoE exactly as the identity key is, but unlike the key it is a
    /// public role name rather than a credential: it may appear on argv and in
    /// logs, which is what makes it usable as the Codex pre-registration channel.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub xats_team: String,
    /// User-declared xats agent name for this pane, empty when undeclared.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub xats_agent_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PaneWorktreeInfo>,
}

impl PaneConfig {
    pub(crate) fn is_safe_tool_name(tool: &str) -> bool {
        !tool.is_empty()
            && tool
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    }

    pub fn new(
        tool: impl Into<String>,
        working_dir: impl Into<String>,
        yolo_mode: bool,
        cross_agent_team: bool,
    ) -> Self {
        let tool = tool.into();
        let yolo_mode = crate::agents::get_agent(&tool)
            .and_then(|agent| agent.yolo.as_ref())
            .is_some_and(|mode| yolo_mode || matches!(mode, crate::agents::YoloMode::AlwaysYolo));
        Self {
            cross_agent_team: cross_agent_team
                && crate::session::Instance::supports_cross_agent_team_tool(&tool),
            tool,
            working_dir: working_dir.into(),
            yolo_mode,
            xats_team: String::new(),
            xats_agent_name: String::new(),
            worktree: None,
        }
    }

    pub fn normalized(mut self) -> Self {
        let normalized = Self::new(
            self.tool.clone(),
            self.working_dir.clone(),
            self.yolo_mode,
            self.cross_agent_team,
        );
        self.yolo_mode = normalized.yolo_mode;
        self.cross_agent_team = normalized.cross_agent_team;
        self.xats_team = self.xats_team.trim().to_string();
        self.xats_agent_name = self.xats_agent_name.trim().to_string();
        self
    }

    /// Whether this pane carries a declared xats identity to pass on.
    ///
    /// A declaration only travels with a pane that actually talks to xats, so a
    /// value left behind on a pane whose Cross Agent Team is off is inert rather
    /// than erased: turning the feature back on restores what was declared.
    pub fn declares_xats_identity(&self) -> bool {
        self.cross_agent_team
            && crate::session::Instance::supports_cross_agent_team_tool(&self.tool)
            && !(self.xats_team.is_empty() && self.xats_agent_name.is_empty())
    }

    pub fn validate(&self) -> Result<()> {
        let agent = crate::agents::get_agent(&self.tool);
        if agent.is_none() && !Self::is_safe_tool_name(&self.tool) {
            bail!("Unsafe pane tool '{}'", self.tool);
        }
        if self.working_dir.trim().is_empty() {
            bail!("Pane working directory cannot be empty");
        }
        if self.yolo_mode
            && agent
                .and_then(|definition| definition.yolo.as_ref())
                .is_none()
        {
            bail!("Pane tool '{}' does not support YOLO mode", self.tool);
        }
        if self.cross_agent_team
            && !crate::session::Instance::supports_cross_agent_team_tool(&self.tool)
        {
            bail!(
                "Pane tool '{}' does not support Cross Agent Team",
                self.tool
            );
        }
        for (label, value, valid, reserved) in [
            (
                "team",
                &self.xats_team,
                is_valid_declared_xats_team as fn(&str) -> bool,
                "( )",
            ),
            (
                "agent name",
                &self.xats_agent_name,
                is_valid_declared_xats_agent_name as fn(&str) -> bool,
                ": ( )",
            ),
        ] {
            if !valid(value) {
                bail!(
                    "Declared xats {} must be at most {} characters, free of control \
                     characters, and free of {} (xats reads those as addressing syntax)",
                    label,
                    MAX_DECLARED_XATS_IDENTITY_LEN,
                    reserved
                );
            }
        }
        if let Some(worktree) = &self.worktree {
            if worktree.is_empty() {
                bail!("Pane worktree metadata cannot be empty");
            }
            match (&worktree.worktree, worktree.worktree_path.as_deref()) {
                (Some(_), Some(path)) if path.trim().is_empty() => {
                    bail!("Pane worktree path cannot be empty");
                }
                (Some(_), None) => bail!("Pane worktree path is missing"),
                (None, Some(_)) => bail!("Pane worktree path has no worktree metadata"),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn worktree_info(&self) -> Option<&WorktreeInfo> {
        self.worktree
            .as_ref()
            .and_then(|info| info.worktree.as_ref())
    }

    pub fn workspace_info(&self) -> Option<&WorkspaceInfo> {
        self.worktree
            .as_ref()
            .and_then(|info| info.workspace.as_ref())
    }

    pub fn working_dir_path(&self) -> &Path {
        Path::new(&self.working_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_empty_path_uses_primary_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let draft = PaneDraft {
            tool: "shell".to_string(),
            ..Default::default()
        };

        assert_eq!(draft.validate(temp.path().to_str()).unwrap(), temp.path());
    }

    #[test]
    fn pane_validation_rejects_unknown_tools_and_non_directories() {
        let temp = tempfile::tempdir().unwrap();
        let unknown = PaneDraft {
            tool: "unknown-pane-tool".to_string(),
            path: temp.path().to_string_lossy().to_string(),
            ..Default::default()
        };
        assert!(unknown.validate(None).is_err());

        let file = temp.path().join("file");
        std::fs::write(&file, "not a directory").unwrap();
        let file_draft = PaneDraft {
            tool: "shell".to_string(),
            path: file.to_string_lossy().to_string(),
            ..Default::default()
        };
        assert!(file_draft.validate(None).is_err());
    }

    #[test]
    fn persisted_tool_names_allow_safe_unknown_binaries_but_reject_shell_syntax() {
        assert!(PaneConfig::new("mystery", "/tmp", false, false)
            .validate()
            .is_ok());
        assert!(PaneConfig {
            tool: "evil;command".to_string(),
            working_dir: "/tmp".to_string(),
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn unsupported_tools_cannot_enable_cross_agent_team() {
        let pane = PaneConfig::new("shell", "/tmp", false, true);
        assert!(!pane.cross_agent_team);
    }

    #[test]
    fn opencode_can_enable_cross_agent_team_per_pane() {
        let pane = PaneConfig::new("opencode", "/tmp", false, true);
        assert!(pane.cross_agent_team);
    }

    #[test]
    fn unsupported_tools_cannot_retain_hidden_yolo_state() {
        let pane = PaneConfig::new("shell", "/tmp", true, false);
        assert!(!pane.yolo_mode);

        let draft = PaneDraft {
            tool: "shell".to_string(),
            yolo_mode: true,
            ..Default::default()
        };
        assert!(!draft.yolo_mode_enabled());
    }

    #[test]
    fn literal_pane_config_is_normalized_against_its_actual_tool() {
        let pane = PaneConfig {
            tool: "shell".to_string(),
            working_dir: "/tmp".to_string(),
            yolo_mode: true,
            cross_agent_team: true,
            ..Default::default()
        };

        assert!(pane.validate().is_err());
        let normalized = pane.normalized();
        normalized.validate().unwrap();
        assert!(!normalized.yolo_mode);
        assert!(!normalized.cross_agent_team);
    }

    #[test]
    fn declared_xats_identity_survives_a_json_round_trip_with_quotes_and_spaces() {
        let mut pane = PaneConfig::new("claude", "/tmp", false, true);
        pane.xats_team = "team 'one'".to_string();
        pane.xats_agent_name = "a \"coder\" here".to_string();
        pane.validate().unwrap();

        let json = serde_json::to_string(&pane).unwrap();
        let restored: PaneConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, pane);
    }

    #[test]
    fn a_record_without_declared_xats_identity_reads_as_undeclared() {
        let restored: PaneConfig =
            serde_json::from_str(r#"{"tool":"claude","working_dir":"/tmp"}"#).unwrap();

        assert!(restored.xats_team.is_empty());
        assert!(restored.xats_agent_name.is_empty());
        assert!(!restored.declares_xats_identity());
    }

    #[test]
    fn declared_xats_identity_rejects_control_characters_and_overlong_values() {
        let mut newline = PaneConfig::new("claude", "/tmp", false, true);
        newline.xats_team = "team\nname".to_string();
        assert!(newline.validate().is_err());

        let mut control = PaneConfig::new("claude", "/tmp", false, true);
        control.xats_agent_name = "coder\u{7}".to_string();
        assert!(control.validate().is_err());

        let mut long = PaneConfig::new("claude", "/tmp", false, true);
        long.xats_team = "x".repeat(MAX_DECLARED_XATS_IDENTITY_LEN + 1);
        assert!(long.validate().is_err());

        let mut at_limit = PaneConfig::new("claude", "/tmp", false, true);
        at_limit.xats_team = "x".repeat(MAX_DECLARED_XATS_IDENTITY_LEN);
        at_limit.validate().unwrap();
    }

    #[test]
    fn only_one_declared_part_is_still_a_declaration() {
        let mut pane = PaneConfig::new("claude", "/tmp", false, true);
        pane.xats_team = "monkeys".to_string();

        assert!(pane.declares_xats_identity());
        assert!(pane.xats_agent_name.is_empty());
    }

    #[test]
    fn a_pane_without_cross_agent_team_declares_nothing() {
        let mut pane = PaneConfig::new("claude", "/tmp", false, false);
        pane.xats_team = "monkeys".to_string();

        assert!(!pane.declares_xats_identity());
        assert_eq!(pane.xats_team, "monkeys", "the value is kept, not erased");
    }

    #[test]
    fn worktree_cleanup_path_does_not_follow_runtime_working_directory() {
        let pane = PaneConfig {
            tool: "claude".to_string(),
            working_dir: "/tmp/runtime-cwd".to_string(),
            worktree: Some(PaneWorktreeInfo {
                worktree_path: Some("/tmp/owned-worktree".to_string()),
                worktree: Some(WorktreeInfo {
                    branch: "feature".to_string(),
                    main_repo_path: "/tmp/repo".to_string(),
                    managed_by_aoe: true,
                    created_at: chrono::Utc::now(),
                    cleanup_on_delete: true,
                }),
                workspace: None,
            }),
            ..Default::default()
        };

        pane.validate().unwrap();
        assert_eq!(
            pane.worktree
                .as_ref()
                .and_then(PaneWorktreeInfo::worktree_path),
            Some(Path::new("/tmp/owned-worktree"))
        );
        assert_ne!(
            pane.working_dir_path(),
            pane.worktree
                .as_ref()
                .and_then(PaneWorktreeInfo::worktree_path)
                .unwrap()
        );
    }

    #[test]
    fn managed_worktree_metadata_requires_an_immutable_path() {
        let pane = PaneConfig {
            tool: "claude".to_string(),
            working_dir: "/tmp/runtime-cwd".to_string(),
            worktree: Some(PaneWorktreeInfo {
                worktree_path: None,
                worktree: Some(WorktreeInfo {
                    branch: "feature".to_string(),
                    main_repo_path: "/tmp/repo".to_string(),
                    managed_by_aoe: true,
                    created_at: chrono::Utc::now(),
                    cleanup_on_delete: true,
                }),
                workspace: None,
            }),
            ..Default::default()
        };

        assert!(pane.validate().is_err());
    }
}
