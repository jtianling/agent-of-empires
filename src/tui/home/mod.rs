//! Home view - main session list and navigation

mod input;
mod operations;
mod render;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tui_input::Input;

use crate::session::{
    config::{load_config, save_config, SortOrder},
    flatten_tree, resolve_config, Group, GroupTree, Instance, Item, Storage,
};
use crate::tmux::AvailableTools;

use super::creation_poller::{CreationPoller, CreationRequest};
use super::deletion_poller::DeletionPoller;
use super::dialogs::{
    ChangelogDialog, ConfirmDialog, GroupDeleteOptionsDialog, HookTrustDialog, InfoDialog,
    NewSessionData, NewSessionDialog, ProfilePickerDialog, RenameDialog, UnifiedDeleteDialog,
    WelcomeDialog,
};
use super::diff::DiffView;
use super::settings::SettingsView;
use super::status_poller::StatusPoller;

/// Cached preview content to avoid subprocess calls on every frame
pub(super) struct PreviewCache {
    pub(super) session_id: Option<String>,
    pub(super) content: String,
    pub(super) last_refresh: Instant,
    pub(super) dimensions: (u16, u16),
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self {
            session_id: None,
            content: String::new(),
            last_refresh: Instant::now(),
            dimensions: (0, 0),
        }
    }
}

pub(super) const INDENTS: [&str; 10] = [
    "",
    "  ",
    "    ",
    "      ",
    "        ",
    "          ",
    "            ",
    "              ",
    "                ",
    "                  ",
];

pub(super) fn get_indent(depth: usize) -> &'static str {
    INDENTS.get(depth).copied().unwrap_or(INDENTS[9])
}

pub(super) const ICON_RUNNING: &str = "●";
pub(super) const ICON_WAITING: &str = "◐";
pub(super) const ICON_IDLE: &str = "○";
pub(super) const ICON_ERROR: &str = "✕";
pub(super) const ICON_STARTING: &str = "◌";
pub(super) const ICON_UNKNOWN: &str = "?";
pub(super) const ICON_STOPPED: &str = "■";
pub(super) const ICON_DELETING: &str = "✗";
pub(super) const ICON_COLLAPSED: &str = "▶";
pub(super) const ICON_EXPANDED: &str = "▼";

pub(super) struct PendingJump {
    pub(super) first_digit: u8,
}

pub struct HomeView {
    pub(super) storage: Storage,
    pub(super) instances: Vec<Instance>,
    pub(super) instance_map: HashMap<String, Instance>,
    pub(super) groups: Vec<Group>,
    pub(super) group_tree: GroupTree,
    pub(super) flat_items: Vec<Item>,

    // UI state
    pub(super) cursor: usize,
    pub(super) selected_session: Option<String>,
    pub(super) selected_group: Option<String>,
    pub(super) sort_order: SortOrder,

    // Dialogs
    pub(super) show_help: bool,
    pub(super) new_dialog: Option<NewSessionDialog>,
    pub(super) confirm_dialog: Option<ConfirmDialog>,
    pub(super) unified_delete_dialog: Option<UnifiedDeleteDialog>,
    pub(super) group_delete_options_dialog: Option<GroupDeleteOptionsDialog>,
    pub(super) rename_dialog: Option<RenameDialog>,
    pub(super) hook_trust_dialog: Option<HookTrustDialog>,
    /// Session data pending hook trust approval
    pub(super) pending_hook_trust_data: Option<NewSessionData>,
    pub(super) welcome_dialog: Option<WelcomeDialog>,
    pub(super) changelog_dialog: Option<ChangelogDialog>,
    pub(super) info_dialog: Option<InfoDialog>,
    pub(super) profile_picker_dialog: Option<ProfilePickerDialog>,
    /// Session to attach after the custom instruction warning dialog is dismissed
    pub(super) pending_attach_after_warning: Option<String>,
    /// Session to stop after the confirmation dialog is accepted
    pub(super) pending_stop_session: Option<String>,
    /// Right pane tool to launch after next session attach (one-shot, consumed on use)
    pub(super) pending_right_pane_tool: Option<String>,

