//! Main TUI application

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

use super::home::HomeView;
use super::styles::load_theme;
use super::styles::Theme;
use super::tab_title;
use crate::session::{get_update_settings, load_config, save_config, Storage};
use crate::tmux::AvailableTools;
use crate::update::{check_for_update, UpdateInfo};

/// Temporarily leave TUI mode, run a closure, and restore TUI mode.
/// Drains stale events and clears the terminal on return.
fn with_raw_mode_disabled<F, R>(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    f: F,
) -> Result<R>
where
    F: FnOnce() -> R,
{
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::cursor::Show
    )?;
    std::io::Write::flush(terminal.backend_mut())?;

    let result = f();

    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::cursor::Hide
    )?;
    std::io::Write::flush(terminal.backend_mut())?;

    while event::poll(Duration::from_millis(0))? {
        let _ = event::read();
    }

    Ok(result)
}

fn reapply_tui_title(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>, profile: &str) {
    let _ = tab_title::set_tui_title(terminal.backend_mut(), profile);
}

fn skipped_slot_warning(skipped: usize) -> Option<String> {
    (skipped > 0).then(|| {
        format!("Skipped {skipped} invalid tracked pane(s); repair their stored pane configuration")
    })
}

fn combine_pane_errors(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
        (Some(error), None) | (None, Some(error)) => Some(error),
        (None, None) => None,
    }
}

pub struct App {
    home: HomeView,
    should_quit: bool,
    theme: Theme,
    needs_redraw: bool,
    update_info: Option<UpdateInfo>,
    update_rx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<UpdateInfo>>>,
    launch_dir: PathBuf,
    session_before_tui: Option<String>,
    last_attach_client: Option<String>,
    /// Last time a redraw was triggered by a tick event (to throttle animations)
    last_tick_redraw: std::time::Instant,
}

/// Check if the app version changed and return the previous version if changelog should be shown.
/// This is called before App::new to allow async cache refresh.
pub fn check_version_change() -> Result<Option<String>> {
    let config = load_config()?.unwrap_or_default();
    let current_version = env!("CARGO_PKG_VERSION");

    if config.app_state.has_seen_welcome
        && config.app_state.last_seen_version.as_deref() != Some(current_version)
    {
        Ok(config.app_state.last_seen_version)
    } else {
        Ok(None)
    }
}

impl App {
    pub fn new(
        profile: &str,
        available_tools: AvailableTools,
        launch_dir: PathBuf,
    ) -> Result<Self> {
        let storage = Storage::new(profile)?;
        let mut home = HomeView::new(storage, available_tools, launch_dir.clone())?;

        // Check if we need to show welcome or changelog dialogs
        let mut config = load_config()?.unwrap_or_default();

        // Load theme from config, defaulting to empire if empty
        let theme_name = if config.theme.name.is_empty() {
            "empire"
        } else {
            &config.theme.name
        };
        let theme = load_theme(theme_name);
        let current_version = env!("CARGO_PKG_VERSION").to_string();

        if !config.app_state.has_seen_welcome {
            home.show_welcome();
            config.app_state.has_seen_welcome = true;
            config.app_state.last_seen_version = Some(current_version);
            save_config(&config)?;
        } else if config.app_state.last_seen_version.as_deref() != Some(&current_version) {
            // Cache should already be refreshed by tui::run() before App::new
            home.show_changelog(config.app_state.last_seen_version.clone());
            config.app_state.last_seen_version = Some(current_version);
            save_config(&config)?;
        }

        if let Err(e) = crate::tmux::notification_monitor::ensure_notification_monitor(profile) {
            tracing::debug!("Failed to ensure notification monitor on startup: {}", e);
        }

        Ok(Self {
            home,
            should_quit: false,
            theme,
            needs_redraw: true,
            update_info: None,
            update_rx: None,
            launch_dir,
            session_before_tui: None,
            last_attach_client: None,
            last_tick_redraw: std::time::Instant::now(),
        })
    }

    pub fn show_startup_warning(&mut self, message: &str) {
        self.home.info_dialog = Some(crate::tui::dialogs::InfoDialog::new("Warning", message));
    }

