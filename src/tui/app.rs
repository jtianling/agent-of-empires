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

/// Build the tmux command for a right pane tool. Wraps with Ctrl-Z disablement
/// and container exec for sandboxed sessions, mirroring the main tool's wrapping.
fn build_right_pane_command(instance: &crate::session::Instance, tool_name: &str) -> String {
    let agent = crate::agents::get_agent(tool_name);

    // For "shell", use the user's shell; for agents, use the registered binary.
    let binary = if tool_name == "shell" {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    } else {
        agent
            .map(|a| a.binary.to_string())
            .unwrap_or_else(|| "bash".to_string())
    };

    // Apply YOLO mode when the session has yolo enabled
    let mut cmd = binary.clone();
    let mut env_prefix = String::new();
    if instance.is_yolo_mode() && tool_name != "shell" {
        match agent.and_then(|a| a.yolo.as_ref()) {
            Some(crate::agents::YoloMode::CliFlag(flag)) => {
                cmd = format!("{} {}", cmd, flag);
            }
            Some(crate::agents::YoloMode::EnvVar(key, value)) => {
                let escaped_v = value.replace('\'', "'\\''");
                env_prefix = format!("{}='{}' ", key, escaped_v);
            }
            None => {}
        }
    }

    let escaped = cmd.replace('\'', "'\\''");

    if instance.is_sandboxed() && instance.sandbox_info.is_some() {
        let container = crate::containers::DockerContainer::from_session_id(&instance.id);
        let workdir = instance.container_workdir();
        let docker_cmd = container.exec_command(Some(&format!("-w {}", workdir)), &cmd);
        let docker_escaped = docker_cmd.replace('\'', "'\\''");
        format!(
            "{}bash -c 'stty susp undef; exec {}'",
            env_prefix, docker_escaped
        )
    } else {
        format!("{}bash -c 'stty susp undef; exec {}'", env_prefix, escaped)
    }
}