    // Number jump
    pub(super) pending_jump: Option<PendingJump>,

    // Search
    pub(super) search_active: bool,
    pub(super) search_query: Input,
    pub(super) search_matches: Vec<usize>,
    pub(super) search_match_index: usize,

    // Tool availability
    pub(super) available_tools: AvailableTools,

    // Directory where aoe was launched (for new session defaults)
    pub(super) launch_dir: std::path::PathBuf,

    // Performance: background status polling
    pub(super) status_poller: StatusPoller,
    pub(super) pending_status_refresh: bool,

    // Performance: background deletion
    pub(super) deletion_poller: DeletionPoller,

    // Performance: background session creation (for sandbox)
    pub(super) creation_poller: CreationPoller,
    /// Set to true if user cancelled while creation was pending
    pub(super) creation_cancelled: bool,
    /// Sessions whose on_launch hooks already ran in the creation poller
    pub(super) on_launch_hooks_ran: HashSet<String>,

    // Performance: preview caching
    pub(super) preview_cache: PreviewCache,

    // Sound config for state transition sounds
    pub(super) sound_config: crate::sound::SoundConfig,

    // Settings view
    pub(super) settings_view: Option<SettingsView>,
    /// Flag to indicate we're confirming settings close (unsaved changes)
    pub(super) settings_close_confirm: bool,

    // Diff view
    pub(super) diff_view: Option<DiffView>,

    // Resizable list column width (percentage-like units)
    pub(super) list_width: u16,

    /// Flag to indicate that a refresh is needed due to internal state changes (e.g., preview update)
    pub(super) needs_redraw: bool,
}

impl HomeView {
    /// Check if an internal redraw was requested (e.g., from render logic)
    pub fn check_redraw(&mut self) -> bool {
        let redraw = self.needs_redraw;
        self.needs_redraw = false;
        redraw
    }

    /// Update preview caches for the currently selected session.
    /// This should be called before rendering to ensure all data is pre-fetched.
    pub fn update_caches(&mut self, width: u16, height: u16) {
        let inner_width = width.saturating_sub(2);
        let inner_height = height.saturating_sub(2);

        if self.refresh_preview_cache_if_needed(inner_width, inner_height) {
            self.needs_redraw = true;
        }
    }

    pub fn new(
        storage: Storage,
        available_tools: AvailableTools,
        launch_dir: std::path::PathBuf,
    ) -> anyhow::Result<Self> {
        let (instances, groups) = storage.load_with_groups()?;

        let instance_map: HashMap<String, Instance> = instances
            .iter()
            .map(|i| (i.id.clone(), i.clone()))
            .collect();
        let group_tree = GroupTree::new_with_groups(&instances, &groups);

        // Load the resolved config to get sound config and sort order
        let resolved = resolve_config(storage.profile());
        let sound_config = resolved
            .as_ref()
            .map(|config| config.sound.clone())
            .unwrap_or_default();
        let user_config = load_config().ok().flatten();
        let sort_order = user_config
            .as_ref()
            .and_then(|c| c.app_state.sort_order)
            .unwrap_or_default();

        let flat_items = flatten_tree(&group_tree, &instances, sort_order);

        let mut view = Self {
            storage,
            instances,
            instance_map,
            groups,
            group_tree,
            flat_items,
            cursor: 0,
            selected_session: None,
            selected_group: None,
            sort_order,
            show_help: false,
            new_dialog: None,
            confirm_dialog: None,
            unified_delete_dialog: None,
            group_delete_options_dialog: None,
            rename_dialog: None,
            hook_trust_dialog: None,
            pending_hook_trust_data: None,
            welcome_dialog: None,
            changelog_dialog: None,
            info_dialog: None,
            profile_picker_dialog: None,
            pending_attach_after_warning: None,
            pending_stop_session: None,
            pending_right_pane_tool: None,
            pending_jump: None,
            search_active: false,
            search_query: Input::default(),
            search_matches: Vec::new(),
            search_match_index: 0,
            available_tools,
            launch_dir,
            status_poller: StatusPoller::new(),
            pending_status_refresh: false,
            deletion_poller: DeletionPoller::new(),
            creation_poller: CreationPoller::new(),
            creation_cancelled: false,
            on_launch_hooks_ran: HashSet::new(),
            preview_cache: PreviewCache::default(),
            sound_config,
            settings_view: None,
            settings_close_confirm: false,
            diff_view: None,
            list_width: user_config
                .and_then(|c| c.app_state.home_list_width)
                .unwrap_or(35),
            needs_redraw: false,
        };

        view.update_selected();
        Ok(view)
    }

