//! New session dialog

mod group_input;
mod layout;
mod path_input;
mod render;

#[cfg(test)]
mod tests;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use super::DialogResult;
use layout::ABSENT;

use crate::containers::{self, ContainerRuntimeInterface};
use crate::session::config::SandboxConfig;
use crate::session::repo_config::HookProgress;
#[cfg(test)]
use crate::session::Config;
use crate::session::{civilizations, resolve_config};
use crate::tmux::AvailableTools;
use crate::tui::components::{
    DirPicker, DirPickerResult, GroupGhostCompletion, ListPicker, ListPickerResult, PathField,
    PathGhostCompletion,
};

/// What makes a help entry visible. Naming the condition on the entry keeps the
/// help overlay from re-deriving it as a positional index, which silently
/// mismatches the moment an entry is inserted anywhere but the end.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum HelpVisibility {
    Always,
    /// Hidden when the profile offers a single tool, which is not selectable.
    ToolSelection,
    RightPanePath,
    Yolo,
    CrossAgentTeam,
    Sandbox,
    /// The sandbox sub-options, shown only while sandboxing is enabled.
    SandboxOptions,
    /// Skipped outright: the profile field is no longer part of the dialog.
    Never,
}

pub(super) struct FieldHelp {
    pub(super) name: &'static str,
    pub(super) description: &'static str,
    pub(super) visibility: HelpVisibility,
}

pub(super) const HELP_DIALOG_WIDTH: u16 = 85;

pub(super) const FIELD_HELP: &[FieldHelp] = &[
    FieldHelp {
        name: "Profile",
        description: "Settings profile for session defaults (Left/Right to cycle)",
        visibility: HelpVisibility::Never,
    },
    FieldHelp {
        name: "Title",
        description: "Session name (auto-generates if empty)",
        visibility: HelpVisibility::Always,
    },
    FieldHelp {
        name: "Path",
        description: "Working directory for the session",
        visibility: HelpVisibility::Always,
    },
    FieldHelp {
        name: "Tool",
        description: "Which AI tool to use (Ctrl+P to configure command and extra args)",
        visibility: HelpVisibility::ToolSelection,
    },
    FieldHelp {
        name: "Right Pane",
        description: "Optional tool for an auto-created right pane (Left/Right to cycle)",
        visibility: HelpVisibility::Always,
    },
    FieldHelp {
        name: "Right Pane Path",
        description: "Working directory for the right pane (empty = same as the session)",
        visibility: HelpVisibility::RightPanePath,
    },
    FieldHelp {
        name: "YOLO Mode",
        description:
            "Skip permission prompts for autonomous operation (--dangerously-skip-permissions)",
        visibility: HelpVisibility::Yolo,
    },
    FieldHelp {
        name: "Cross Agent Teams",
        description: "Launch the selected tool with its local xats integration",
        visibility: HelpVisibility::CrossAgentTeam,
    },
    FieldHelp {
        name: "Worktree",
        description:
            "Branch name for git worktree (Ctrl+P to configure branch mode and extra repos)",
        visibility: HelpVisibility::Always,
    },
    FieldHelp {
        name: "Sandbox",
        description: "Run session in Docker container for isolation (Ctrl+P to configure)",
        visibility: HelpVisibility::Sandbox,
    },
    FieldHelp {
        name: "Image",
        description: "Docker image. Edit config.toml [sandbox] default_image to change default",
        visibility: HelpVisibility::SandboxOptions,
    },
    FieldHelp {
        name: "Environment",
        description: "Env vars: bare KEY passes host value, KEY=VALUE sets explicitly",
        visibility: HelpVisibility::SandboxOptions,
    },
    FieldHelp {
        name: "Group",
        description: "Optional grouping for organization (Ctrl+P to browse existing groups)",
        visibility: HelpVisibility::Always,
    },
];

#[derive(Clone)]
pub struct NewSessionData {
    pub profile: String,
    pub title: String,
    pub path: String,
    pub group: String,
    pub tool: String,
    pub worktree_branch: Option<String>,
    pub create_new_branch: bool,
    pub extra_repo_paths: Vec<String>,
    pub sandbox: bool,
    /// The sandbox image to use (always populated from the input field).
    pub sandbox_image: String,
    pub yolo_mode: bool,
    /// Whether to launch in Cross Agent Team mode for a supported non-sandboxed tool.
    pub cross_agent_team: bool,
    /// Claude development-channels string for Cross Agent Team launches.
    pub cross_agent_team_channel: String,
    /// Additional environment entries for the container.
    /// `KEY` = pass through from host, `KEY=VALUE` = set explicitly.
    pub extra_env: Vec<String>,
    /// Extra arguments to append after the agent binary
    pub extra_args: String,
    /// Command override for the agent binary (replaces the default binary)
    pub command_override: String,
    /// Whether to reuse an existing worktree instead of failing
    pub reuse_worktree: bool,
    /// Optional tool to launch in a right pane (auto-split). None or empty = no split.
    pub right_pane_tool: Option<String>,
    /// Working directory for the right pane. `None` means the session's own,
    /// resolved when the pane is split rather than snapshotted here: a
    /// worktree-backed session does not know its directory until it is created.
    pub right_pane_path: Option<String>,
}

/// Spinner frames for loading animation
pub(super) const SPINNER_FRAMES: &[&str] = &["◐", "◓", "◑", "◒"];

/// Which field the shared directory picker was opened for. The picker itself
/// only reports a directory, so the dialog has to remember where to put it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DirPickerTarget {
    SessionPath,
    RightPanePath,
    WorkspaceRepo,
}

/// A pending confirmation to create the directories a submit needs.
///
/// One confirmation covers every missing directory. Prompting per field can
/// leave the first directory created after the user declines the second, which
/// is a state they did not ask for and cannot see.
pub(super) struct CreateDirsConfirm {
    pub(super) dirs: Vec<String>,
    pub(super) yes_selected: bool,
}