    pub fn set_theme(&mut self, name: &str) {
        self.theme = load_theme(name);
        self.needs_redraw = true;
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        // Initial render
        terminal.clear()?;
        crossterm::execute!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
        terminal.draw(|f| self.render(f))?;
        crossterm::execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;

        // Refresh tmux session cache
        crate::tmux::refresh_session_cache();

        // Spawn async update check
        let settings = get_update_settings();
        if settings.check_enabled {
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.update_rx = Some(rx);
            tokio::spawn(async move {
                let version = env!("CARGO_PKG_VERSION");
                let _ = tx.send(check_for_update(version, false).await);
            });
        }

        let mut last_status_refresh = std::time::Instant::now();
        let mut last_disk_refresh = std::time::Instant::now();
        const STATUS_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
        const DISK_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

        loop {
            let mut refresh_needed = false;

            // Force full redraw if needed (e.g., after returning from tmux)
            if self.needs_redraw {
                terminal.clear()?;
                self.needs_redraw = false;
                refresh_needed = true;
            }

            // Poll with short timeout for responsive input
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) => {
                        self.handle_key(key, terminal).await?;
                        refresh_needed = true;

                        if self.should_quit {
                            break;
                        }
                    }
                    Event::Mouse(mouse) => {
                        self.handle_mouse(mouse, terminal).await?;
                        refresh_needed = true;
                    }
                    _ => {}
                }
            }

            // Check for update result (non-blocking)
            if self.poll_update_check() {
                refresh_needed = true;
            }

            // Periodic refreshes (only when no input pending)

            // Request status refresh every interval (non-blocking)
            if last_status_refresh.elapsed() >= STATUS_REFRESH_INTERVAL {
                self.home.request_status_refresh();
                last_status_refresh = std::time::Instant::now();
            }

            // Always check for and apply status updates (non-blocking)
            if self.home.apply_status_updates() {
                refresh_needed = true;
            }

            // Check for and apply deletion results (non-blocking)
            if self.home.apply_deletion_results() {
                refresh_needed = true;
            }

            // Check for and apply creation results (non-blocking)
            if let Some(session_id) = self.home.apply_creation_results() {
                // Creation succeeded - attach to the new session
                self.attach_session(&session_id, terminal)?;
                refresh_needed = true;
            }

            if self.try_restore_selection_from_client_context() {
                refresh_needed = true;
            }

            // Tick dialog animations/timers (spinner, transient flashes)
            if self.home.tick_dialog() {
                // Throttle animation redraws to ~10Hz to prevent flicker in tmux
                if self.last_tick_redraw.elapsed() >= Duration::from_millis(100) {
                    refresh_needed = true;
                }
            }

            // Check for internal redraw requests (e.g., from preview refresh during render)
            if self.home.check_redraw() {
                refresh_needed = true;
            }

            // Periodic disk refresh to sync with other instances
            if last_disk_refresh.elapsed() >= DISK_REFRESH_INTERVAL {
                self.home.reload()?;
                last_disk_refresh = std::time::Instant::now();
                refresh_needed = true;
            }

            // Single draw after all refreshes to avoid flicker
            if refresh_needed {
                // Pre-calculate layout to get preview dimensions for cache refresh
                let size = terminal.size()?;
                let area = Rect::new(0, 0, size.width, size.height);

                // This mimics the constraints in render.rs to get the same preview area
                let main_constraints = if self.update_info.is_some() {
                    vec![
                        Constraint::Min(0),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ]
                } else {
                    vec![Constraint::Min(0), Constraint::Length(1)]
                };
                let main_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(main_constraints)
                    .split(area);
                if !self.home.is_narrow_layout(main_chunks[0].width) {
                    let effective_list_width = self.home.effective_list_width(main_chunks[0].width);
                    let chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(effective_list_width),
                            Constraint::Min(40),
                        ])
                        .split(main_chunks[0]);

                    let preview_area = chunks[1];

                    // Settle all state (including tmux captures) BEFORE drawing
                    self.home
                        .update_caches(preview_area.width, preview_area.height);
                }

                crossterm::execute!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
                terminal.draw(|f| self.render(f))?;
                crossterm::execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;

                // Reset animation timer if this was a tick-induced redraw
                self.last_tick_redraw = std::time::Instant::now();
            }

            if self.should_quit {
                break;
            }
        }

        if let Err(e) = self.home.save() {
            tracing::error!("Failed to save on quit: {}", e);
        }

        Ok(())
    }

    fn render(&mut self, frame: &mut Frame) {
        self.home
            .render(frame, frame.area(), &self.theme, self.update_info.as_ref());
    }

    /// Poll for update check result (non-blocking).
    /// Returns true if an update is available and was just received.
    fn poll_update_check(&mut self) -> bool {
        let (update_info, update_rx, received) =
            poll_update_receiver(self.update_rx.take(), self.update_info.take());
        self.update_info = update_info;
        self.update_rx = update_rx;
        received
    }
}