    pub fn reload(&mut self) -> anyhow::Result<()> {
        let (mut instances, groups) = self.storage.load_with_groups()?;

        for inst in &mut instances {
            if let Some(prev) = self.instance_map.get(&inst.id) {
                inst.status = prev.status;
                inst.last_error = prev.last_error.clone();
                inst.last_error_check = prev.last_error_check;
                inst.last_start_time = prev.last_start_time;
            }
        }

        self.instances = instances;
        self.instance_map = self
            .instances
            .iter()
            .map(|i| (i.id.clone(), i.clone()))
            .collect();
        self.groups = groups;
        self.group_tree = GroupTree::new_with_groups(&self.instances, &self.groups);
        self.flat_items = flatten_tree(&self.group_tree, &self.instances, self.sort_order);

        if self.cursor >= self.flat_items.len() && !self.flat_items.is_empty() {
            self.cursor = self.flat_items.len() - 1;
        }

        if self.search_active && !self.search_query.value().is_empty() {
            self.update_search();
        } else if !self.search_matches.is_empty() {
            // Recalculate match indices without moving the cursor
            self.refresh_search_matches();
        }

        self.update_selected();
        Ok(())
    }

    /// Request a status refresh in the background (non-blocking).
    /// Call `apply_status_updates` to check for and apply results.
    pub fn request_status_refresh(&mut self) {
        if !self.pending_status_refresh {
            let instances: Vec<Instance> = self.instances.clone();
            self.status_poller.request_refresh(instances);
            self.pending_status_refresh = true;
        }
    }

    /// Apply any pending status updates from the background poller.
    /// Returns true if updates were applied.
    pub fn apply_status_updates(&mut self) -> bool {
        use crate::session::Status;

        if let Some(updates) = self.status_poller.try_recv_updates() {
            for update in updates {
                let old_status = self.get_instance(&update.id).map(|i| i.status);

                let should_update = old_status.is_some_and(|s| {
                    s != Status::Deleting
                        && s != Status::Stopped
                        && update.status != Status::Stopped
                });

                if should_update {
                    let new_status = update.status;
                    let new_error = update.last_error;
                    self.mutate_instance(&update.id, |inst| {
                        inst.status = new_status;
                        inst.last_error = new_error;
                    });

                    if let Some(old) = old_status {
                        if old != new_status {
                            crate::sound::play_for_transition(old, new_status, &self.sound_config);
                        }
                    }
                }
            }
            self.pending_status_refresh = false;
            return true;
        }
        false
    }

    pub fn apply_deletion_results(&mut self) -> bool {
        use crate::session::Status;

        if let Some(result) = self.deletion_poller.try_recv_result() {
            if result.success {
                self.instances.retain(|i| i.id != result.session_id);
                self.instance_map.remove(&result.session_id);
                self.group_tree = GroupTree::new_with_groups(&self.instances, &self.groups);

                if let Err(e) = self.save() {
                    tracing::error!("Failed to save after deletion: {}", e);
                }
                let _ = self.reload();
            } else {
                let error = result.error;
                self.mutate_instance(&result.session_id, |inst| {
                    inst.status = Status::Error;
                    inst.last_error = error;
                });
            }
            return true;
        }
        false
    }

    /// Request background session creation. Used for sandbox sessions to avoid blocking UI.
    pub fn request_creation(
        &mut self,
        data: NewSessionData,
        hooks: Option<crate::session::HooksConfig>,
    ) {
        let has_hooks = hooks
            .as_ref()
            .is_some_and(|h| !h.on_create.is_empty() || !h.on_launch.is_empty());
        if let Some(dialog) = &mut self.new_dialog {
            dialog.set_loading(true);
            dialog.set_has_hooks(has_hooks);
        }

        self.creation_cancelled = false;
        let request = CreationRequest {
            data,
            existing_instances: self.instances.clone(),
            hooks,
        };
        self.creation_poller.request_creation(request);
    }

