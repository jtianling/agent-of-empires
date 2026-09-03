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

use crate::session::repo_config::HookProgress;
#[cfg(test)]
use crate::session::Config;
use crate::session::{civilizations, resolve_config, PaneDraft, PaneWorktreeRequest};
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
}

pub(super) struct FieldHelp {
    pub(super) name: &'static str,
    pub(super) description: &'static str,
    pub(super) visibility: HelpVisibility,
}

pub(super) const HELP_DIALOG_WIDTH: u16 = 85;

pub(super) const FIELD_HELP: &[FieldHelp] = &[
    FieldHelp {
        name: "Title",
        description: "Session name (auto-generates if empty)",
        visibility: HelpVisibility::Always,
    },
    FieldHelp {
        name: "Group",
        description: "Optional grouping for organization (Ctrl+P to browse existing groups)",
        visibility: HelpVisibility::Always,
    },
    FieldHelp {
        name: "Tool",
        description: "Which AI tool to use (Ctrl+P to configure command and extra args)",
        visibility: HelpVisibility::ToolSelection,
    },
    FieldHelp {
        name: "Path",
        description: "Working directory for the session",
        visibility: HelpVisibility::Always,
    },
    FieldHelp {
        name: "YOLO Mode",
        description:
            "Skip permission prompts for autonomous operation (--dangerously-skip-permissions)",
        visibility: HelpVisibility::Yolo,
    },
    FieldHelp {
        name: "Cross Agent Team",
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
        name: "Right Pane",
        description: "Optional tool for an auto-created right pane (Left/Right to cycle)",
        visibility: HelpVisibility::Always,
    },
    FieldHelp {
        name: "Right Pane Path",
        description: "Working directory for the right pane (empty = same as the session)",
        visibility: HelpVisibility::RightPanePath,
    },
];

#[derive(Clone)]
pub struct NewSessionData {
    pub profile: String,
    pub title: String,
    pub group: String,
    pub primary: PaneDraft,
    pub secondary: Option<PaneDraft>,
    /// Claude development-channels string for Cross Agent Team launches.
    pub cross_agent_team_channel: String,
    /// Extra arguments to append after the agent binary
    pub extra_args: String,
    /// Command override for the agent binary (replaces the default binary)
    pub command_override: String,
}

/// Spinner frames for loading animation
pub(super) const SPINNER_FRAMES: &[&str] = &["◐", "◓", "◑", "◒"];