/// Create `dir`, appending to `owned` every component this call actually made,
/// outermost first.
///
/// Each level goes through `create_dir`, which is atomic and distinguishes
/// "this call made it" from "it was already there". `create_dir_all` does
/// neither: it reports success for a directory that already existed, so a
/// directory another process created while the confirmation was on screen would
/// be treated as ours, and it creates intermediate parents without naming them,
/// so rolling back only the path that was asked for leaves those parents behind.
///
/// On failure the components made so far stay in `owned`, so the caller can undo
/// exactly them.
fn create_dir_tracked(dir: &str, owned: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    let path = std::path::Path::new(dir);
    if path.components().next().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty path",
        ));
    }

    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component);
        match std::fs::create_dir(&current) {
            Ok(()) => owned.push(current.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // `create_dir` reports AlreadyExists for anything occupying that
                // name, including a regular file and a symlink whose target does
                // not exist. Neither is a directory to launch a pane in, and a
                // dangling symlink reaches here routinely: `Path::exists` follows
                // the link, so it reads as missing and the dialog offers to
                // create it. `metadata` follows the link too, which is what makes
                // it answer the question that matters here.
                let usable = std::fs::metadata(&current).is_ok_and(|m| m.is_dir());
                if !usable {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("{} exists but is not a directory", current.display()),
                    ));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub struct NewSessionDialog {
    pub(super) profile: String,
    pub(super) title: Input,
    pub(super) path: PathField,
    /// Working directory for the right pane; empty means the session's own.
    pub(super) right_pane_path: PathField,
    pub(super) group: Input,
    pub(super) tool_index: usize,
    /// Right pane tool index: 0 = "none", 1+ maps to available_tools[index-1]
    pub(super) right_pane_tool_index: usize,
    pub(super) focused_field: usize,
    pub(super) available_tools: Vec<&'static str>,
    pub(super) existing_titles: Vec<String>,
    pub(super) worktree_branch: Input,
    pub(super) create_new_branch: bool,
    pub(super) sandbox_enabled: bool,
    pub(super) sandbox_image: Input,
    pub(super) docker_available: bool,
    pub(super) yolo_mode: bool,
    pub(super) yolo_mode_default: bool,
    pub(super) cross_agent_team: bool,
    pub(super) cross_agent_team_channel: String,
    /// Additional repo paths for multi-repo workspace
    pub(super) workspace_repos: Vec<String>,
    /// Whether the workspace repos list is expanded (editing mode)
    pub(super) workspace_repos_expanded: bool,
    /// Currently selected index in the workspace repos list
    pub(super) workspace_repo_selected_index: usize,
    /// Input for editing/adding workspace repo entries
    pub(super) workspace_repo_editing_input: Option<Input>,
    /// Whether we are adding a new repo entry (vs editing existing)
    pub(super) workspace_repo_adding_new: bool,
    /// Ghost completion for workspace repo path editing
    pub(super) workspace_repo_ghost: Option<PathGhostCompletion>,
    /// Which field the directory picker will write into when it returns.
    pub(super) dir_picker_target: DirPickerTarget,
    /// Worktree configuration overlay mode (Ctrl+P on worktree field)
    pub(super) worktree_config_mode: bool,
    /// Focused field within the worktree config overlay (0=new_branch, 1=extra_repos)
    pub(super) worktree_config_focused_field: usize,
    /// Extra environment entries (session-specific).
    /// `KEY` = pass through, `KEY=VALUE` = set explicitly.
    pub(super) extra_env: Vec<String>,
    /// Whether the env list is expanded (editing mode)
    pub(super) env_list_expanded: bool,
    /// Currently selected index in the env list
    pub(super) env_selected_index: usize,
    /// Input for editing/adding env entries
    pub(super) env_editing_input: Option<Input>,
    /// Whether we are adding a new entry (vs editing existing)
    pub(super) env_adding_new: bool,
    /// Pre-computed label/value pairs for non-default inherited sandbox settings.
    pub(super) inherited_settings: Vec<(String, String)>,
    pub(super) sandbox_config_mode: bool,
    pub(super) sandbox_focused_field: usize,
    /// Tool configuration mode (Ctrl+P on tool field)
    pub(super) tool_config_mode: bool,
    pub(super) tool_config_focused_field: usize,
    /// Extra args for the selected tool (loaded from config)
    pub(super) extra_args: Input,
    /// Command override for the selected tool (loaded from config)
    pub(super) command_override: Input,
    pub(super) existing_groups: Vec<String>,
    pub(super) group_directories: HashMap<String, String>,
    pub(super) path_user_edited: bool,
    pub(super) group_picker: ListPicker,
    pub(super) branch_picker: ListPicker,
    pub(super) dir_picker: DirPicker,
    pub(super) error_message: Option<String>,
    pub(super) show_help: bool,
    /// Whether the dialog is in loading state (creating session in background)
    pub(super) loading: bool,
    /// Spinner animation frame counter
    pub(super) spinner_frame: usize,
    /// Whether hooks are being executed during loading
    pub(super) has_hooks: bool,
    /// The currently running hook command
    pub(super) current_hook: Option<String>,
    /// Accumulated output lines from hook execution
    pub(super) hook_output: Vec<String>,
    /// Ghost text completion for the group field (fish-shell style).
    group_ghost: Option<GroupGhostCompletion>,
    /// Inline confirmation for creating the directories a submit needs.
    pub(super) confirm_create_dirs: Option<CreateDirsConfirm>,
    /// Whether the user has been warned about reusing an existing worktree.
    /// On first Enter the warning is shown; on second Enter the session is created with reuse.
    pub(super) confirm_reuse_worktree: bool,
    /// Saved yolo_mode value before switching to shell, restored on switch back.
    saved_yolo_mode: Option<bool>,
}

/// Shared logic for handling key events in an editable list (env keys or env values).
fn handle_editable_list_key(
    key: KeyEvent,
    items: &mut Vec<String>,
    expanded: &mut bool,
    selected_index: &mut usize,
    editing_input: &mut Option<Input>,
    adding_new: &mut bool,
    validate: impl Fn(&str, &[String]) -> bool,
) -> DialogResult<NewSessionData> {
    // Handle text input mode (editing or adding)
    if let Some(ref mut input) = editing_input {
        match key.code {
            KeyCode::Enter => {
                let value = input.value().trim().to_string();
                if validate(&value, items) {
                    if *adding_new {
                        items.push(value);
                        *selected_index = items.len().saturating_sub(1);
                    } else if *selected_index < items.len() {
                        items[*selected_index] = value;
                    }
                }
                *editing_input = None;
                *adding_new = false;
                return DialogResult::Continue;
            }
            KeyCode::Esc => {
                *editing_input = None;
                *adding_new = false;
                return DialogResult::Continue;
            }
            _ => {
                input.handle_event(&crossterm::event::Event::Key(key));
                return DialogResult::Continue;
            }
        }
    }

    match key.code {
        KeyCode::Esc => {
            *expanded = false;
            DialogResult::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if *selected_index > 0 {
                *selected_index -= 1;
            }
            DialogResult::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if *selected_index < items.len().saturating_sub(1) {
                *selected_index += 1;
            }
            DialogResult::Continue
        }
        KeyCode::Char('a') => {
            *editing_input = Some(Input::default());
            *adding_new = true;
            DialogResult::Continue
        }
        KeyCode::Char('d') => {
            if !items.is_empty() && *selected_index < items.len() {
                items.remove(*selected_index);
                if *selected_index > 0 && *selected_index >= items.len() {
                    *selected_index = items.len().saturating_sub(1);
                }
            }
            DialogResult::Continue
        }
        KeyCode::Enter => {
            if !items.is_empty() && *selected_index < items.len() {
                let current = items[*selected_index].clone();
                *editing_input = Some(Input::new(current));
                *adding_new = false;
            }
            DialogResult::Continue
        }
        _ => DialogResult::Continue,
    }
}

/// Build label/value pairs for non-default inherited sandbox settings.
fn build_inherited_settings(sandbox: &SandboxConfig) -> Vec<(String, String)> {
    let mut settings = Vec::new();
    if sandbox.mount_ssh {
        settings.push(("Mount SSH".to_string(), "yes".to_string()));
    }
    if !sandbox.extra_volumes.is_empty() {
        settings.push((
            "Extra Volumes".to_string(),
            format!("{} items", sandbox.extra_volumes.len()),
        ));
    }
    if !sandbox.volume_ignores.is_empty() {
        settings.push((
            "Volume Ignores".to_string(),
            format!("{} items", sandbox.volume_ignores.len()),
        ));
    }
    if let Some(ref cpu) = sandbox.cpu_limit {
        settings.push(("CPU Limit".to_string(), cpu.clone()));
    }
    if let Some(ref mem) = sandbox.memory_limit {
        settings.push(("Memory Limit".to_string(), mem.clone()));
    }
    settings
}

impl NewSessionDialog {
    pub fn new(
        tools: AvailableTools,
        existing_titles: Vec<String>,
        existing_groups: Vec<String>,
        group_directories: HashMap<String, String>,
        default_group: Option<String>,
        profile: &str,
        launch_dir: &std::path::Path,
    ) -> Self {
        let launch_dir_str = launch_dir.to_string_lossy().to_string();
        let current_dir = default_group
            .as_ref()
            .and_then(|g| group_directories.get(g))
            .cloned()
            .unwrap_or(launch_dir_str);

        let available_tools = tools.available_list();
        let docker_available = containers::get_container_runtime().is_available();

        // Load resolved config (global merged with profile overrides)
        let config = resolve_config(profile).unwrap_or_default();

        // Determine default tool index based on config
        let tool_index = if let Some(ref default_tool) = config.session.default_tool {
            available_tools
                .iter()
                .position(|&t| t == default_tool.as_str())
                .unwrap_or(0)
        } else {
            0
        };

        // Apply sandbox defaults from config
        let sandbox_enabled = docker_available && config.sandbox.enabled_by_default;
        let yolo_mode = config.session.yolo_mode_default;
        let cross_agent_team = config.session.cross_agent_team_default;
        let cross_agent_team_channel = config.session.cross_agent_team_channel.clone();

        // Load extra args and command override for the default tool
        let selected_tool = available_tools
            .get(tool_index)
            .or_else(|| available_tools.first())
            .copied()
            .unwrap_or("claude");
        let extra_args_value = config
            .session
            .agent_extra_args
            .get(selected_tool)
            .cloned()
            .unwrap_or_default();
        let command_override_value = config
            .session
            .agent_command_override
            .get(selected_tool)
            .cloned()
            .unwrap_or_default();

        // Initialize env entries and inherited settings from config when sandbox is enabled
        let (extra_env, inherited_settings) = if sandbox_enabled {
            let inherited = build_inherited_settings(&config.sandbox);
            (config.sandbox.environment.clone(), inherited)
        } else {
            (Vec::new(), Vec::new())
        };

        Self {
            profile: profile.to_string(),
            title: Input::default(),
            path: PathField::new(current_dir),
            right_pane_path: PathField::default(),
            group: Input::new(default_group.unwrap_or_default()),
            tool_index,
            right_pane_tool_index: 0,
            focused_field: 0,
            available_tools,
            existing_titles,
            existing_groups,
            group_directories,
            path_user_edited: false,
            group_picker: ListPicker::new("Select Group"),
            branch_picker: ListPicker::new("Select Branch"),
            dir_picker: DirPicker::new(),
            worktree_branch: Input::default(),
            create_new_branch: true,
            workspace_repos: Vec::new(),
            workspace_repos_expanded: false,
            workspace_repo_selected_index: 0,
            workspace_repo_editing_input: None,
            workspace_repo_adding_new: false,
            workspace_repo_ghost: None,
            dir_picker_target: DirPickerTarget::SessionPath,
            worktree_config_mode: false,
            worktree_config_focused_field: 0,
            sandbox_enabled,
            sandbox_image: Input::new(
                containers::get_container_runtime().effective_default_image(),
            ),
            docker_available,
            yolo_mode,
            yolo_mode_default: yolo_mode,
            cross_agent_team,
            cross_agent_team_channel,
            extra_env,
            env_list_expanded: false,
            env_selected_index: 0,
            env_editing_input: None,
            env_adding_new: false,
            inherited_settings,
            sandbox_config_mode: false,
            sandbox_focused_field: 0,
            tool_config_mode: false,
            tool_config_focused_field: 0,
            extra_args: Input::new(extra_args_value),
            command_override: Input::new(command_override_value),
            error_message: None,
            show_help: false,
            loading: false,
            spinner_frame: 0,
            has_hooks: false,
            current_hook: None,
            hook_output: Vec::new(),
            group_ghost: None,
            confirm_create_dirs: None,
            confirm_reuse_worktree: false,
            saved_yolo_mode: None,
        }
    }

    /// Pre-fill the path field (e.g. from a selected session).
    pub fn set_path(&mut self, path: String) {
        self.path.set_value(path);
    }

    /// Pre-fill the group field (e.g. from a selected session or group).
    pub fn set_group(&mut self, group: String) {
        self.group = Input::new(group);
    }

    #[cfg(test)]
    pub fn path_value(&self) -> &str {
        self.path.value()
    }

    #[cfg(test)]
    pub fn group_value(&self) -> &str {
        self.group.value()
    }

    #[cfg(test)]
    pub fn profile_value(&self) -> &str {
        &self.profile
    }

    /// Set whether hooks will be executed during session creation
    pub fn set_has_hooks(&mut self, has_hooks: bool) {
        self.has_hooks = has_hooks;
    }

    /// Push a hook progress message into the dialog state
    pub fn push_hook_progress(&mut self, progress: HookProgress) {
        match progress {
            HookProgress::Started(cmd) => {
                self.current_hook = Some(cmd);
            }
            HookProgress::Output(line) => {
                self.hook_output.push(line);
            }
        }
    }

    /// Set the dialog to loading state
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.error_message = None;
        }
    }

    /// Check if the dialog is in loading state
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Advance dialog timers (spinner and transient highlights).
    /// Returns true when visual state changed and the UI should redraw.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;

        if self.loading {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
            changed = true;
        }

        if self.path.tick() {
            changed = true;
        }

        if self.right_pane_path.tick() {
            changed = true;
        }

        changed
    }

    pub(super) fn is_terminal_selected(&self) -> bool {
        self.available_tools.get(self.tool_index).copied() == Some("shell")
    }

    /// Whether the currently selected tool is always in YOLO mode (no opt-in needed).
    pub(super) fn selected_tool_always_yolo(&self) -> bool {
        let tool_name = self.available_tools[self.tool_index];
        crate::agents::get_agent(tool_name)
            .and_then(|a| a.yolo.as_ref())
            .is_some_and(|y| matches!(y, crate::agents::YoloMode::AlwaysYolo))
    }

    pub(super) fn right_pane_needs_yolo(&self) -> bool {
        if self.right_pane_tool_index == 0 {
            return false;
        }

        let tool_name = self.available_tools[self.right_pane_tool_index - 1];
        tool_name != "shell"
            && crate::agents::get_agent(tool_name)
                .and_then(|a| a.yolo.as_ref())
                .is_some_and(|y| !matches!(y, crate::agents::YoloMode::AlwaysYolo))
    }

    /// The right pane path field is shown only for a real right pane tool, and
    /// never under sandboxing: there the agent's directory is decided by the
    /// container exec, so a host-side directory would be accepted and ignored.
    pub(super) fn has_right_pane_path_field(&self) -> bool {
        self.right_pane_tool_index != 0 && !self.sandbox_enabled
    }

    /// Whether a help entry applies to the dialog as it currently stands. The
    /// conditions are the same ones the fields themselves are built from.
    pub(super) fn help_entry_visible(&self, visibility: HelpVisibility) -> bool {
        match visibility {
            HelpVisibility::Always => true,
            HelpVisibility::Never => false,
            HelpVisibility::ToolSelection => self.available_tools.len() > 1,
            HelpVisibility::RightPanePath => self.has_right_pane_path_field(),
            HelpVisibility::Yolo => self.has_yolo_field(),
            HelpVisibility::CrossAgentTeam => self.has_cross_agent_team_field(),
            HelpVisibility::Sandbox => self.docker_available,
            HelpVisibility::SandboxOptions => self.docker_available && self.sandbox_enabled,
        }
    }

    pub(super) fn has_yolo_field(&self) -> bool {
        (!self.is_terminal_selected() && !self.selected_tool_always_yolo())
            || self.right_pane_needs_yolo()
    }

    /// Cross Agent Team supports Claude and Codex, and is unavailable in Sandbox.
    pub(super) fn has_cross_agent_team_field(&self) -> bool {
        self.available_tools
            .get(self.tool_index)
            .is_some_and(|&t| crate::session::Instance::supports_cross_agent_team_tool(t))
            && !self.sandbox_enabled
    }

    fn sync_yolo_for_right_pane(&mut self) {
        if self.is_terminal_selected() {
            if self.right_pane_needs_yolo() && !self.yolo_mode {
                self.yolo_mode = self.saved_yolo_mode.unwrap_or(self.yolo_mode_default);
            } else if !self.right_pane_needs_yolo() {
                self.yolo_mode = false;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_config(tools: Vec<&'static str>, path: String, config: Config) -> Self {
        let tool_index = if let Some(ref default_tool) = config.session.default_tool {
            tools
                .iter()
                .position(|&t| t == default_tool.as_str())
                .unwrap_or(0)
        } else {
            0
        };

        Self {
            profile: "default".to_string(),
            title: Input::default(),
            path: PathField::new(path),
            right_pane_path: PathField::default(),
            group: Input::default(),
            tool_index,
            right_pane_tool_index: 0,
            focused_field: 0,
            available_tools: tools,
            existing_titles: Vec::new(),
            existing_groups: Vec::new(),
            group_directories: HashMap::new(),
            path_user_edited: false,
            group_picker: ListPicker::new("Select Group"),
            branch_picker: ListPicker::new("Select Branch"),
            dir_picker: DirPicker::new(),
            worktree_branch: Input::default(),
            create_new_branch: true,
            workspace_repos: Vec::new(),
            workspace_repos_expanded: false,
            workspace_repo_selected_index: 0,
            workspace_repo_editing_input: None,
            workspace_repo_adding_new: false,
            workspace_repo_ghost: None,
            dir_picker_target: DirPickerTarget::SessionPath,
            worktree_config_mode: false,
            worktree_config_focused_field: 0,
            sandbox_enabled: false,
            sandbox_image: Input::new(
                containers::get_container_runtime().effective_default_image(),
            ),
            docker_available: false,
            yolo_mode: false,
            yolo_mode_default: false,
            cross_agent_team: config.session.cross_agent_team_default,
            cross_agent_team_channel: config.session.cross_agent_team_channel.clone(),
            extra_env: Vec::new(),
            env_list_expanded: false,
            env_selected_index: 0,
            env_editing_input: None,
            env_adding_new: false,
            inherited_settings: Vec::new(),
            sandbox_config_mode: false,
            sandbox_focused_field: 0,
            tool_config_mode: false,
            tool_config_focused_field: 0,
            extra_args: Input::default(),
            command_override: Input::default(),
            error_message: None,
            show_help: false,
            loading: false,
            spinner_frame: 0,
            has_hooks: false,
            current_hook: None,
            hook_output: Vec::new(),
            group_ghost: None,
            confirm_create_dirs: None,
            confirm_reuse_worktree: false,
            saved_yolo_mode: None,
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_tools(tools: Vec<&'static str>, path: String) -> Self {
        Self::new_with_config(tools, path, Config::default())
    }

    pub fn set_error(&mut self, error: String) {
        self.error_message = Some(error);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<NewSessionData> {
        // When loading, only allow Esc to cancel
        if self.loading {
            if matches!(key.code, KeyCode::Esc) {
                self.loading = false;
                return DialogResult::Cancel;
            }
            return DialogResult::Continue;
        }

        if self.show_help {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                self.show_help = false;
            }
            return DialogResult::Continue;
        }

        // Delegate to sandbox config mode handler when active
        if self.sandbox_config_mode {
            return self.handle_sandbox_config_key(key);
        }

        // Delegate to tool config mode handler when active
        if self.tool_config_mode {
            return self.handle_tool_config_key(key);
        }

        // Delegate to worktree config mode handler when active
        if self.worktree_config_mode {
            return self.handle_worktree_config_key(key);
        }

        if self.confirm_create_dirs.is_some() {
            return self.handle_confirm_create_dirs_key(key);
        }

        if self.group_picker.is_active() {
            if let ListPickerResult::Selected(value) = self.group_picker.handle_key(key) {
                self.group = Input::new(value);
                self.clear_group_ghost();
                self.apply_group_default_directory();
            }
            return DialogResult::Continue;
        }

        if self.branch_picker.is_active() {
            if let ListPickerResult::Selected(value) = self.branch_picker.handle_key(key) {
                self.worktree_branch = Input::new(value);
                self.confirm_reuse_worktree = false;
            }
            return DialogResult::Continue;
        }

        if self.dir_picker.is_active() {
            match self.dir_picker.handle_key(key) {
                DirPickerResult::Selected(path) => match self.dir_picker_target {
                    DirPickerTarget::SessionPath => {
                        self.path.set_value(path);
                        self.path_user_edited = true;
                    }
                    DirPickerTarget::RightPanePath => self.right_pane_path.set_value(path),
                    DirPickerTarget::WorkspaceRepo => {
                        self.workspace_repo_editing_input = Some(Input::new(path));
                        self.workspace_repo_ghost = self
                            .workspace_repo_editing_input
                            .as_ref()
                            .and_then(path_input::compute_path_ghost);
                    }
                },
                DirPickerResult::Cancelled | DirPickerResult::Continue => {}
            }
            return DialogResult::Continue;
        }

        // Worktree sub-options (extra_repos) are in a Ctrl+P overlay.
        // Tool config (extra_args, command_override) is in a Ctrl+P overlay on tool field.
        // Sandbox sub-options are in a separate sandbox_config_mode overlay.
        let layout = self.field_layout();
        let tool_field = layout.tool;
        let right_pane_field = layout.right_pane;
        let yolo_mode_field = layout.yolo;
        let cross_agent_team_field = layout.cross_agent_team;
        let has_cross_agent_team = cross_agent_team_field != ABSENT;
        let worktree_field = layout.worktree;
        let new_branch_field = layout.new_branch;
        let sandbox_field = layout.sandbox;
        let group_field = layout.group;
        let max_field = layout.count;

        // Ctrl+P opens a context-sensitive picker/config overlay
        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.focused_field == layout.path {
                self.dir_picker_target = DirPickerTarget::SessionPath;
                self.path.activate_picker(&mut self.dir_picker);
                return DialogResult::Continue;
            }
            if self.focused_field == layout.right_pane_path {
                self.dir_picker_target = DirPickerTarget::RightPanePath;
                self.right_pane_path.activate_picker(&mut self.dir_picker);
                return DialogResult::Continue;
            }
            if self.focused_field == tool_field {
                self.tool_config_mode = true;
                self.tool_config_focused_field = 0;
                return DialogResult::Continue;
            }
            if self.focused_field == group_field && !self.existing_groups.is_empty() {
                self.group_picker.activate(self.existing_groups.clone());
                return DialogResult::Continue;
            }
            if self.focused_field == worktree_field && !self.worktree_branch.value().is_empty() {
                self.worktree_config_mode = true;
                self.worktree_config_focused_field = 0;
                return DialogResult::Continue;
            }
            if self.focused_field == sandbox_field && self.sandbox_enabled {
                self.sandbox_config_mode = true;
                self.sandbox_focused_field = 0;
                return DialogResult::Continue;
            }
        }

        if self.handle_path_shortcuts(key) {
            return DialogResult::Continue;
        }

        if self.handle_group_shortcuts(key, group_field) {
            return DialogResult::Continue;
        }

        match key.code {
            KeyCode::Char('?') => {
                self.show_help = true;
                DialogResult::Continue
            }
            KeyCode::Esc => {
                self.error_message = None;
                DialogResult::Cancel
            }
            KeyCode::Enter => {
                self.error_message = None;
                // Refused before the create-directory confirmation: a path that
                // exists but is not a directory is not missing, so confirming
                // would create nothing and the split would fail on a value the
                // dialog had already accepted.
                if let Some((field, path)) = self.non_directory_path() {
                    self.error_message = Some(format!("Not a directory: {path}"));
                    self.focused_field = field;
                    return DialogResult::Continue;
                }
                let missing = self.missing_directories();
                if !missing.is_empty() {
                    self.confirm_create_dirs = Some(CreateDirsConfirm {
                        dirs: missing,
                        yes_selected: false,
                    });
                    return DialogResult::Continue;
                }
                // Check for worktree reuse: if a worktree branch is specified,
                // compute the path and warn once before allowing reuse.
                if !self.confirm_reuse_worktree {
                    if let Some(existing_path) = self.check_worktree_exists() {
                        self.confirm_reuse_worktree = true;
                        self.error_message = Some(format!(
                            "Worktree already exists at {}. Press Enter again to reuse it.",
                            existing_path
                        ));
                        return DialogResult::Continue;
                    }
                }
                self.build_submit_result()
            }
            KeyCode::Tab | KeyCode::Down => {
                self.clear_focused_ghost();
                self.focused_field = (self.focused_field + 1) % max_field;
                self.recompute_focused_ghost();
                DialogResult::Continue
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.clear_focused_ghost();
                self.focused_field = if self.focused_field == 0 {
                    max_field - 1
                } else {
                    self.focused_field - 1
                };
                self.recompute_focused_ghost();
                DialogResult::Continue
            }
            KeyCode::Left if self.focused_field == tool_field => {
                let len = self.available_tools.len();
                self.tool_index = (self.tool_index + len - 1) % len;
                self.reload_tool_config();
                DialogResult::Continue
            }
            KeyCode::Right if self.focused_field == tool_field => {
                self.tool_index = (self.tool_index + 1) % self.available_tools.len();
                if self.selected_tool_always_yolo() {
                    self.yolo_mode = true;
                } else {
                    self.yolo_mode = self.yolo_mode_default;
                }
                self.reload_tool_config();
                DialogResult::Continue
            }
            KeyCode::Char(' ') if self.focused_field == tool_field => {
                self.tool_index = (self.tool_index + 1) % self.available_tools.len();
                if self.selected_tool_always_yolo() {
                    self.yolo_mode = true;
                } else {
                    self.yolo_mode = self.yolo_mode_default;
                }
                self.reload_tool_config();
                DialogResult::Continue
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                if self.focused_field == sandbox_field =>
            {
                self.sandbox_enabled = !self.sandbox_enabled;
                if self.sandbox_enabled {
                    let config = resolve_config(&self.profile).unwrap_or_default();
                    self.extra_env = config.sandbox.environment.clone();
                    self.inherited_settings = build_inherited_settings(&config.sandbox);
                } else {
                    self.extra_env.clear();
                    self.env_list_expanded = false;
                    self.env_editing_input = None;
                    self.inherited_settings.clear();
                    self.sandbox_config_mode = false;
                }
                // Sandboxing hides the Cross Agent Teams and Right Pane Path
                // fields, so the checkbox just toggled has moved. Follow it
                // rather than leaving focus on whatever inherited its index.
                self.focused_field = self.field_layout().sandbox;
                DialogResult::Continue
            }
            KeyCode::Left if self.focused_field == right_pane_field => {
                let len = self.available_tools.len() + 1; // +1 for "none"
                self.right_pane_tool_index = (self.right_pane_tool_index + len - 1) % len;
                self.sync_yolo_for_right_pane();
                DialogResult::Continue
            }
            KeyCode::Right | KeyCode::Char(' ') if self.focused_field == right_pane_field => {
                let len = self.available_tools.len() + 1; // +1 for "none"
                self.right_pane_tool_index = (self.right_pane_tool_index + 1) % len;
                self.sync_yolo_for_right_pane();
                DialogResult::Continue
            }
            KeyCode::Char(' ') if self.focused_field == yolo_mode_field => {
                self.yolo_mode = !self.yolo_mode;
                DialogResult::Continue
            }
            KeyCode::Char(' ') if self.focused_field == cross_agent_team_field => {
                self.cross_agent_team = !self.cross_agent_team;
                DialogResult::Continue
            }
            // Left/Right move focus between the YOLO Mode and Cross Agent Teams
            // checkboxes (rendered on the same row). Falls back to toggling YOLO
            // when the Cross Agent Teams checkbox is not shown.
            KeyCode::Left | KeyCode::Right
                if self.focused_field == yolo_mode_field && has_cross_agent_team =>
            {
                self.focused_field = cross_agent_team_field;
                DialogResult::Continue
            }
            KeyCode::Left | KeyCode::Right if self.focused_field == cross_agent_team_field => {
                self.focused_field = yolo_mode_field;
                DialogResult::Continue
            }
            KeyCode::Left | KeyCode::Right if self.focused_field == yolo_mode_field => {
                self.yolo_mode = !self.yolo_mode;
                DialogResult::Continue
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                if self.focused_field == new_branch_field =>
            {
                self.create_new_branch = !self.create_new_branch;
                DialogResult::Continue
            }
            _ => {
                if self.focused_field != tool_field
                    && self.focused_field != right_pane_field
                    && self.focused_field != new_branch_field
                    && self.focused_field != sandbox_field
                    && self.focused_field != yolo_mode_field
                    && self.focused_field != cross_agent_team_field
                {
                    if self.focused_field == layout.path {
                        self.path.handle_text_key(key);
                        self.path_user_edited = true;
                    } else if self.focused_field == layout.right_pane_path {
                        self.right_pane_path.handle_text_key(key);
                    } else {
                        self.current_input_mut()
                            .handle_event(&crossterm::event::Event::Key(key));
                    }
                    self.error_message = None;
                    self.confirm_reuse_worktree = false;
                    if self.focused_field == group_field {
                        self.recompute_group_ghost();
                        self.apply_group_default_directory();
                    }
                }
                DialogResult::Continue
            }
        }
    }

    /// Handle key events when in sandbox configuration mode.
    fn handle_sandbox_config_key(&mut self, key: KeyEvent) -> DialogResult<NewSessionData> {
        // Sandbox config fields: 0=image, 1=env (inherited is always-visible, not focusable)
        const SANDBOX_IMAGE: usize = 0;
        const SANDBOX_ENV: usize = 1;
        const SANDBOX_MAX: usize = 2;

        // Handle env list editing when expanded
        if self.env_list_expanded && self.sandbox_focused_field == SANDBOX_ENV {
            return self.handle_env_list_key(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.sandbox_config_mode = false;
                DialogResult::Continue
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                DialogResult::Continue
            }
            KeyCode::Enter if self.sandbox_focused_field == SANDBOX_ENV => {
                self.env_list_expanded = true;
                self.env_selected_index = 0;
                DialogResult::Continue
            }
            KeyCode::Enter => {
                self.sandbox_config_mode = false;
                DialogResult::Continue
            }
            KeyCode::Tab | KeyCode::Down => {
                self.sandbox_focused_field = (self.sandbox_focused_field + 1) % SANDBOX_MAX;
                DialogResult::Continue
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.sandbox_focused_field = if self.sandbox_focused_field == 0 {
                    SANDBOX_MAX - 1
                } else {
                    self.sandbox_focused_field - 1
                };
                DialogResult::Continue
            }
            _ => {
                // Text input for image field only
                if self.sandbox_focused_field == SANDBOX_IMAGE {
                    self.sandbox_image
                        .handle_event(&crossterm::event::Event::Key(key));
                }
                DialogResult::Continue
            }
        }
    }

    /// Handle key events when in tool configuration mode.
    fn handle_tool_config_key(&mut self, key: KeyEvent) -> DialogResult<NewSessionData> {
        // Tool config fields: 0=command override, 1=extra args
        const TOOL_CMD: usize = 0;
        const TOOL_ARGS: usize = 1;
        const TOOL_MAX: usize = 2;

        match key.code {
            KeyCode::Esc => {
                self.tool_config_mode = false;
                DialogResult::Continue
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                DialogResult::Continue
            }
            KeyCode::Enter => {
                self.tool_config_mode = false;
                DialogResult::Continue
            }
            KeyCode::Tab | KeyCode::Down => {
                self.tool_config_focused_field = (self.tool_config_focused_field + 1) % TOOL_MAX;
                DialogResult::Continue
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.tool_config_focused_field = if self.tool_config_focused_field == 0 {
                    TOOL_MAX - 1
                } else {
                    self.tool_config_focused_field - 1
                };
                DialogResult::Continue
            }
            _ => {
                match self.tool_config_focused_field {
                    TOOL_CMD => {
                        self.command_override
                            .handle_event(&crossterm::event::Event::Key(key));
                    }
                    TOOL_ARGS => {
                        self.extra_args
                            .handle_event(&crossterm::event::Event::Key(key));
                    }
                    _ => {}
                }
                DialogResult::Continue
            }
        }
    }

    /// Handle key events when in worktree configuration mode.
    fn handle_worktree_config_key(&mut self, key: KeyEvent) -> DialogResult<NewSessionData> {
        // Worktree config fields: 0=new_branch checkbox, 1=extra_repos list
        const WT_NEW_BRANCH: usize = 0;
        const WT_EXTRA_REPOS: usize = 1;
        const WT_MAX: usize = 2;

        // Handle workspace repos list editing when expanded
        if self.workspace_repos_expanded && self.worktree_config_focused_field == WT_EXTRA_REPOS {
            return self.handle_workspace_repos_list_key(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.worktree_config_mode = false;
                DialogResult::Continue
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                DialogResult::Continue
            }
            // Ctrl+P on new_branch field opens branch picker
            KeyCode::Char('p')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.worktree_config_focused_field == WT_NEW_BRANCH =>
            {
                let path = std::path::PathBuf::from(self.path.resolved());
                if let Ok(branches) = crate::git::diff::list_branches(&path) {
                    if !branches.is_empty() {
                        self.branch_picker.activate(branches);
                    }
                }
                DialogResult::Continue
            }
            KeyCode::Enter if self.worktree_config_focused_field == WT_EXTRA_REPOS => {
                self.workspace_repos_expanded = true;
                self.workspace_repo_selected_index = 0;
                DialogResult::Continue
            }
            KeyCode::Enter => {
                self.worktree_config_mode = false;
                DialogResult::Continue
            }
            KeyCode::Tab | KeyCode::Down => {
                self.worktree_config_focused_field =
                    (self.worktree_config_focused_field + 1) % WT_MAX;
                DialogResult::Continue
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.worktree_config_focused_field = if self.worktree_config_focused_field == 0 {
                    WT_MAX - 1
                } else {
                    self.worktree_config_focused_field - 1
                };
                DialogResult::Continue
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                if self.worktree_config_focused_field == WT_NEW_BRANCH =>
            {
                self.create_new_branch = !self.create_new_branch;
                DialogResult::Continue
            }
            _ => DialogResult::Continue,
        }
    }

    /// Handle key events when the env list is expanded
    fn handle_env_list_key(&mut self, key: KeyEvent) -> DialogResult<NewSessionData> {
        let validate =
            |value: &str, list: &[String]| !value.is_empty() && !list.contains(&value.to_string());
        let snapshot: Vec<String> = self.extra_env.clone();
        let result = handle_editable_list_key(
            key,
            &mut self.extra_env,
            &mut self.env_list_expanded,
            &mut self.env_selected_index,
            &mut self.env_editing_input,
            &mut self.env_adding_new,
            validate,
        );

        // Validate the current entry if the list changed
        if self.extra_env != snapshot {
            self.error_message = self
                .extra_env
                .get(self.env_selected_index)
                .and_then(|entry| crate::session::validate_env_entry(entry));
        }

        result
    }

    /// Handle key events when the workspace repos list is expanded
    fn handle_workspace_repos_list_key(&mut self, key: KeyEvent) -> DialogResult<NewSessionData> {
        // When actively editing a repo path, handle path-specific keys first
        if self.workspace_repo_editing_input.is_some() {
            // Ctrl+P: open dir picker for repo path
            if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
                let initial = self
                    .workspace_repo_editing_input
                    .as_ref()
                    .map(|i| i.value().trim().to_string())
                    .unwrap_or_default();
                let initial = if initial.is_empty() {
                    std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| ".".to_string())
                } else {
                    initial
                };
                self.dir_picker_target = DirPickerTarget::WorkspaceRepo;
                self.dir_picker.activate(&initial);
                return DialogResult::Continue;
            }

            // Right/End at end of input: accept ghost text
            if matches!(key.code, KeyCode::Right | KeyCode::End)
                && key.modifiers == KeyModifiers::NONE
            {
                if let Some(ref input) = self.workspace_repo_editing_input {
                    let cursor = input.visual_cursor();
                    let char_len = input.value().chars().count();
                    if cursor >= char_len {
                        if let Some(ghost) = self.workspace_repo_ghost.take() {
                            if let Some(ref mut input) = self.workspace_repo_editing_input {
                                if let Some(new_value) = ghost.accept(input) {
                                    *input = Input::new(new_value);
                                    self.workspace_repo_ghost =
                                        path_input::compute_path_ghost(input);
                                    return DialogResult::Continue;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Intercept 'a' to pre-populate with the expanded cwd (like the main path field)
        if self.workspace_repo_editing_input.is_none()
            && key.code == KeyCode::Char('a')
            && key.modifiers == KeyModifiers::NONE
        {
            let cwd = std::env::current_dir()
                .map(|p| {
                    let mut s = path_input::collapse_tilde(&p.to_string_lossy());
                    if !s.ends_with('/') {
                        s.push('/');
                    }
                    s
                })
                .unwrap_or_default();
            self.workspace_repo_editing_input = Some(Input::new(cwd));
            self.workspace_repo_adding_new = true;
            self.workspace_repo_ghost = self
                .workspace_repo_editing_input
                .as_ref()
                .and_then(path_input::compute_path_ghost);
            return DialogResult::Continue;
        }

        let validate =
            |value: &str, list: &[String]| !value.is_empty() && !list.contains(&value.to_string());

        // Wrap the generic handler to add tilde expansion and ghost recomputation
        let had_input = self.workspace_repo_editing_input.is_some();
        let was_adding = self.workspace_repo_adding_new;
        let edit_index = self.workspace_repo_selected_index;
        let result = handle_editable_list_key(
            key,
            &mut self.workspace_repos,
            &mut self.workspace_repos_expanded,
            &mut self.workspace_repo_selected_index,
            &mut self.workspace_repo_editing_input,
            &mut self.workspace_repo_adding_new,
            validate,
        );

        // If editing just finished (Enter pressed), expand tilde in the stored value
        if had_input && self.workspace_repo_editing_input.is_none() {
            let idx = if was_adding {
                self.workspace_repos.len().saturating_sub(1)
            } else {
                edit_index
            };
            if let Some(entry) = self.workspace_repos.get_mut(idx) {
                *entry = path_input::expand_tilde(entry);
            }
            self.workspace_repo_ghost = None;
        }

        // If still editing, recompute ghost
        if self.workspace_repo_editing_input.is_some() {
            self.workspace_repo_ghost = self
                .workspace_repo_editing_input
                .as_ref()
                .and_then(path_input::compute_path_ghost);
        } else {
            self.workspace_repo_ghost = None;
        }

        result
    }

    fn reload_tool_config(&mut self) {
        let config = resolve_config(&self.profile).unwrap_or_default();
        let tool = self
            .available_tools
            .get(self.tool_index)
            .or_else(|| self.available_tools.first())
            .copied()
            .unwrap_or("claude");
        self.extra_args = Input::new(
            config
                .session
                .agent_extra_args
                .get(tool)
                .cloned()
                .unwrap_or_default(),
        );
        self.command_override = Input::new(
            config
                .session
                .agent_command_override
                .get(tool)
                .cloned()
                .unwrap_or_default(),
        );
        if tool == "shell" {
            self.saved_yolo_mode = Some(self.yolo_mode);
            if !self.right_pane_needs_yolo() {
                self.yolo_mode = false;
            }
            self.worktree_branch = Input::default();
            self.create_new_branch = true;
        } else if let Some(saved) = self.saved_yolo_mode.take() {
            self.yolo_mode = saved;
        }
    }

    fn current_input_mut(&mut self) -> &mut Input {
        let layout = self.field_layout();
        match self.focused_field {
            n if n == layout.title => &mut self.title,
            n if n == layout.worktree => &mut self.worktree_branch,
            n if n == layout.group => &mut self.group,
            _ => &mut self.title,
        }
    }

    /// Check if the worktree path for the current branch already exists on disk.
    /// Returns `Some(path_display)` if it exists, `None` otherwise.
    fn check_worktree_exists(&self) -> Option<String> {
        use crate::git::GitWorktree;

        let branch = self.worktree_branch.value().trim();
        if branch.is_empty() {
            return None;
        }

        let path = std::path::PathBuf::from(self.path.resolved());

        if !GitWorktree::is_git_repo(&path) {
            return None;
        }

        let main_repo_path = GitWorktree::find_main_repo(&path).ok()?;
        let git_wt = GitWorktree::new(main_repo_path.clone()).ok()?;

        let config = resolve_config(&self.profile).unwrap_or_default();
        let is_bare = GitWorktree::is_bare_repo(&main_repo_path);
        let template = if is_bare {
            &config.worktree.bare_repo_path_template
        } else {
            &config.worktree.path_template
        };

        // Use a dummy session_id since we just need to check existence
        let worktree_path = git_wt.compute_path(branch, template, "00000000").ok()?;
        if worktree_path.exists() {
            Some(worktree_path.display().to_string())
        } else {
            None
        }
    }

    fn build_submit_result(&self) -> DialogResult<NewSessionData> {
        let title_value = self.title.value().trim();
        let final_title = if title_value.is_empty() {
            let refs: Vec<&str> = self.existing_titles.iter().map(|s| s.as_str()).collect();
            civilizations::generate_random_title(&refs)
        } else {
            title_value.to_string()
        };
        let worktree_value = self.worktree_branch.value().trim();
        let has_worktree_branch = !worktree_value.is_empty();
        let worktree_branch = if has_worktree_branch {
            Some(worktree_value.to_string())
        } else {
            None
        };
        DialogResult::Submit(NewSessionData {
            profile: self.profile.clone(),
            title: final_title,
            path: self.path.trimmed(),
            group: self.group.value().trim().to_string(),
            tool: self.available_tools[self.tool_index].to_string(),
            worktree_branch,
            create_new_branch: self.create_new_branch,
            extra_repo_paths: if has_worktree_branch {
                self.workspace_repos.clone()
            } else {
                Vec::new()
            },
            sandbox: self.sandbox_enabled,
            sandbox_image: self.sandbox_image.value().trim().to_string(),
            yolo_mode: self.yolo_mode || self.selected_tool_always_yolo(),
            cross_agent_team: self.cross_agent_team && self.has_cross_agent_team_field(),
            cross_agent_team_channel: self.cross_agent_team_channel.clone(),
            extra_env: if self.sandbox_enabled {
                self.extra_env.clone()
            } else {
                Vec::new()
            },
            extra_args: self.extra_args.value().trim().to_string(),
            command_override: self.command_override.value().trim().to_string(),
            reuse_worktree: self.confirm_reuse_worktree,
            right_pane_tool: if self.right_pane_tool_index == 0 {
                None
            } else {
                self.available_tools
                    .get(self.right_pane_tool_index - 1)
                    .map(|t| t.to_string())
            },
            right_pane_path: self.right_pane_path_value(),
        })
    }

    /// The right pane's chosen directory, or `None` for "the session's own".
    /// A hidden field contributes nothing, so toggling sandboxing on does not
    /// smuggle a directory into a session that cannot use it.
    fn right_pane_path_value(&self) -> Option<String> {
        if !self.has_right_pane_path_field() {
            return None;
        }
        if self.right_pane_path.trimmed().is_empty() {
            return None;
        }
        // The resolved form, not the typed one: a leading `~` is expanded by the
        // shell, never by tmux, so handing `split-window -c` the literal text
        // fails the pane after the existence check has already passed on the
        // expanded path.
        Some(self.right_pane_path.resolved())
    }

    /// Every directory this submit would have to create, in field order.
    fn missing_directories(&self) -> Vec<String> {
        let mut missing = Vec::new();
        let session_path = self.path.resolved();
        if !std::path::Path::new(&session_path).exists() {
            missing.push(session_path);
        }
        if self.right_pane_path_value().is_some() {
            let right_pane_path = self.right_pane_path.resolved();
            if !std::path::Path::new(&right_pane_path).exists()
                && !missing.contains(&right_pane_path)
            {
                missing.push(right_pane_path);
            }
        }
        missing
    }

    /// A path that exists but is not a directory, if either field names one.
    ///
    /// `exists()` alone lets a regular file through as "not missing", and the
    /// split then fails on a path the dialog had already accepted.
    /// Returns the field alongside the path: matching the path back to a field
    /// afterwards picks the wrong one when both fields name the same file.
    fn non_directory_path(&self) -> Option<(usize, String)> {
        let layout = self.field_layout();
        let candidates = [
            (layout.path, Some(self.path.resolved())),
            (
                layout.right_pane_path,
                self.right_pane_path_value()
                    .map(|_| self.right_pane_path.resolved()),
            ),
        ];
        for (field, path) in candidates {
            let Some(path) = path else { continue };
            let p = std::path::Path::new(&path);
            if p.exists() && !p.is_dir() {
                return Some((field, path));
            }
        }
        None
    }

    /// The field a directory belongs to, so a creation failure lands the cursor
    /// on the input the user has to fix.
    fn field_for_directory(&self, dir: &str) -> usize {
        let layout = self.field_layout();
        if layout.right_pane_path != ABSENT && self.right_pane_path.resolved() == dir {
            layout.right_pane_path
        } else {
            layout.path
        }
    }

    fn handle_confirm_create_dirs_key(&mut self, key: KeyEvent) -> DialogResult<NewSessionData> {
        let confirm = self.confirm_create_dirs.as_mut().unwrap();
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                confirm.yes_selected = true;
                DialogResult::Continue
            }
            KeyCode::Right | KeyCode::Char('l') => {
                confirm.yes_selected = false;
                DialogResult::Continue
            }
            KeyCode::Tab => {
                confirm.yes_selected = !confirm.yes_selected;
                DialogResult::Continue
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => self.create_dirs_and_submit(),
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => self.decline_create_dirs(),
            KeyCode::Enter => {
                if confirm.yes_selected {
                    self.create_dirs_and_submit()
                } else {
                    self.decline_create_dirs()
                }
            }
            _ => DialogResult::Continue,
        }
    }

    /// Declining creates nothing, which is the whole reason one confirmation
    /// covers every directory rather than one prompt per field.
    fn decline_create_dirs(&mut self) -> DialogResult<NewSessionData> {
        self.confirm_create_dirs = None;
        self.focused_field = self.path_field();
        DialogResult::Continue
    }

    fn create_dirs_and_submit(&mut self) -> DialogResult<NewSessionData> {
        let Some(confirm) = self.confirm_create_dirs.take() else {
            return DialogResult::Continue;
        };
        // One confirmation covering every directory has to mean all or none.
        // A failure partway through would otherwise leave the earlier ones
        // behind while the session is not created, which is neither what the
        // user confirmed nor something the dialog goes on to mention.
        //
        // Only the components this dialog itself created are rolled back, and
        // every one of them is. `create_dir_all` gives neither: it reports
        // success for a directory that already existed, and it creates parents
        // without naming them, so undoing only the leaf leaves the parents
        // behind. See `create_dir_tracked`.
        let mut owned: Vec<std::path::PathBuf> = Vec::new();
        for dir in &confirm.dirs {
            if let Err(e) = create_dir_tracked(dir, &mut owned) {
                let mut kept = Vec::new();
                for done in owned.iter().rev() {
                    if std::fs::remove_dir(done).is_err() {
                        kept.push(done.display().to_string());
                    }
                }
                // Silence here would leave directories behind after telling the
                // user nothing was created.
                self.error_message = Some(if kept.is_empty() {
                    format!("Failed to create directory {}: {}", dir, e)
                } else {
                    format!(
                        "Failed to create directory {}: {}. Left in place: {}",
                        dir,
                        e,
                        kept.join(", ")
                    )
                });
                self.focused_field = self.field_for_directory(dir);
                return DialogResult::Continue;
            }
        }
        self.build_submit_result()
    }

    fn clear_focused_ghost(&mut self) {
        let layout = self.field_layout();
        if self.focused_field == layout.path {
            self.path.clear_ghost();
        } else if self.focused_field == layout.right_pane_path {
            self.right_pane_path.clear_ghost();
        } else if self.focused_field == layout.group {
            self.clear_group_ghost();
        }
    }

    fn recompute_focused_ghost(&mut self) {
        let layout = self.field_layout();
        if self.focused_field == layout.path {
            self.path.recompute_ghost();
        } else if self.focused_field == layout.right_pane_path {
            self.right_pane_path.recompute_ghost();
        } else if self.focused_field == layout.group {
            self.recompute_group_ghost();
        }
    }
}