/// Polls the update receiver and returns the new state.
/// Returns (update_info, update_rx, was_update_received).
fn poll_update_receiver(
    rx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<UpdateInfo>>>,
    current_info: Option<UpdateInfo>,
) -> (
    Option<UpdateInfo>,
    Option<tokio::sync::oneshot::Receiver<anyhow::Result<UpdateInfo>>>,
    bool,
) {
    if let Some(mut rx) = rx {
        match rx.try_recv() {
            Ok(result) => {
                if let Ok(info) = result {
                    if info.available {
                        return (Some(info), None, true);
                    }
                }
                (current_info, None, false)
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                (current_info, Some(rx), false)
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => (current_info, None, false),
        }
    } else {
        (current_info, None, false)
    }
}

impl App {
    async fn handle_key(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        // Global keybindings
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
            | (KeyCode::Char('q'), KeyModifiers::NONE)
                if !self.home.has_dialog() =>
            {
                self.should_quit = true;
                return Ok(());
            }
            _ => {}
        }

        if let Some(action) = self.home.handle_key(key) {
            self.execute_action(action, terminal)?;
        }

        Ok(())
    }

    async fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        if let Some(action) = self.home.handle_mouse(mouse) {
            self.execute_action(action, terminal)?;
        }

        Ok(())
    }

    fn execute_action(
        &mut self,
        action: Action,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        match action {
            Action::Quit => self.should_quit = true,
            Action::AttachSession(id) => {
                self.attach_session(&id, terminal)?;
            }
            Action::AddAgentPane(id) => {
                self.add_agent_pane(&id, terminal)?;
            }
            Action::SwitchProfile(profile) => {
                let storage = Storage::new(&profile)?;
                let tools = self.home.available_tools();
                self.home = HomeView::new(storage, tools, self.launch_dir.clone())?;
            }
            Action::EditFile(path) => {
                self.edit_file(&path, terminal)?;
            }
            Action::RespawnAgentPane(id, mode, post) => {
                if let Some(inst) = self.home.get_instance(&id).cloned() {
                    // Ignore a second R/r while a multi-pane restart is in flight.
                    if inst.restart_in_flight {
                        return Ok(());
                    }

                    let tmux_session = match inst.tmux_session() {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!("Failed to get tmux session: {}", e);
                            return Ok(());
                        }
                    };

                    if !tmux_session.exists() {
                        return match post {
                            PostRestart::Attach => self.attach_session(&id, terminal),
                            // Nothing to respawn: bring the session up the normal
                            // way, but leave the user on the list.
                            PostRestart::StayOnHome => self.start_session(&id, post, terminal),
                        };
                    }

                    let profile = self.home.storage.profile().to_string();
                    // Distinguish "no tracked panes" from "could not read the
                    // store". A read failure is not an empty slot set: degrade to
                    // a primary-pane restart but surface the failure instead of
                    // silently narrowing the restart scope.
                    let (slots, slot_read_error) =
                        match crate::db::Store::open_with_schema(&profile)
                            .and_then(|store| store.read_slots_for_instance_with_diagnostics(&id))
                        {
                            Ok(read) => {
                                let warning = skipped_slot_warning(read.skipped);
                                (read.slots, warning)
                            }
                            Err(e) => {
                                tracing::error!("Failed to read agent slots for '{}': {}", id, e);
                                (
                                    Vec::new(),
                                    Some(format!(
                                    "Could not read tracked panes: {e}; restarted primary pane only"
                                )),
                                )
                            }
                        };

                    if slots.is_empty() {
                        // No tracked panes (or unreadable store): restart the
                        // primary @aoe_agent_pane with the single-pane behavior.
                        let mut respawn_result = Ok(());
                        self.home.mutate_instance(&id, |inst| {
                            respawn_result = match mode {
                                crate::session::RestartMode::Resume => inst.respawn_agent_pane(),
                                crate::session::RestartMode::Fresh => {
                                    inst.respawn_agent_pane_fresh()
                                }
                            };
                        });

                        if let Err(e) = respawn_result {
                            tracing::error!("Failed to respawn agent pane: {}", e);
                            self.home.set_instance_error(&id, Some(e.to_string()));
                            self.home
                                .set_instance_status(&id, crate::session::Status::Error);
                            return Ok(());
                        }
                        self.home.set_instance_error(&id, slot_read_error);
                    } else {
                        // Fan out to every tracked pane, each resumed from its
                        // own persisted native_session_id. Per-pane failures are
                        // recorded but do not abort sibling restarts.
                        let mut slots = slots;
                        let mut identity_origins = std::collections::HashMap::new();
                        if let (Some(inst), Ok(store)) = (
                            self.home.get_instance(&id),
                            crate::db::Store::open_with_schema(&profile),
                        ) {
                            identity_origins = inst.ensure_slot_identity_keys(&store, &mut slots);
                        }

                        let mut outcomes = Vec::new();
                        self.home.mutate_instance(&id, |inst| {
                            inst.restart_in_flight = true;
                            outcomes =
                                inst.resume_all_tracked_panes(&slots, mode, &identity_origins);
                            inst.restart_in_flight = false;
                        });

                        let errors: Vec<String> = outcomes
                            .iter()
                            .filter_map(|o| match o {
                                crate::session::PaneResumeOutcome::Error(e) => Some(e.clone()),
                                _ => None,
                            })
                            .collect();
                        let restart_error = (!errors.is_empty()).then(|| {
                            format!(
                                "{} pane(s) failed to restart: {}",
                                errors.len(),
                                errors.join("; ")
                            )
                        });
                        self.home.set_instance_error(
                            &id,
                            combine_pane_errors(slot_read_error, restart_error),
                        );
                    }

                    if let Err(err) = self.home.save() {
                        tracing::error!("Failed to save after respawning agent pane: {}", err);
                    }
                    crate::tmux::refresh_session_cache();

                    match post {
                        // Auto-attach so the user sees the restarted agent immediately
                        PostRestart::Attach => self.attach_session(&id, terminal)?,
                        PostRestart::StayOnHome => self.needs_redraw = true,
                    }
                }
            }
            Action::RecoverInstance(id, mode, post) => {
                self.recover_instance(&id, mode, post, terminal)?;
            }
            Action::StopSession(id) => {
                if let Some(inst) = self.home.get_instance(&id) {
                    let inst_clone = inst.clone();
                    // Set Stopped immediately so the status poller won't
                    // override to Error while stop() blocks (docker stop
                    // can take up to 10s).
                    self.home
                        .set_instance_status(&id, crate::session::Status::Stopped);
                    match inst_clone.stop() {
                        Ok(()) => {
                            crate::tmux::refresh_session_cache();
                            self.home.reload()?;
                            self.home
                                .set_instance_status(&id, crate::session::Status::Stopped);
                            self.home.save()?;
                        }
                        Err(e) => {
                            tracing::error!("Failed to stop session: {}", e);
                            self.home.set_instance_error(&id, Some(e.to_string()));
                            self.home
                                .set_instance_status(&id, crate::session::Status::Error);
                            self.home.save()?;
                        }
                    }
                }
            }
            Action::SetTheme(name) => {
                self.set_theme(&name);
            }
        }
        Ok(())
    }

    /// Cold-start recover a single focused instance: rebuild its tmux session
    /// from persisted slots and launch each pane in `mode` (resume its
    /// conversation, or start it clean). Recoverability is re-checked at action
    /// time against the store and live tmux, so a no-longer-recoverable (or
    /// now-alive) instance is a silent no-op.
    fn recover_instance(
        &mut self,
        id: &str,
        mode: crate::session::RestartMode,
        post: PostRestart,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let Some(inst) = self.home.get_instance(id).cloned() else {
            return Ok(());
        };

        let profile = self.home.storage.profile().to_string();
        let store = match crate::db::Store::open_with_schema(&profile) {
            Ok(store) => store,
            Err(e) => {
                tracing::error!("Failed to open store for recovery of '{}': {}", id, e);
                self.home.set_instance_error(id, Some(e.to_string()));
                return Ok(());
            }
        };
        let read = match store.read_slots_for_instance_with_diagnostics(id) {
            Ok(read) => read,
            Err(e) => {
                tracing::error!("Failed to read slots for recovery of '{}': {}", id, e);
                self.home.set_instance_error(id, Some(e.to_string()));
                return Ok(());
            }
        };
        let slot_warning = skipped_slot_warning(read.skipped);
        let slots = read.slots;

        // Re-check recoverability at action time: a live session or an instance
        // with no slots is not recoverable. Invalid tracked slots remain visible.
        if !inst.is_recoverable(!slots.is_empty()) {
            if slot_warning.is_some() {
                self.home.set_instance_error(id, slot_warning);
            }
            return Ok(());
        }

        let mut slots = slots;
        let identity_origins = inst.ensure_slot_identity_keys(&store, &mut slots);

        let mut result = Ok(Vec::new());
        self.home.mutate_instance(id, |inst| {
            result = inst.recover_from_slots(&store, &slots, mode, &identity_origins);
        });

        match result {
            Ok(outcomes) => {
                let errors: Vec<String> = outcomes
                    .iter()
                    .filter_map(|o| match o {
                        crate::session::PaneResumeOutcome::Error(e) => Some(e.clone()),
                        _ => None,
                    })
                    .collect();
                let recovery_error = (!errors.is_empty()).then(|| {
                    format!(
                        "{} pane(s) failed to recover: {}",
                        errors.len(),
                        errors.join("; ")
                    )
                });
                self.home
                    .set_instance_error(id, combine_pane_errors(slot_warning, recovery_error));
            }
            Err(e) => {
                tracing::error!("Failed to recover instance '{}': {}", id, e);
                self.home
                    .set_instance_error(id, combine_pane_errors(slot_warning, Some(e.to_string())));
                self.home
                    .set_instance_status(id, crate::session::Status::Error);
                return Ok(());
            }
        }

        if let Err(err) = self.home.save() {
            tracing::error!("Failed to save after recovery: {}", err);
        }
        crate::tmux::refresh_session_cache();
        self.home.refresh_recoverable_cache();

        match post {
            PostRestart::Attach => self.attach_session(id, terminal),
            PostRestart::StayOnHome => {
                self.needs_redraw = true;
                Ok(())
            }
        }
    }

    fn attach_session(
        &mut self,
        session_id: &str,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        self.start_session(session_id, PostRestart::Attach, terminal)
    }

    /// Add a managed agent pane to a running session, then attach to it.
    ///
    /// Adding a pane is an act of wanting to use it, so this attaches rather
    /// than staying on the home list. The session is not started: the `%` key
    /// promises one more pane, not a whole session, and the home view already
    /// refused a session that is not running before opening the dialog.
    fn add_agent_pane(
        &mut self,
        session_id: &str,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let Some(pending) = self.home.take_pending_right_pane() else {
            return Ok(());
        };
        let Some(inst) = self.home.get_instance(session_id).cloned() else {
            return Ok(());
        };

        // The cap was checked before the dialog opened, but the dialog is modal
        // only to this TUI: `aoe session add-agent-pane` can take the last slot
        // while it is up. Splitting anyway would leave a fifth pane that no slot
        // can hold, live but untracked.
        let session_name = crate::tmux::Session::generate_name(&inst.id, &inst.title);
        let panes = crate::db::reconcile::session_pane_ids(&session_name);
        if panes.len() >= crate::db::reconcile::MAX_AGENT_PANES {
            self.report_pane_not_created(&format!(
                "'{}' already has {} panes (max {}).",
                inst.title,
                panes.len(),
                crate::db::reconcile::MAX_AGENT_PANES
            ));
            return Ok(());
        }

        // Attaching after a failed add would drop the user into a session that
        // gained nothing, with the reason left behind on the home screen.
        if !self.launch_managed_pane(&inst, &pending) {
            return Ok(());
        }
        crate::tmux::refresh_session_cache();
        self.attach_session(session_id, terminal)
    }

    /// Bring a session's panes up if they are not already, then either attach to
    /// it or leave the user on the home list, depending on `post`.
    fn start_session(
        &mut self,
        session_id: &str,
        post: PostRestart,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let instance = match self.home.get_instance(session_id) {
            Some(inst) => inst.clone(),
            None => return Ok(()),
        };

        // Refuse to attach to the session AoE is itself running inside: tmux
        // would nest a client into our own pane (the infinite re-enter loop).
        if post == PostRestart::Attach
            && crate::tmux::is_host_session(&crate::tmux::Session::generate_name(
                &instance.id,
                &instance.title,
            ))
        {
            self.home.info_dialog = Some(crate::tui::dialogs::InfoDialog::new(
                "Already in this session",
                "AoE is running inside this tmux session, so it can't attach to itself. \
                 Detach (Ctrl+b d) or pick another session.",
            ));
            return Ok(());
        }

        let tmux_session = match instance.tmux_session() {
            Ok(session) => session,
            Err(error) => {
                self.home.take_pending_right_pane();
                return Err(error);
            }
        };

        // Determine whether the agent pane needs to be (re)started. Attaching
        // must preserve whatever is currently in the pane -- a running agent,
        // or the shell the pane-died hook dropped into after the agent exited
        // -- so only a missing session or a truly dead pane triggers a restart.
        let is_starting = matches!(
            instance.status,
            crate::session::Status::Starting | crate::session::Status::Restarting
        );
        let session_exists = tmux_session.exists();
        let multi_pane = session_exists && tmux_session.pane_count() > 1;
        let needs_restart = !is_starting && (!session_exists || tmux_session.is_pane_dead());

        if needs_restart {
            if multi_pane {
                // Respawn only the agent pane, preserving user-created panes and layout
                self.home
                    .set_instance_status(session_id, crate::session::Status::Starting);
                let mut inst = instance.clone();
                if let Err(e) = inst.respawn_agent_pane() {
                    self.home.take_pending_right_pane();
                    self.home
                        .set_instance_error(session_id, Some(e.to_string()));
                    self.home
                        .set_instance_status(session_id, crate::session::Status::Error);
                    return Ok(());
                }
                self.home.set_instance_error(session_id, None);
                // The respawn mints the key on the clone just like a full launch
                // does, so it has to travel back here too (see the single-pane
                // branch below).
                if let Some(key) = inst.xats_identity_key.as_deref() {
                    self.home.adopt_xats_identity_key(session_id, key);
                }
                self.home.take_pending_right_pane();
            } else {
                // Single-pane or non-existent session: kill and recreate from scratch
                if session_exists {
                    let _ = tmux_session.kill();
                }
                // Show warning (once) if custom instruction is configured for an
                // unsupported agent. Only on the attach path: the dialog resumes
                // by attaching, which is exactly what StayOnHome asked not to do.
                if instance.is_sandboxed() && post == PostRestart::Attach {
                    let has_instruction = instance
                        .sandbox_info
                        .as_ref()
                        .and_then(|s| s.custom_instruction.as_ref())
                        .is_some_and(|i| !i.is_empty());

                    if has_instruction
                        && !crate::agents::get_agent(&instance.tool)
                            .is_some_and(|a| a.instruction_flag.is_some())
                    {
                        let config = load_config()?.unwrap_or_default();
                        if !config.app_state.has_seen_custom_instruction_warning {
                            self.home.info_dialog = Some(
                                crate::tui::dialogs::InfoDialog::new(
                                    "Custom Instruction Not Supported",
                                    &format!(
                                        "'{}' does not support custom instruction injection. The session will launch without the custom instruction.",
                                        instance.tool
                                    ),
                                ),
                            );
                            self.home.pending_attach_after_warning = Some(session_id.to_string());

                            let mut config = config;
                            config.app_state.has_seen_custom_instruction_warning = true;
                            save_config(&config)?;

                            return Ok(());
                        }
                    }
                }

                let size = crate::terminal::get_size();
                let skip_on_launch = self.home.take_on_launch_hooks_ran(session_id);

                self.home
                    .set_instance_status(session_id, crate::session::Status::Starting);
                let mut inst = instance.clone();
                if let Err(e) = inst.start_with_size_opts(size, skip_on_launch) {
                    self.home.take_pending_right_pane();
                    self.home
                        .set_instance_error(session_id, Some(e.to_string()));
                    self.home
                        .set_instance_status(session_id, crate::session::Status::Error);
                    return Ok(());
                }
                self.home.set_instance_error(session_id, None);
                // Propagate fork_pending clearing from the clone back to the
                // authoritative instance so subsequent restarts follow the
                // normal resume path instead of re-forking.
                if inst.fork_pending.is_none() {
                    self.home.clear_fork_pending(session_id);
                }
                // The launch mints the xats identity key on the clone, so it has
                // to travel back too: without it the record stays keyless and
                // every restart hands xats a different key, which reads as a new
                // identity instead of a restarted one.
                if let Some(key) = inst.xats_identity_key.as_deref() {
                    self.home.adopt_xats_identity_key(session_id, key);
                }

                // A failed split leaves the session with one pane and the reason
                // on the home screen. Attaching anyway would bury that reason
                // behind the session the user did not get what they asked for.
                if let Some(pending) = self.home.take_pending_right_pane() {
                    if !self.launch_managed_pane(&inst, &pending) && post == PostRestart::Attach {
                        crate::tmux::refresh_session_cache();
                        self.needs_redraw = true;
                        return Ok(());
                    }
                }
            }
        } else {
            // Session already running -- discard any pending right pane request
            self.home.take_pending_right_pane();
        }

        if post == PostRestart::StayOnHome {
            crate::tmux::refresh_session_cache();
            self.needs_redraw = true;
            return Ok(());
        }

        let attach_client_name = crate::tmux::get_tty_name();
        if let Some(client_name) = &attach_client_name {
            let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
            // Track the last managed session visited so home-screen
            // selection follows the user back after detach.
            crate::tmux::utils::set_last_detached_session_for_client(client_name, &session_name);
            self.last_attach_client = Some(client_name.clone());
        }

        instance.refresh_agent_tmux_options(self.home.storage.profile());

        let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
        let source_session = self
            .session_before_tui
            .take()
            .filter(|source| source != &session_name);
        if let Some(source_session) = source_session.as_deref() {
            crate::tmux::utils::set_target_from_title(source_session, &session_name);
            if let Some(client_name) = &attach_client_name {
                crate::tmux::utils::set_previous_session_for_client(client_name, source_session);
            }
        } else {
            crate::tmux::utils::clear_from_title(&session_name);
            if let Some(client_name) = &attach_client_name {
                crate::tmux::utils::clear_previous_session_for_client(client_name);
            }
        }
        crate::tmux::utils::update_session_index(
            &self.home.instances,
            &self.home.groups,
            self.home.sort_order,
            &session_name,
        );
        self.home
            .mutate_instance(session_id, |inst| inst.acknowledged = true);

        let profile = self.home.storage.profile().to_string();
        crate::tmux::utils::setup_session_cycle_bindings(&profile);
        let is_narrow_terminal = crate::terminal::get_size()
            .map(|(width, _)| self.home.is_narrow_layout(width))
            .unwrap_or(false);
        if tmux_session.pane_count() > 1 {
            let is_zoomed = crate::tmux::tmux_command()
                .args([
                    "display-message",
                    "-t",
                    &session_name,
                    "-p",
                    "#{window_zoomed_flag}",
                ])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim() == "1")
                .unwrap_or(false);

            let should_zoom = is_narrow_terminal && !is_zoomed;
            let should_unzoom = !is_narrow_terminal && is_zoomed;

            if should_zoom || should_unzoom {
                let agent_pane_target = format!("{session_name}:.0");
                match crate::tmux::tmux_command()
                    .args(["resize-pane", "-Z", "-t", &agent_pane_target])
                    .output()
                {
                    Ok(output) if !output.status.success() => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!("Failed to toggle zoom before attach: {}", stderr);
                    }
                    Err(error) => {
                        tracing::warn!("Failed to toggle zoom before attach: {}", error);
                    }
                    _ => {}
                }
            }
        }
        let attach_result = with_raw_mode_disabled(terminal, || tmux_session.attach())?;
        reapply_tui_title(terminal, self.home.storage.profile());

        self.needs_redraw = true;
        crate::tmux::refresh_session_cache();

        // One-shot inner-agent discovery for shell sessions. When the user
        // manually launches an agent (e.g. `claude`) inside a shell pane, we
        // want the status indicator to reflect that agent's real state
        // instead of the shell stub's always-`?`. We detect once per detach,
        // storing the result on the in-memory instance only (never
        // persisted). For every other tool this path is a no-op.
        //
        // The pane-info cache is refreshed explicitly here: attach typically
        // exceeds the 2s cache TTL, so the cached entry is stale (or missing)
        // and would yield None for the foreground process the user just
        // launched.
        if instance.tool == "shell" {
            crate::tmux::refresh_pane_info_cache();
            let detected = crate::tmux::get_cached_pane_info(&session_name)
                .as_ref()
                .and_then(crate::tmux::status_detection::detect_agent_type_from_pane);
            let normalized = match detected {
                Some("shell") | None => None,
                Some(agent) => Some(agent.to_string()),
            };
            self.home.mutate_instance(session_id, |inst| {
                inst.detected_inner_agent = normalized;
            });
        }

        self.home.reload()?;
        if !self.try_restore_selection_from_client_context() {
            self.home.select_session_by_id(session_id);
        }

        if let Err(e) = attach_result {
            tracing::warn!("tmux attach returned error: {}", e);
        }

        Ok(())
    }

    /// Split a managed agent pane into a running session and record its durable
    /// slot, so the pane is restartable and the key the launch minted has a home.
    ///
    /// An unset directory falls back to the session's own here rather than when
    /// the dialog was submitted. A worktree-backed session's directory is
    /// decided during creation, so a snapshot would put the pane in the original
    /// repository while the session went to the worktree.
    /// Returns whether the pane was created, so a caller that would otherwise
    /// attach can stay put instead of dropping the user into a session that
    /// gained nothing.
    fn launch_managed_pane(
        &mut self,
        inst: &crate::session::Instance,
        pending: &crate::session::PaneDraft,
    ) -> bool {
        let session_name = crate::tmux::Session::generate_name(&inst.id, &inst.title);
        let profile = self.home.storage.profile().to_string();
        let resolved = match crate::session::builder::resolve_pane_config(
            pending.clone(),
            Some(&inst.project_path),
            &profile,
        ) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.report_pane_not_created(&format!("{error:#}"));
                return false;
            }
        };
        let pane = &resolved.config;
        let cwd = pane.working_dir.as_str();

        // Splitting anyway would leave an empty pane the user has to close,
        // with nothing saying why it is empty.
        let launch = match inst.prepare_extra_pane_config_command(&profile, &session_name, pane) {
            Ok(launch) => launch,
            Err(error) => {
                let detail = Self::append_pane_cleanup_error(
                    format!("{error:#}"),
                    crate::session::builder::cleanup_resolved_pane(&resolved),
                );
                self.report_pane_not_created(&detail);
                return false;
            }
        };

        // The directory can be one the user typed, so a split that fails is
        // surfaced rather than logged: a pane that silently does not appear is
        // the failure mode a chosen directory introduces.
        let pane_id = match crate::tmux::split_window_right(
            &session_name,
            cwd,
            &launch.command,
            pane.tool != "shell",
        ) {
            Ok(pane_id) => pane_id,
            Err(e) => {
                let detail = Self::append_pane_cleanup_error(
                    format!("{e:#}"),
                    inst.rollback_prepared_extra_pane(&profile, &launch),
                );
                let detail = Self::append_pane_cleanup_error(
                    detail,
                    crate::session::builder::cleanup_resolved_pane(&resolved),
                );
                self.report_pane_not_created(&detail);
                return false;
            }
        };

        // The key the launch minted lives on the pane's slot record, so every
        // later relaunch reuses it instead of handing xats a key no identity
        // holds. The pane's own directory lives there too, so a restart returns
        // it here rather than to the session's directory.
        let recorded = inst.record_launched_extra_pane(
            &profile,
            &session_name,
            &crate::db::reconcile::LaunchedPane {
                pane_id: &pane_id,
                config: pane,
                identity_key: &launch.identity_key,
                native_session_id: &launch.native_session_id,
                prepared_slot: launch.prepared_slot,
                prepared_generation: launch.prepared_generation,
            },
        );

        if let Err(e) = recorded {
            tracing::error!("{:#}", e);
            let detail = match crate::tmux::kill_pane_exact(&pane_id) {
                Ok(()) => format!("{e:#}"),
                Err(rollback_error) => {
                    format!("{e:#}. Failed to roll back pane {pane_id}: {rollback_error:#}")
                }
            };
            let detail = Self::append_pane_cleanup_error(
                detail,
                inst.rollback_prepared_extra_pane(&profile, &launch),
            );
            let detail = Self::append_pane_cleanup_error(
                detail,
                crate::session::builder::cleanup_resolved_pane(&resolved),
            );
            self.report_pane_not_created(&detail);
            return false;
        }
        inst.auto_confirm_launched_pane(&pane_id, pane);
        true
    }

    fn append_pane_cleanup_error(detail: String, cleanup: anyhow::Result<()>) -> String {
        match cleanup {
            Ok(()) => detail,
            Err(error) => format!("{detail}. {error:#}"),
        }
    }

    fn report_pane_not_created(&mut self, detail: &str) {
        tracing::warn!("Right pane not created: {}", detail);
        self.home.info_dialog = Some(crate::tui::dialogs::InfoDialog::new(
            "Right pane not created",
            detail,
        ));
    }

    fn try_restore_selection_from_client_context(&mut self) -> bool {
        let Some(client_name) = self.last_attach_client.as_deref() else {
            return false;
        };

        let Some(tmux_session_name) =
            crate::tmux::utils::take_last_detached_session_for_client(client_name)
        else {
            return false;
        };

        self.session_before_tui = Some(tmux_session_name.clone());

        self.home
            .select_session_by_managed_tmux_name(&tmux_session_name)
    }

    fn edit_file(
        &mut self,
        path: &std::path::Path,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        // Determine which editor to use (prefer vim, fall back to nano)
        let editor = std::env::var("EDITOR")
            .ok()
            .or_else(|| {
                // Check if vim is available
                if std::process::Command::new("vim")
                    .arg("--version")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok()
                {
                    Some("vim".to_string())
                } else if std::process::Command::new("nano")
                    .arg("--version")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok()
                {
                    Some("nano".to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "vim".to_string());

        let path = path.to_owned();
        let editor_clone = editor.clone();
        let status = with_raw_mode_disabled(terminal, move || {
            std::process::Command::new(&editor_clone)
                .arg(&path)
                .status()
        })?;

        self.needs_redraw = true;

        // Refresh diff view if it's open (file may have changed)
        if let Some(ref mut diff_view) = self.home.diff_view {
            if let Err(e) = diff_view.refresh_files() {
                tracing::warn!("Failed to refresh diff after edit: {}", e);
            }
        }

        // Log any editor errors but don't fail
        if let Err(e) = status {
            tracing::warn!("Editor '{}' returned error: {}", editor, e);
        }

        Ok(())
    }
}

/// What to do once a restart or recovery finishes: follow the session into tmux,
/// or stay on the home list so several sessions can be restarted in a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostRestart {
    Attach,
    StayOnHome,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    AttachSession(String),
    RespawnAgentPane(String, crate::session::RestartMode, PostRestart),
    RecoverInstance(String, crate::session::RestartMode, PostRestart),
    SwitchProfile(String),
    EditFile(PathBuf),
    StopSession(String),
    SetTheme(String),
    /// Add a managed agent pane to a running session, then attach to it. The
    /// pane's tool and directory travel on `HomeView::pending_right_pane`.
    AddAgentPane(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_enum() {
        let quit = Action::Quit;
        let attach = Action::AttachSession("test-id".to_string());

        assert_eq!(quit, Action::Quit);
        assert_eq!(attach, Action::AttachSession("test-id".to_string()));
    }

    #[test]
    fn test_action_clone() {
        let original = Action::AttachSession("session-123".to_string());
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_poll_update_check_returns_true_when_update_available() {
        // Create a oneshot channel and send an update notification
        let (tx, rx) = tokio::sync::oneshot::channel();
        let update_info = UpdateInfo {
            available: true,
            current_version: "0.4.0".to_string(),
            latest_version: "0.5.0".to_string(),
        };
        tx.send(Ok(update_info)).unwrap();

        // poll_update_receiver should return true when an update is available
        let (info, rx_out, received) = poll_update_receiver(Some(rx), None);
        assert!(received);
        assert!(info.is_some());
        assert_eq!(info.as_ref().unwrap().latest_version, "0.5.0");
        assert!(rx_out.is_none()); // Channel consumed
    }

    #[test]
    fn test_poll_update_check_returns_false_when_no_update() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let update_info = UpdateInfo {
            available: false,
            current_version: "0.5.0".to_string(),
            latest_version: "0.5.0".to_string(),
        };
        tx.send(Ok(update_info)).unwrap();

        // poll_update_receiver should return false when no update available
        let (info, rx_out, received) = poll_update_receiver(Some(rx), None);
        assert!(!received);
        assert!(info.is_none());
        assert!(rx_out.is_none()); // Channel consumed even though no update
    }

    #[test]
    fn test_poll_update_check_returns_false_when_channel_empty() {
        let (_tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<UpdateInfo>>();

        // poll_update_receiver should return false when channel is empty
        let (info, rx_out, received) = poll_update_receiver(Some(rx), None);
        assert!(!received);
        assert!(info.is_none());
        // Receiver should be put back for next poll
        assert!(rx_out.is_some());
    }

    #[test]
    fn test_poll_update_check_preserves_existing_info() {
        // If we already have update info and the channel is closed, preserve the existing info
        let existing_info = UpdateInfo {
            available: true,
            current_version: "0.4.0".to_string(),
            latest_version: "0.5.0".to_string(),
        };

        // No receiver, just existing info
        let (info, rx_out, received) = poll_update_receiver(None, Some(existing_info));
        assert!(!received); // No new update received
        assert!(info.is_some()); // But existing info is preserved
        assert_eq!(info.as_ref().unwrap().latest_version, "0.5.0");
        assert!(rx_out.is_none());
    }
}