/// Which field the shared directory picker was opened for. The picker itself
/// only reports a directory, so the dialog has to remember where to put it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DirPickerTarget {
    SessionPath,
    RightPanePath,
    WorkspaceRepo(PaneTarget),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneTarget {
    Primary,
    Secondary,
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

pub(super) struct PaneDialogState {
    pub(super) tool_index: usize,
    pub(super) path: PathField,
    pub(super) worktree_branch: Input,
    pub(super) create_new_branch: bool,
    pub(super) yolo_mode: bool,
    pub(super) cross_agent_team: bool,
    pub(super) workspace_repos: Vec<String>,
    pub(super) workspace_repos_expanded: bool,
    pub(super) workspace_repo_selected_index: usize,
    pub(super) workspace_repo_editing_input: Option<Input>,
    pub(super) workspace_repo_adding_new: bool,
    pub(super) workspace_repo_ghost: Option<PathGhostCompletion>,
    pub(super) confirm_reuse_worktree: bool,
    saved_yolo_mode: Option<bool>,
    saved_cross_agent_team: Option<bool>,
}

impl PaneDialogState {
    fn new(path: String, tool_index: usize, yolo_mode: bool, cross_agent_team: bool) -> Self {
        Self {
            tool_index,
            path: PathField::new(path),
            worktree_branch: Input::default(),
            create_new_branch: true,
            yolo_mode,
            cross_agent_team,
            workspace_repos: Vec::new(),
            workspace_repos_expanded: false,
            workspace_repo_selected_index: 0,
            workspace_repo_editing_input: None,
            workspace_repo_adding_new: false,
            workspace_repo_ghost: None,
            confirm_reuse_worktree: false,
            saved_yolo_mode: None,
            saved_cross_agent_team: None,
        }
    }
}

pub struct NewSessionDialog {
    pub(super) profile: String,
    pub(super) title: Input,
    pub(super) group: Input,
    pub(super) primary: PaneDialogState,
    pub(super) secondary: Option<PaneDialogState>,
    pub(super) collapsed_secondary: Option<PaneDialogState>,
    pub(super) focused_field: usize,
    pub(super) available_tools: Vec<&'static str>,
    pub(super) existing_titles: Vec<String>,
    pub(super) cross_agent_team_channel: String,
    default_yolo_mode: bool,
    default_cross_agent_team: bool,
    /// Which field the directory picker will write into when it returns.
    pub(super) dir_picker_target: DirPickerTarget,
    /// Worktree configuration overlay mode (Ctrl+P on worktree field)
    pub(super) worktree_config_mode: bool,
    /// Focused field within the worktree config overlay (0=new_branch, 1=extra_repos)
    pub(super) worktree_config_focused_field: usize,
    pub(super) worktree_config_target: PaneTarget,
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

        Self {
            profile: profile.to_string(),
            title: Input::default(),
            group: Input::new(default_group.unwrap_or_default()),
            primary: PaneDialogState::new(current_dir, tool_index, yolo_mode, cross_agent_team),
            secondary: None,
            collapsed_secondary: None,
            focused_field: 0,
            available_tools,
            existing_titles,
            existing_groups,
            group_directories,
            path_user_edited: false,
            group_picker: ListPicker::new("Select Group"),
            branch_picker: ListPicker::new("Select Branch"),
            dir_picker: DirPicker::new(),
            dir_picker_target: DirPickerTarget::SessionPath,
            worktree_config_mode: false,
            worktree_config_focused_field: 0,
            worktree_config_target: PaneTarget::Primary,
            cross_agent_team_channel,
            default_yolo_mode: yolo_mode,
            default_cross_agent_team: cross_agent_team,
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
        }
    }

    /// Pre-fill the path field (e.g. from a selected session).
    pub fn set_path(&mut self, path: String) {
        self.primary.path.set_value(path);
    }

    /// Pre-fill the group field (e.g. from a selected session or group).
    pub fn set_group(&mut self, group: String) {
        self.group = Input::new(group);
    }

    #[cfg(test)]
    pub fn path_value(&self) -> &str {
        self.primary.path.value()
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

        if self.primary.path.tick() {
            changed = true;
        }

        if self.secondary.as_mut().is_some_and(|pane| pane.path.tick()) {
            changed = true;
        }

        changed
    }

    pub(super) fn pane(&self, target: PaneTarget) -> Option<&PaneDialogState> {
        match target {
            PaneTarget::Primary => Some(&self.primary),
            PaneTarget::Secondary => self.secondary.as_ref(),
        }
    }

    pub(super) fn pane_mut(&mut self, target: PaneTarget) -> Option<&mut PaneDialogState> {
        match target {
            PaneTarget::Primary => Some(&mut self.primary),
            PaneTarget::Secondary => self.secondary.as_mut(),
        }
    }

    pub(super) fn pane_tool(&self, target: PaneTarget) -> Option<&'static str> {
        let pane = self.pane(target)?;
        self.available_tools.get(pane.tool_index).copied()
    }

    /// Whether the selected pane tool is always in YOLO mode (no opt-in needed).
    pub(super) fn pane_tool_always_yolo(&self, target: PaneTarget) -> bool {
        let Some(tool_name) = self.pane_tool(target) else {
            return false;
        };
        crate::agents::get_agent(tool_name)
            .and_then(|a| a.yolo.as_ref())
            .is_some_and(|y| matches!(y, crate::agents::YoloMode::AlwaysYolo))
    }

    pub(super) fn pane_has_yolo(&self, target: PaneTarget) -> bool {
        let Some(tool_name) = self.pane_tool(target) else {
            return false;
        };
        crate::agents::get_agent(tool_name)
            .and_then(|agent| agent.yolo.as_ref())
            .is_some_and(|mode| !matches!(mode, crate::agents::YoloMode::AlwaysYolo))
    }

    pub(super) fn has_right_pane_path_field(&self) -> bool {
        self.secondary.is_some()
    }

    pub(super) fn pane_has_cross_agent_team(&self, target: PaneTarget) -> bool {
        self.pane_tool(target)
            .is_some_and(crate::session::Instance::supports_cross_agent_team_tool)
    }

    pub(super) fn right_pane_selection_index(&self) -> usize {
        self.secondary
            .as_ref()
            .map_or(0, |pane| pane.tool_index + 1)
    }

    fn set_right_pane_selection(&mut self, selection: usize) {
        if selection == 0 {
            if let Some(pane) = self.secondary.take() {
                self.collapsed_secondary = Some(pane);
            }
            return;
        }
        let tool_index = selection - 1;
        let default_yolo_mode = self.default_yolo_mode;
        let default_cross_agent_team = self.default_cross_agent_team;
        let mut pane = self
            .secondary
            .take()
            .or_else(|| self.collapsed_secondary.take())
            .unwrap_or_else(|| {
                PaneDialogState::new(
                    String::new(),
                    tool_index,
                    default_yolo_mode,
                    default_cross_agent_team,
                )
            });
        pane.tool_index = tool_index;
        self.secondary = Some(pane);
        self.normalize_pane_for_tool(PaneTarget::Secondary);
    }

    fn normalize_pane_for_tool(&mut self, target: PaneTarget) {
        let always_yolo = self.pane_tool_always_yolo(target);
        let is_shell = self.pane_tool(target) == Some("shell");
        let Some(pane) = self.pane_mut(target) else {
            return;
        };
        if is_shell {
            pane.saved_yolo_mode.get_or_insert(pane.yolo_mode);
            pane.saved_cross_agent_team
                .get_or_insert(pane.cross_agent_team);
            pane.yolo_mode = false;
            pane.cross_agent_team = false;
        } else {
            if let Some(saved) = pane.saved_yolo_mode.take() {
                pane.yolo_mode = saved;
            }
            if always_yolo {
                pane.yolo_mode = true;
            }
        }
        if !is_shell {
            if let Some(saved) = pane.saved_cross_agent_team.take() {
                pane.cross_agent_team = saved;
            }
        }
    }

    /// Whether a help entry applies to the dialog as it currently stands. The
    /// conditions are the same ones the fields themselves are built from.
    pub(super) fn help_entry_visible(&self, visibility: HelpVisibility) -> bool {
        match visibility {
            HelpVisibility::Always => true,
            HelpVisibility::ToolSelection => self.available_tools.len() > 1,
            HelpVisibility::RightPanePath => self.has_right_pane_path_field(),
            HelpVisibility::Yolo => {
                self.pane_has_yolo(PaneTarget::Primary) || self.pane_has_yolo(PaneTarget::Secondary)
            }
            HelpVisibility::CrossAgentTeam => {
                self.pane_has_cross_agent_team(PaneTarget::Primary)
                    || self.pane_has_cross_agent_team(PaneTarget::Secondary)
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
            group: Input::default(),
            primary: PaneDialogState::new(
                path,
                tool_index,
                config.session.yolo_mode_default,
                config.session.cross_agent_team_default,
            ),
            secondary: None,
            collapsed_secondary: None,
            focused_field: 0,
            available_tools: tools,
            existing_titles: Vec::new(),
            existing_groups: Vec::new(),
            group_directories: HashMap::new(),
            path_user_edited: false,
            group_picker: ListPicker::new("Select Group"),
            branch_picker: ListPicker::new("Select Branch"),
            dir_picker: DirPicker::new(),
            dir_picker_target: DirPickerTarget::SessionPath,
            worktree_config_mode: false,
            worktree_config_focused_field: 0,
            worktree_config_target: PaneTarget::Primary,
            cross_agent_team_channel: config.session.cross_agent_team_channel.clone(),
            default_yolo_mode: config.session.yolo_mode_default,
            default_cross_agent_team: config.session.cross_agent_team_default,
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
                if let Some(pane) = self.pane_mut(self.worktree_config_target) {
                    pane.worktree_branch = Input::new(value);
                    pane.confirm_reuse_worktree = false;
                }
            }
            return DialogResult::Continue;
        }

        if self.dir_picker.is_active() {
            match self.dir_picker.handle_key(key) {
                DirPickerResult::Selected(path) => match self.dir_picker_target {
                    DirPickerTarget::SessionPath => {
                        self.primary.path.set_value(path);
                        self.path_user_edited = true;
                    }
                    DirPickerTarget::RightPanePath => {
                        if let Some(pane) = self.secondary.as_mut() {
                            pane.path.set_value(path);
                        }
                    }
                    DirPickerTarget::WorkspaceRepo(target) => {
                        if let Some(pane) = self.pane_mut(target) {
                            pane.workspace_repo_editing_input = Some(Input::new(path));
                            pane.workspace_repo_ghost = pane
                                .workspace_repo_editing_input
                                .as_ref()
                                .and_then(path_input::compute_path_ghost);
                        }
                    }
                },
                DirPickerResult::Cancelled | DirPickerResult::Continue => {}
            }
            return DialogResult::Continue;
        }

        // Worktree sub-options (extra_repos) are in a Ctrl+P overlay.
        // Tool config (extra_args, command_override) is in a Ctrl+P overlay on tool field.
        let layout = self.field_layout();
        let tool_field = layout.tool;
        let right_pane_field = layout.right_pane;
        let yolo_mode_field = layout.yolo;
        let cross_agent_team_field = layout.cross_agent_team;
        let has_cross_agent_team = cross_agent_team_field != ABSENT;
        let right_yolo_field = layout.right_pane_yolo;
        let right_cross_agent_team_field = layout.right_pane_cross_agent_team;
        let has_right_cross_agent_team = right_cross_agent_team_field != ABSENT;
        let worktree_field = layout.worktree;
        let group_field = layout.group;
        let max_field = layout.count;

        // Ctrl+P opens a context-sensitive picker/config overlay
        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.focused_field == layout.path {
                self.dir_picker_target = DirPickerTarget::SessionPath;
                self.primary.path.activate_picker(&mut self.dir_picker);
                return DialogResult::Continue;
            }
            if self.focused_field == layout.right_pane_path {
                self.dir_picker_target = DirPickerTarget::RightPanePath;
                if let Some(pane) = self.secondary.as_mut() {
                    pane.path.activate_picker(&mut self.dir_picker);
                }
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
            if self.focused_field == worktree_field
                && !self.primary.worktree_branch.value().is_empty()
            {
                self.worktree_config_mode = true;
                self.worktree_config_focused_field = 0;
                self.worktree_config_target = PaneTarget::Primary;
                return DialogResult::Continue;
            }
            if self.focused_field == layout.right_pane_worktree
                && self
                    .secondary
                    .as_ref()
                    .is_some_and(|pane| !pane.worktree_branch.value().is_empty())
            {
                self.worktree_config_mode = true;
                self.worktree_config_focused_field = 0;
                self.worktree_config_target = PaneTarget::Secondary;
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
                let existing = self.unconfirmed_worktrees();
                if !existing.is_empty() {
                    for target in existing.iter().map(|(target, _)| *target) {
                        if let Some(pane) = self.pane_mut(target) {
                            pane.confirm_reuse_worktree = true;
                        }
                    }
                    let paths = existing
                        .iter()
                        .map(|(target, path)| {
                            let label = match target {
                                PaneTarget::Primary => "primary",
                                PaneTarget::Secondary => "secondary",
                            };
                            format!("{label}: {path}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.error_message = Some(format!(
                        "Worktrees already exist. Press Enter again to reuse: {paths}"
                    ));
                    return DialogResult::Continue;
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
                self.primary.tool_index = (self.primary.tool_index + len - 1) % len;
                self.normalize_pane_for_tool(PaneTarget::Primary);
                self.reload_tool_config();
                DialogResult::Continue
            }
            KeyCode::Right if self.focused_field == tool_field => {
                self.primary.tool_index =
                    (self.primary.tool_index + 1) % self.available_tools.len();
                self.normalize_pane_for_tool(PaneTarget::Primary);
                self.reload_tool_config();
                DialogResult::Continue
            }
            KeyCode::Char(' ') if self.focused_field == tool_field => {
                self.primary.tool_index =
                    (self.primary.tool_index + 1) % self.available_tools.len();
                self.normalize_pane_for_tool(PaneTarget::Primary);
                self.reload_tool_config();
                DialogResult::Continue
            }
            KeyCode::Left if self.focused_field == right_pane_field => {
                let len = self.available_tools.len() + 1; // +1 for "none"
                let selection = (self.right_pane_selection_index() + len - 1) % len;
                self.set_right_pane_selection(selection);
                DialogResult::Continue
            }
            KeyCode::Right | KeyCode::Char(' ') if self.focused_field == right_pane_field => {
                let len = self.available_tools.len() + 1; // +1 for "none"
                let selection = (self.right_pane_selection_index() + 1) % len;
                self.set_right_pane_selection(selection);
                DialogResult::Continue
            }
            KeyCode::Char(' ') if self.focused_field == yolo_mode_field => {
                self.primary.yolo_mode = !self.primary.yolo_mode;
                DialogResult::Continue
            }
            KeyCode::Char(' ') if self.focused_field == cross_agent_team_field => {
                self.primary.cross_agent_team = !self.primary.cross_agent_team;
                DialogResult::Continue
            }
            KeyCode::Char(' ') if self.focused_field == right_yolo_field => {
                if let Some(pane) = self.secondary.as_mut() {
                    pane.yolo_mode = !pane.yolo_mode;
                }
                DialogResult::Continue
            }
            KeyCode::Char(' ') if self.focused_field == right_cross_agent_team_field => {
                if let Some(pane) = self.secondary.as_mut() {
                    pane.cross_agent_team = !pane.cross_agent_team;
                }
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
                self.primary.yolo_mode = !self.primary.yolo_mode;
                DialogResult::Continue
            }
            KeyCode::Left | KeyCode::Right
                if self.focused_field == right_yolo_field && has_right_cross_agent_team =>
            {
                self.focused_field = right_cross_agent_team_field;
                DialogResult::Continue
            }
            KeyCode::Left | KeyCode::Right
                if self.focused_field == right_cross_agent_team_field =>
            {
                self.focused_field = right_yolo_field;
                DialogResult::Continue
            }
            KeyCode::Left | KeyCode::Right if self.focused_field == right_yolo_field => {
                if let Some(pane) = self.secondary.as_mut() {
                    pane.yolo_mode = !pane.yolo_mode;
                }
                DialogResult::Continue
            }
            _ => {
                if self.focused_field != tool_field
                    && self.focused_field != right_pane_field
                    && self.focused_field != yolo_mode_field
                    && self.focused_field != cross_agent_team_field
                    && self.focused_field != right_yolo_field
                    && self.focused_field != right_cross_agent_team_field
                {
                    if self.focused_field == layout.path {
                        self.primary.path.handle_text_key(key);
                        self.path_user_edited = true;
                    } else if self.focused_field == layout.right_pane_path {
                        if let Some(pane) = self.secondary.as_mut() {
                            pane.path.handle_text_key(key);
                        }
                    } else {
                        self.current_input_mut()
                            .handle_event(&crossterm::event::Event::Key(key));
                    }
                    self.error_message = None;
                    if self.focused_field == layout.worktree {
                        self.primary.confirm_reuse_worktree = false;
                    } else if self.focused_field == layout.right_pane_worktree {
                        if let Some(pane) = self.secondary.as_mut() {
                            pane.confirm_reuse_worktree = false;
                        }
                    }
                    if self.focused_field == group_field {
                        self.recompute_group_ghost();
                        self.apply_group_default_directory();
                    }
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
        if self
            .pane(self.worktree_config_target)
            .is_some_and(|pane| pane.workspace_repos_expanded)
            && self.worktree_config_focused_field == WT_EXTRA_REPOS
        {
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
                let path = self
                    .pane(self.worktree_config_target)
                    .map(|pane| std::path::PathBuf::from(pane.path.resolved()))
                    .unwrap_or_default();
                if let Ok(branches) = crate::git::diff::list_branches(&path) {
                    if !branches.is_empty() {
                        self.branch_picker.activate(branches);
                    }
                }
                DialogResult::Continue
            }
            KeyCode::Enter if self.worktree_config_focused_field == WT_EXTRA_REPOS => {
                if let Some(pane) = self.pane_mut(self.worktree_config_target) {
                    pane.workspace_repos_expanded = true;
                    pane.workspace_repo_selected_index = 0;
                }
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
                if let Some(pane) = self.pane_mut(self.worktree_config_target) {
                    pane.create_new_branch = !pane.create_new_branch;
                }
                DialogResult::Continue
            }
            _ => DialogResult::Continue,
        }
    }

    /// Handle key events when the workspace repos list is expanded
    fn handle_workspace_repos_list_key(&mut self, key: KeyEvent) -> DialogResult<NewSessionData> {
        let target = self.worktree_config_target;
        let is_editing = self
            .pane(target)
            .is_some_and(|pane| pane.workspace_repo_editing_input.is_some());
        if is_editing {
            if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
                let initial = self
                    .pane(target)
                    .and_then(|pane| pane.workspace_repo_editing_input.as_ref())
                    .map(|input| input.value().trim().to_string())
                    .unwrap_or_default();
                let initial = if initial.is_empty() {
                    std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| ".".to_string())
                } else {
                    initial
                };
                self.dir_picker_target = DirPickerTarget::WorkspaceRepo(target);
                self.dir_picker.activate(&initial);
                return DialogResult::Continue;
            }

            if matches!(key.code, KeyCode::Right | KeyCode::End)
                && key.modifiers == KeyModifiers::NONE
            {
                if let Some(pane) = self.pane_mut(target) {
                    let cursor = pane
                        .workspace_repo_editing_input
                        .as_ref()
                        .map_or(0, Input::visual_cursor);
                    let char_len = pane
                        .workspace_repo_editing_input
                        .as_ref()
                        .map_or(0, |input| input.value().chars().count());
                    if cursor >= char_len {
                        if let Some(ghost) = pane.workspace_repo_ghost.take() {
                            if let Some(ref mut input) = pane.workspace_repo_editing_input {
                                if let Some(new_value) = ghost.accept(input) {
                                    *input = Input::new(new_value);
                                    pane.workspace_repo_ghost =
                                        path_input::compute_path_ghost(input);
                                    return DialogResult::Continue;
                                }
                            }
                        }
                    }
                }
            }
        }

        if !is_editing && key.code == KeyCode::Char('a') && key.modifiers == KeyModifiers::NONE {
            let cwd = std::env::current_dir()
                .map(|p| {
                    let mut s = path_input::collapse_tilde(&p.to_string_lossy());
                    if !s.ends_with('/') {
                        s.push('/');
                    }
                    s
                })
                .unwrap_or_default();
            if let Some(pane) = self.pane_mut(target) {
                pane.workspace_repo_editing_input = Some(Input::new(cwd));
                pane.workspace_repo_adding_new = true;
                pane.workspace_repo_ghost = pane
                    .workspace_repo_editing_input
                    .as_ref()
                    .and_then(path_input::compute_path_ghost);
            }
            return DialogResult::Continue;
        }

        let validate =
            |value: &str, list: &[String]| !value.is_empty() && !list.contains(&value.to_string());

        let Some(pane) = self.pane_mut(target) else {
            return DialogResult::Continue;
        };
        let had_input = pane.workspace_repo_editing_input.is_some();
        let was_adding = pane.workspace_repo_adding_new;
        let edit_index = pane.workspace_repo_selected_index;
        let result = handle_editable_list_key(
            key,
            &mut pane.workspace_repos,
            &mut pane.workspace_repos_expanded,
            &mut pane.workspace_repo_selected_index,
            &mut pane.workspace_repo_editing_input,
            &mut pane.workspace_repo_adding_new,
            validate,
        );

        // If editing just finished (Enter pressed), expand tilde in the stored value
        if had_input && pane.workspace_repo_editing_input.is_none() {
            let idx = if was_adding {
                pane.workspace_repos.len().saturating_sub(1)
            } else {
                edit_index
            };
            if let Some(entry) = pane.workspace_repos.get_mut(idx) {
                *entry = path_input::expand_tilde(entry);
            }
            pane.workspace_repo_ghost = None;
        }

        // If still editing, recompute ghost
        if pane.workspace_repo_editing_input.is_some() {
            pane.workspace_repo_ghost = pane
                .workspace_repo_editing_input
                .as_ref()
                .and_then(path_input::compute_path_ghost);
        } else {
            pane.workspace_repo_ghost = None;
        }

        result
    }

    fn reload_tool_config(&mut self) {
        let config = resolve_config(&self.profile).unwrap_or_default();
        let tool = self
            .available_tools
            .get(self.primary.tool_index)
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
    }

    fn current_input_mut(&mut self) -> &mut Input {
        let layout = self.field_layout();
        match self.focused_field {
            n if n == layout.title => &mut self.title,
            n if n == layout.worktree => &mut self.primary.worktree_branch,
            n if n == layout.right_pane_worktree => {
                &mut self
                    .secondary
                    .as_mut()
                    .expect("visible secondary field")
                    .worktree_branch
            }
            n if n == layout.group => &mut self.group,
            _ => &mut self.title,
        }
    }

    fn check_worktree_exists(&self, target: PaneTarget) -> Option<String> {
        use crate::git::GitWorktree;

        let pane = self.pane(target)?;
        let branch = pane.worktree_branch.value().trim();
        if branch.is_empty() {
            return None;
        }

        let pane_path = pane.path.resolved();
        let source_path = if target == PaneTarget::Secondary && pane_path.trim().is_empty() {
            self.primary.path.resolved()
        } else {
            pane_path
        };
        let path = std::path::PathBuf::from(source_path);

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

    fn unconfirmed_worktrees(&self) -> Vec<(PaneTarget, String)> {
        [PaneTarget::Primary, PaneTarget::Secondary]
            .into_iter()
            .filter_map(|target| {
                let pane = self.pane(target)?;
                if pane.confirm_reuse_worktree {
                    return None;
                }
                self.check_worktree_exists(target)
                    .map(|path| (target, path))
            })
            .collect()
    }

    fn pane_to_draft(&self, target: PaneTarget) -> Option<PaneDraft> {
        let pane = self.pane(target)?;
        let branch = pane.worktree_branch.value().trim();
        let has_worktree = !branch.is_empty();
        Some(PaneDraft {
            tool: self.pane_tool(target)?.to_string(),
            path: pane.path.resolved(),
            yolo_mode: pane.yolo_mode && self.pane_has_yolo(target)
                || self.pane_tool_always_yolo(target),
            cross_agent_team: pane.cross_agent_team && self.pane_has_cross_agent_team(target),
            worktree: PaneWorktreeRequest {
                branch: has_worktree.then(|| branch.to_string()),
                create_new_branch: pane.create_new_branch,
                extra_repo_paths: if has_worktree {
                    pane.workspace_repos.clone()
                } else {
                    Vec::new()
                },
                reuse_existing: pane.confirm_reuse_worktree,
            },
        })
    }

    fn build_submit_result(&self) -> DialogResult<NewSessionData> {
        let title_value = self.title.value().trim();
        let final_title = if title_value.is_empty() {
            let refs: Vec<&str> = self.existing_titles.iter().map(|s| s.as_str()).collect();
            civilizations::generate_random_title(&refs)
        } else {
            title_value.to_string()
        };
        DialogResult::Submit(NewSessionData {
            profile: self.profile.clone(),
            title: final_title,
            group: self.group.value().trim().to_string(),
            primary: self
                .pane_to_draft(PaneTarget::Primary)
                .expect("primary pane"),
            secondary: self.pane_to_draft(PaneTarget::Secondary),
            cross_agent_team_channel: self.cross_agent_team_channel.clone(),
            extra_args: self.extra_args.value().trim().to_string(),
            command_override: self.command_override.value().trim().to_string(),
        })
    }

    /// Every directory this submit would have to create, in field order.
    fn missing_directories(&self) -> Vec<String> {
        let mut missing = Vec::new();
        let session_path = self.primary.path.resolved();
        if !std::path::Path::new(&session_path).exists() {
            missing.push(session_path);
        }
        if let Some(secondary) = self.secondary.as_ref() {
            let right_pane_path = secondary.path.resolved();
            if right_pane_path.is_empty() {
                return missing;
            }
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
            (layout.path, Some(self.primary.path.resolved())),
            (
                layout.right_pane_path,
                self.secondary
                    .as_ref()
                    .map(|pane| pane.path.resolved())
                    .filter(|path| !path.is_empty()),
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
        if layout.right_pane_path != ABSENT
            && self
                .secondary
                .as_ref()
                .is_some_and(|pane| pane.path.resolved() == dir)
        {
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
            self.primary.path.clear_ghost();
        } else if self.focused_field == layout.right_pane_path {
            if let Some(pane) = self.secondary.as_mut() {
                pane.path.clear_ghost();
            }
        } else if self.focused_field == layout.group {
            self.clear_group_ghost();
        }
    }

    fn recompute_focused_ghost(&mut self) {
        let layout = self.field_layout();
        if self.focused_field == layout.path {
            self.primary.path.recompute_ghost();
        } else if self.focused_field == layout.right_pane_path {
            if let Some(pane) = self.secondary.as_mut() {
                pane.path.recompute_ghost();
            }
        } else if self.focused_field == layout.group {
            self.recompute_group_ghost();
        }
    }
}