fn reapply_tui_title(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>, profile: &str) {
    let _ = tab_title::set_tui_title(terminal.backend_mut(), profile);
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

        // Load theme from config, defaulting to phosphor if empty
        let theme_name = if config.theme.name.is_empty() {
            "phosphor"
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

            let restarting_ids: Vec<String> = self
                .home
                .instances
                .iter()
                .filter(|inst| inst.status == crate::session::Status::Restarting)
                .map(|inst| inst.id.clone())
                .collect();
            let mut restart_actions = Vec::new();
            for id in restarting_ids {
                let mut action = None;
                self.home.mutate_instance(&id, |inst| {
                    action = inst.tick_pending_resume();
                });
                if let Some(action) = action {
                    restart_actions.push(action);
                }
            }
            if !restart_actions.is_empty() {
                refresh_needed = true;
            }
            for action in restart_actions {
                self.execute_action(action, terminal)?;
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
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(self.home.list_width),
                        Constraint::Min(40),
                    ])
                    .split(main_chunks[0]);

                let preview_area = chunks[1];

                // Settle all state (including tmux captures) BEFORE drawing
                self.home
                    .update_caches(preview_area.width, preview_area.height);

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
            | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                if !self.home.has_dialog() {
                    self.should_quit = true;
                    return Ok(());
                }
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
            Action::SwitchProfile(profile) => {
                let storage = Storage::new(&profile)?;
                let tools = self.home.available_tools();
                self.home = HomeView::new(storage, tools, self.launch_dir.clone())?;
            }
            Action::EditFile(path) => {
                self.edit_file(&path, terminal)?;
            }
            Action::RespawnAgentPane(id) => {
                if let Some(inst) = self.home.get_instance(&id).cloned() {
                    if inst.pending_resume.is_some() {
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
                        return self.attach_session(&id, terminal);
                    }

                    if inst.can_gracefully_restart() {
                        let mut initiate_result = Ok(false);
                        self.home.mutate_instance(&id, |inst| {
                            initiate_result = inst.initiate_graceful_restart();
                        });

                        match initiate_result {
                            Ok(true) => {
                                self.home.set_instance_error(&id, None);
                                return Ok(());
                            }
                            Ok(false) => {}
                            Err(e) => {
                                tracing::error!("Failed to initiate graceful restart: {}", e);
                                self.home.set_instance_error(&id, Some(e.to_string()));
                                self.home
                                    .set_instance_status(&id, crate::session::Status::Error);
                                return Ok(());
                            }
                        }
                    }

                    let mut respawn_result = Ok(());
                    self.home.mutate_instance(&id, |inst| {
                        respawn_result = inst.respawn_agent_pane();
                    });

                    if let Err(e) = respawn_result {
                        tracing::error!("Failed to respawn agent pane: {}", e);
                        self.home.set_instance_error(&id, Some(e.to_string()));
                        self.home
                            .set_instance_status(&id, crate::session::Status::Error);
                        return Ok(());
                    }
                    self.home.set_instance_error(&id, None);
                    crate::tmux::refresh_session_cache();

                    // Auto-attach so the user sees the restarted agent immediately
                    self.attach_session(&id, terminal)?;
                }
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

    fn attach_session(
        &mut self,
        session_id: &str,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let instance = match self.home.get_instance(session_id) {
            Some(inst) => inst.clone(),
            None => return Ok(()),
        };

        let tmux_session = instance.tmux_session()?;

        // Determine whether the agent pane needs to be (re)started.
        // Grace period: skip for freshly started/respawned instances (the brief
        // bash wrapper phase would otherwise trigger is_pane_running_shell).
        let is_starting = matches!(
            instance.status,
            crate::session::Status::Starting | crate::session::Status::Restarting
        );
        let session_exists = tmux_session.exists();
        let multi_pane = session_exists && tmux_session.pane_count() > 1;

        // For multi-pane sessions the is_pane_running_shell check is unreliable:
        // user-created shell panes (Ctrl+B %) are legitimate and should not
        // trigger a restart. Only restart when the agent pane itself is dead.
        let needs_restart = !is_starting
            && (!session_exists
                || tmux_session.is_pane_dead()
                || (!multi_pane
                    && !instance.expects_shell()
                    && tmux_session.is_pane_running_shell()));

        if needs_restart {
            if multi_pane {
                // Respawn only the agent pane, preserving user-created panes and layout
                self.home
                    .set_instance_status(session_id, crate::session::Status::Starting);
                let mut inst = instance.clone();
                if let Err(e) = inst.respawn_agent_pane() {
                    self.home
                        .set_instance_error(session_id, Some(e.to_string()));
                    self.home
                        .set_instance_status(session_id, crate::session::Status::Error);
                    return Ok(());
                }
                self.home.set_instance_error(session_id, None);
                self.home.take_pending_right_pane_tool();
            } else {
                // Single-pane or non-existent session: kill and recreate from scratch
                if session_exists {
                    let _ = tmux_session.kill();
                }
                // Show warning (once) if custom instruction is configured for an unsupported agent
                if instance.is_sandboxed() {
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
                    self.home
                        .set_instance_error(session_id, Some(e.to_string()));
                    self.home
                        .set_instance_status(session_id, crate::session::Status::Error);
                    return Ok(());
                }
                self.home.set_instance_error(session_id, None);

                if let Some(right_tool) = self.home.take_pending_right_pane_tool() {
                    let session_name = crate::tmux::Session::generate_name(&inst.id, &inst.title);
                    let right_cmd = build_right_pane_command(&inst, &right_tool);
                    if let Err(e) = crate::tmux::split_window_right(
                        &session_name,
                        &inst.project_path,
                        &right_cmd,
                    ) {
                        tracing::warn!("Failed to split right pane: {}", e);
                    }
                }
            }
        } else {
            // Session already running -- discard any pending right pane request
            self.home.take_pending_right_pane_tool();
        }

        let attach_client_name = crate::tmux::get_tty_name();
        if let Some(client_name) = &attach_client_name {
            let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
            // Track the last managed session visited so home-screen
            // selection follows the user back after detach.
            crate::tmux::utils::set_last_detached_session_for_client(client_name, &session_name);
            self.last_attach_client = Some(client_name.clone());
        }

        instance.refresh_agent_tmux_options();

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

        let profile = self.home.storage.profile().to_string();
        let attach_result = with_raw_mode_disabled(terminal, || tmux_session.attach(&profile))?;
        reapply_tui_title(terminal, self.home.storage.profile());

        self.needs_redraw = true;
        crate::tmux::refresh_session_cache();
        self.home.reload()?;
        if !self.try_restore_selection_from_client_context() {
            self.home.select_session_by_id(session_id);
        }

        if let Err(e) = attach_result {
            tracing::warn!("tmux attach returned error: {}", e);
        }

        Ok(())
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

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    AttachSession(String),
    RespawnAgentPane(String),
    SwitchProfile(String),
    EditFile(PathBuf),
    StopSession(String),
    SetTheme(String),
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