    /// Mark the current creation operation as cancelled (user pressed Esc)
    pub fn cancel_creation(&mut self) {
        if self.creation_poller.is_pending() {
            self.creation_cancelled = true;
        }
        self.new_dialog = None;
    }

    /// Apply any pending creation results from the background poller.
    /// Returns Some(session_id) if creation succeeded and we should attach.
    pub fn apply_creation_results(&mut self) -> Option<String> {
        use super::creation_poller::CreationResult;
        use crate::session::builder::{self, CreatedWorktree};
        use std::path::PathBuf;

        let result = self.creation_poller.try_recv_result()?;

        // Check if the user cancelled while waiting
        if self.creation_cancelled {
            self.creation_cancelled = false;
            if let CreationResult::Success {
                ref instance,
                ref created_worktree,
                ..
            } = result
            {
                let worktree = created_worktree.as_ref().map(|wt| CreatedWorktree {
                    path: PathBuf::from(&wt.path),
                    main_repo_path: PathBuf::from(&wt.main_repo_path),
                });
                builder::cleanup_instance(instance, worktree.as_ref());
            }
            return None;
        }

        match result {
            CreationResult::Success {
                session_id,
                instance,
                on_launch_hooks_ran,
                ..
            } => {
                let instance = *instance;

                // Check if this was created for a different profile
                let target_profile = self
                    .creation_poller
                    .last_profile()
                    .unwrap_or_else(|| self.storage.profile().to_string());
                let is_cross_profile = target_profile != self.storage.profile();

                if is_cross_profile {
                    // Save to target profile's storage
                    match Storage::new(&target_profile) {
                        Ok(target_storage) => match target_storage.load_with_groups() {
                            Ok((mut target_instances, target_groups)) => {
                                target_instances.push(instance.clone());
                                let mut target_tree =
                                    GroupTree::new_with_groups(&target_instances, &target_groups);
                                if !instance.group_path.is_empty() {
                                    target_tree.create_group(&instance.group_path);
                                }
                                if let Err(e) =
                                    target_storage.save_with_groups(&target_instances, &target_tree)
                                {
                                    tracing::error!("Failed to save to target profile: {}", e);
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to load target profile data: {}", e);
                            }
                        },
                        Err(e) => {
                            tracing::error!("Failed to open target profile storage: {}", e);
                        }
                    }
                } else {
                    self.instances.push(instance.clone());
                    self.group_tree = GroupTree::new_with_groups(&self.instances, &self.groups);
                    if !instance.group_path.is_empty() {
                        self.group_tree.create_group(&instance.group_path);
                    }

                    if let Err(e) = self.save() {
                        tracing::error!("Failed to save after creation: {}", e);
                    }
                }

                if on_launch_hooks_ran {
                    self.on_launch_hooks_ran.insert(session_id.clone());
                }

                let _ = self.reload();
                self.new_dialog = None;

                Some(session_id)
            }
            CreationResult::Error(error) => {
                if let Some(dialog) = &mut self.new_dialog {
                    dialog.set_loading(false);
                    dialog.set_error(error);
                }
                None
            }
        }
    }

    /// Consume the pending right pane tool (one-shot, set during session creation).
    pub fn take_pending_right_pane_tool(&mut self) -> Option<String> {
        self.pending_right_pane_tool.take()
    }

    /// Check if on_launch hooks already ran for this session (and consume the flag).
    pub fn take_on_launch_hooks_ran(&mut self, session_id: &str) -> bool {
        self.on_launch_hooks_ran.remove(session_id)
    }

    /// Check if there's a pending creation operation
    pub fn is_creation_pending(&self) -> bool {
        self.creation_poller.is_pending()
    }

    /// Tick dialog animations/timers and drain hook progress.
    /// Returns true when a redraw is needed.
    pub fn tick_dialog(&mut self) -> bool {
        let mut changed = false;

        if let Some(dialog) = &mut self.new_dialog {
            if dialog.tick() {
                changed = true;
            }

            if dialog.is_loading() {
                // Drain all pending hook progress messages
                while let Some(progress) = self.creation_poller.try_recv_progress() {
                    dialog.push_hook_progress(progress);
                    changed = true;
                }
            }
        }

        changed
    }

    pub fn has_dialog(&self) -> bool {
        self.show_help
            || self.new_dialog.is_some()
            || self.confirm_dialog.is_some()
            || self.unified_delete_dialog.is_some()
            || self.group_delete_options_dialog.is_some()
            || self.rename_dialog.is_some()
            || self.hook_trust_dialog.is_some()
            || self.welcome_dialog.is_some()
            || self.changelog_dialog.is_some()
            || self.info_dialog.is_some()
            || self.profile_picker_dialog.is_some()
            || self.settings_view.is_some()
            || self.diff_view.is_some()
    }

    pub fn shrink_list(&mut self) {
        self.list_width = self.list_width.saturating_sub(5).max(10);
        self.save_list_width();
    }

    pub fn grow_list(&mut self) {
        self.list_width = (self.list_width + 5).min(80);
        self.save_list_width();
    }

    fn save_list_width(&self) {
        if let Ok(mut config) = load_config().map(|c| c.unwrap_or_default()) {
            config.app_state.home_list_width = Some(self.list_width);
            let _ = save_config(&config);
        }
    }

    pub fn show_welcome(&mut self) {
        self.welcome_dialog = Some(WelcomeDialog::new());
    }

    pub fn show_changelog(&mut self, from_version: Option<String>) {
        self.changelog_dialog = Some(ChangelogDialog::new(from_version));
    }

    pub fn get_instance(&self, id: &str) -> Option<&Instance> {
        self.instance_map.get(id)
    }

    pub fn available_tools(&self) -> AvailableTools {
        self.available_tools.clone()
    }

    /// Show the profile picker dialog with fresh data from disk.
    pub(super) fn show_profile_picker(&mut self) {
        use crate::session::list_profiles;
        use crate::tui::dialogs::{ProfileEntry, ProfilePickerDialog};

        let current_profile = self.storage.profile().to_string();
        let profiles = list_profiles().unwrap_or_else(|_| vec![current_profile.clone()]);
        let entries: Vec<ProfileEntry> = profiles
            .iter()
            .map(|name| {
                let session_count = Storage::new(name)
                    .and_then(|s| s.load())
                    .map(|instances| instances.len())
                    .unwrap_or(0);
                ProfileEntry {
                    name: name.clone(),
                    session_count,
                    is_active: name == &current_profile,
                }
            })
            .collect();
        self.profile_picker_dialog = Some(ProfilePickerDialog::new(entries, &current_profile));
    }

    pub fn set_instance_status(&mut self, id: &str, status: crate::session::Status) {
        self.mutate_instance(id, |inst| inst.status = status);
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.storage
            .save_with_groups(&self.instances, &self.group_tree)?;
        Ok(())
    }

    pub(super) fn selected_group_context(&self) -> Option<String> {
        if let Some(group_path) = &self.selected_group {
            return Some(group_path.clone());
        }

        let session_id = self.selected_session.as_ref()?;
        let instance = self.get_instance(session_id)?;
        if instance.group_path.is_empty() {
            None
        } else {
            Some(instance.group_path.clone())
        }
    }

    pub(super) fn move_selected_manual_item(&mut self, delta: i32) {
        if self.sort_order != SortOrder::Manual || delta == 0 {
            return;
        }

        let moved = if let Some(session_id) = self.selected_session.clone() {
            self.move_session_within_group(&session_id, delta)
        } else if let Some(group_path) = self.selected_group.clone() {
            self.group_tree.move_group(&group_path, delta)
        } else {
            false
        };

        if !moved {
            return;
        }

        self.rebuild_flat_items_preserve_selection();
        if let Err(e) = self.save() {
            tracing::error!("Failed to save manual ordering: {}", e);
        }
    }

    fn move_session_within_group(&mut self, session_id: &str, delta: i32) -> bool {
        if delta == 0 {
            return false;
        }

        let current_idx = match self.instances.iter().position(|inst| inst.id == session_id) {
            Some(idx) => idx,
            None => return false,
        };
        let group_path = self.instances[current_idx].group_path.clone();
        let sibling_indices: Vec<usize> = self
            .instances
            .iter()
            .enumerate()
            .filter(|(_, inst)| inst.group_path == group_path)
            .map(|(idx, _)| idx)
            .collect();

        let sibling_pos = match sibling_indices.iter().position(|idx| *idx == current_idx) {
            Some(idx) => idx,
            None => return false,
        };
        let target_pos = if delta < 0 {
            match sibling_pos.checked_sub((-delta) as usize) {
                Some(idx) => idx,
                None => return false,
            }
        } else {
            let idx = sibling_pos + delta as usize;
            if idx >= sibling_indices.len() {
                return false;
            }
            idx
        };

        let target_idx = sibling_indices[target_pos];
        let instance = self.instances.remove(current_idx);
        self.instances.insert(target_idx, instance);
        true
    }

    fn rebuild_flat_items_preserve_selection(&mut self) {
        let selected_session = self.selected_session.clone();
        let selected_group = self.selected_group.clone();

        self.instance_map = self
            .instances
            .iter()
            .map(|i| (i.id.clone(), i.clone()))
            .collect();
        self.groups = self.group_tree.get_all_groups();
        self.flat_items = flatten_tree(&self.group_tree, &self.instances, self.sort_order);

        if self.search_active && !self.search_query.value().is_empty() {
            self.update_search();
        } else if !self.search_matches.is_empty() {
            self.refresh_search_matches();
        }

        if let Some(session_id) = selected_session {
            self.select_session_by_id(&session_id);
            if self.selected_session.as_deref() == Some(session_id.as_str()) {
                return;
            }
        }

        if let Some(group_path) = selected_group {
            self.select_group_by_path(&group_path);
            if self.selected_group.as_deref() == Some(group_path.as_str()) {
                return;
            }
        }

        if self.cursor >= self.flat_items.len() && !self.flat_items.is_empty() {
            self.cursor = self.flat_items.len() - 1;
        }
        self.update_selected();
    }

    /// Centralized instance mutation: applies `f` once to the `instances` vec
    /// entry, then clones the result into `instance_map`. This guarantees both
    /// collections stay in sync even for non-idempotent closures.
    pub(super) fn mutate_instance(&mut self, id: &str, f: impl FnOnce(&mut Instance)) {
        if let Some(inst) = self.instances.iter_mut().find(|i| i.id == id) {
            f(inst);
            self.instance_map.insert(id.to_string(), inst.clone());
        }
    }

    pub fn set_instance_error(&mut self, id: &str, error: Option<String>) {
        self.mutate_instance(id, |inst| inst.last_error = error);
    }

    pub fn select_session_by_id(&mut self, session_id: &str) {
        for (idx, item) in self.flat_items.iter().enumerate() {
            if let Item::Session { id, .. } = item {
                if id == session_id {
                    self.cursor = idx;
                    self.update_selected();
                    return;
                }
            }
        }
    }

    pub fn select_group_by_path(&mut self, group_path: &str) {
        for (idx, item) in self.flat_items.iter().enumerate() {
            if let Item::Group { path, .. } = item {
                if path == group_path {
                    self.cursor = idx;
                    self.update_selected();
                    return;
                }
            }
        }
    }

    pub fn select_session_by_managed_tmux_name(&mut self, tmux_session_name: &str) -> bool {
        for instance in &self.instances {
            if crate::tmux::Session::generate_name(&instance.id, &instance.title)
                == tmux_session_name
            {
                let session_id = instance.id.clone();
                self.select_session_by_id(&session_id);
                return self.selected_session.as_deref() == Some(session_id.as_str());
            }
        }

        false
    }

    /// Refresh all config-dependent state from the current profile's config.
    /// Call this after settings are saved to pick up any changes.
    pub fn refresh_from_config(&mut self) {
        if let Ok(config) = resolve_config(self.storage.profile()) {
            self.sound_config = config.sound.clone();
        }
    }
}
