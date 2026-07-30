//! Session instance definition and operations

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::containers::{self, ContainerRuntimeInterface, DockerContainer};
use crate::tmux;

use super::container_config;
use super::environment::{build_docker_env_args, shell_escape};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalInfo {
    #[serde(default)]
    pub created: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Waiting,
    #[default]
    Idle,
    Unknown,
    Stopped,
    Error,
    Starting,
    Restarting,
    Deleting,
}

/// Screen markers that identify Claude's startup confirmation prompts. Each
/// default-highlights the safe-to-proceed option, so a single Enter confirms.
/// A startup question Claude asks, identified by what it is rather than by how
/// its screen happens to be drawn.
///
/// The distinction is what makes answering it exactly once possible. A prompt's
/// rendered screen is not stable while the prompt is up -- a spinner, a status
/// line, or a partial redraw all change the bytes without the question having
/// been answered -- so screen content cannot stand in for the question's
/// identity. Enter is queued input: a second one sent at the same question is
/// not absorbed by it, it waits and is consumed by whatever comes next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoConfirmPrompt {
    DevelopmentChannels,
    WorkspaceTrust,
}

const AUTO_CONFIRM_MARKERS: &[(&str, AutoConfirmPrompt)] = &[
    (
        "Loading development channels",
        AutoConfirmPrompt::DevelopmentChannels,
    ),
    (
        "I am using this for local development",
        AutoConfirmPrompt::DevelopmentChannels,
    ),
    ("Quick safety check", AutoConfirmPrompt::WorkspaceTrust),
    ("trust this folder", AutoConfirmPrompt::WorkspaceTrust),
];

/// Every startup question auto-confirm knows how to answer. A pane that has
/// answered all of them cannot be asked anything else, which is one of the two
/// signals that finish a pane without a timer; the other is Claude's own input
/// prompt appearing (see `shows_claude_input_prompt`), which is what finishes a
/// launch that raises fewer questions than this list holds.
const AUTO_CONFIRM_PROMPTS: &[AutoConfirmPrompt] = &[
    AutoConfirmPrompt::DevelopmentChannels,
    AutoConfirmPrompt::WorkspaceTrust,
];

/// What to do with one pane, given what it shows and what has already been
/// answered for it. Pure, so the "same question redrawn many times is answered
/// once" rule is testable without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoConfirmStep {
    /// This pane is showing a question that has not been answered yet.
    Answer(AutoConfirmPrompt),
    /// A question is up, but it is one this pane was already answered for.
    /// Sending again would queue an Enter for whatever screen comes next.
    AlreadyAnswered,
    /// No recognized question on screen.
    NoPrompt,
}

/// Whether a pane's screen shows Claude past its startup screens and waiting for
/// input.
///
/// This is positive evidence, which is what distinguishes it from a timer. The
/// alternative -- deciding a pane is done because nothing has appeared for a
/// while -- cannot tell a question that will never be asked from one that has
/// not been asked yet, so it either abandons slow panes or waits out the whole
/// deadline on every launch. Claude's own input prompt says the startup screens
/// are behind it.
///
/// Callers must establish that no confirmation question is on screen before
/// consulting this: the questions themselves are drawn with the same prompt
/// glyph, so on their own screens it means the opposite of ready.
///
/// Two things must hold, and the second is what makes this positive rather than
/// a guess about how menus happen to be drawn: the line looks like the input
/// prompt, and it sits inside the input box. Claude draws that box as a rule
/// above and below the prompt; a menu option is drawn among its sibling
/// options. Requiring the border means a menu whose options carry no numbers --
/// which the shape test alone reads as ready -- is still recognized as a
/// question. Measured against a real ready pane, both borders are present.
fn shows_claude_input_prompt(screen: &str) -> bool {
    let plain = strip_ansi(screen);
    let window: Vec<&str> = plain
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .rev()
        .take(6)
        .collect();

    window.iter().enumerate().any(|(i, line)| {
        is_claude_input_prompt_line(line)
            && [i.checked_sub(1), Some(i + 1)]
                .into_iter()
                .flatten()
                .filter_map(|j| window.get(j))
                .any(|neighbor| is_input_box_border(neighbor))
    })
}

/// Whether a line is one of the rules Claude draws above and below its input
/// prompt.
///
/// Only the characters a real ready pane was observed to use. A rule this does
/// not recognize costs a pane the wait its deadline already bounds; treating
/// some other horizontal run as the input box would settle a pane on a question.
fn is_input_box_border(line: &str) -> bool {
    line.chars().count() >= 3 && line.chars().all(|c| c == '\u{2500}' || c == '\u{2501}')
}

/// Whether one line is Claude's input prompt rather than a selected menu entry.
///
/// The glyph alone does not distinguish them: Claude draws every menu selection
/// with it too -- the theme picker, the login chooser, and the startup
/// confirmations this code answers. Reading any `❯` as "ready" therefore reports
/// a pane that is waiting on a question as a pane that is done with them.
///
/// A menu entry is numbered (`❯ 1. ...`); the input prompt is the glyph alone,
/// or the glyph followed by what the user has typed. Text that merely starts
/// like a numbered entry is treated as a menu entry: erring that way costs a
/// wait that the overall deadline bounds, while erring the other way abandons a
/// pane on an unanswered question.
fn is_claude_input_prompt_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('\u{276f}') else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = rest.strip_prefix(' ') else {
        return false;
    };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    !(digits > 0 && rest[digits..].starts_with('.'))
}

/// Collapse every run of whitespace, newlines included, into single spaces.
///
/// A pane's screen is not the text that was written to it. Claude re-flows its
/// own output to the pane width, so a phrase can arrive split across lines with
/// fresh indentation on the continuation -- and tmux soft-wraps on top of that.
/// Matching against the screen as captured therefore fails on exactly the panes
/// this code exists for: the narrow ones a multi-pane session produces.
///
/// Measured against a real Claude confirmation screen: at 36 and 40 columns
/// neither marker matches the captured text, and both match after collapsing.
/// `capture-pane -J` does not help, because the wrapping that breaks these
/// phrases is Claude's own, not tmux's.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn auto_confirm_step(screen: &str, answered: &[AutoConfirmPrompt]) -> AutoConfirmStep {
    let plain = collapse_whitespace(&strip_ansi(screen));

    // Every question the screen shows, not the first one the marker table
    // happens to list. A screen can carry more than one: the text of a question
    // already answered can still be on screen above the one now being asked, and
    // picking the first match would report the visible history as the current
    // state and leave the real question unanswered until the deadline.
    let mut present = AUTO_CONFIRM_MARKERS
        .iter()
        .filter(|(marker, _)| plain.contains(marker))
        .map(|(_, prompt)| *prompt)
        .peekable();

    if present.peek().is_none() {
        return AutoConfirmStep::NoPrompt;
    }

    match present.find(|prompt| !answered.contains(prompt)) {
        Some(prompt) => AutoConfirmStep::Answer(prompt),
        None => AutoConfirmStep::AlreadyAnswered,
    }
}

/// Max time to wait for Claude's confirmation screens before giving up and
/// attaching anyway (claude shows the dev-channels gate within ~1-2s).
const AUTO_CONFIRM_TIMEOUT: Duration = Duration::from_secs(12);
/// Delay after sending Enter before polling again, so the next screen can render.
const AUTO_CONFIRM_SEND_INTERVAL: Duration = Duration::from_millis(600);
/// Poll cadence while waiting for a confirmation screen to appear.
const AUTO_CONFIRM_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// How long a rebuilt session is left to settle before recovery decides which
/// slots came back. A relaunched pane can survive its own respawn and disappear
/// a moment later, so checking immediately would confirm a state that is about
/// to stop being true. Recovery is a one-shot user action that already spends
/// far longer rebuilding the session, so the wait is affordable here in a way it
/// would not be on a polling path.
const RECOVERY_SETTLE: Duration = Duration::from_millis(500);

/// Override for [`RECOVERY_SETTLE`], in milliseconds. Exists for tests: whether a
/// pane dies inside the default window depends on process scheduling, so a test
/// that needs the "launched, then vanished" state to be observable widens the
/// window instead of racing it.
const RECOVERY_SETTLE_ENV: &str = "AGENT_OF_EMPIRES_RECOVERY_SETTLE_MS";

/// Ceiling for [`RECOVERY_SETTLE_ENV`]. The settle is waited out synchronously
/// on a user's recovery, before the session is handed back, so an over-large
/// value does not read as "a long setting" -- it reads as recovery having hung.
/// The knob exists to widen a window measured in hundreds of milliseconds, and
/// this is far above any value a test has needed while staying inside what a
/// user would sit through.
const RECOVERY_SETTLE_MAX: Duration = Duration::from_secs(5);

fn recovery_settle() -> Duration {
    let Some(requested) = std::env::var(RECOVERY_SETTLE_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
    else {
        return RECOVERY_SETTLE;
    };

    if requested > RECOVERY_SETTLE_MAX {
        tracing::warn!(
            "{} is {:?}, above the {:?} ceiling; using the ceiling",
            RECOVERY_SETTLE_ENV,
            requested,
            RECOVERY_SETTLE_MAX
        );
        return RECOVERY_SETTLE_MAX;
    }

    requested
}

const CODEX_XATS_APP_SERVER_HOST: &str = "127.0.0.1";
const CODEX_XATS_APP_SERVER_PORT: &str = "8799";
const CODEX_XATS_APP_SERVER_URL: &str = "ws://127.0.0.1:8799";
/// The npx spec for the xats pre-registration CLI, `@latest` included.
///
/// The tag is load-bearing, not decoration. `npx --no-install` resolves against
/// its cache by the exact spec it was asked for, so a bare name and `@latest`
/// look up different entries -- and the entry that exists on a machine running
/// xats is the one its own launcher creates, which is `@latest`. Asking for the
/// bare name therefore reports the package missing and, with `--no-install`,
/// refuses to run it: the bootstrap exits, Codex never execs, and the pane
/// falls back to a shell with npm's error as the only explanation.
///
/// A pinned version is no better -- `@0.7.7` misses the `@latest` entry just as
/// the bare name does, even with 0.7.7 sitting in the cache. And this is not
/// reaching for the network: the app-server check immediately above already
/// requires xats to be running here, which is what put the entry there.
const CODEX_XATS_PACKAGE: &str = "cross-agent-teams-mcp@latest";
/// Environment variable carrying a pane's opaque xats identity key. Deliberately
/// not named `*_TOKEN`: the xats project already uses `XATS_TOKEN` for the
/// daemon's bearer credential, and both appear in the same launcher shell.
const XATS_IDENTITY_KEY_ENV: &str = "XATS_IDENTITY_KEY";

const CODEX_XATS_MISSING_PANE: &str = "[xats] Missing TMUX_PANE for Codex pre-registration.";
const CODEX_XATS_MISSING_UUIDGEN: &str =
    "[xats] Missing uuidgen required for Codex pre-registration.";
const CODEX_XATS_MISSING_NC: &str = "[xats] Missing nc required to check the Codex app-server.";
const CODEX_XATS_MISSING_NPX: &str = "[xats] Missing npx required for Codex pre-registration.";
const CODEX_XATS_INVALID_UUID: &str = "[xats] uuidgen returned an invalid Codex agent UUID.";
const CODEX_XATS_APP_SERVER_UNAVAILABLE: &str =
    "[xats] Codex app-server is not listening on ws://127.0.0.1:8799.";

/// Strip ANSI/CSI escape sequences (e.g. SGR color codes) from captured pane
/// content. Claude colors the warning title per-word, so `tmux capture-pane -e`
/// interleaves escape codes between words ("Loading\x1b[0m development...");
/// stripping them restores contiguous text for substring matching.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI sequence: ESC '[' params... final-byte (e.g. 'm'). Drop through
            // the final alphabetic byte. Other escapes: just drop the ESC.
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct StatusUpdateOptions {
    pub allow_capture: bool,
    pub reused_status: Option<Status>,
}

impl Default for StatusUpdateOptions {
    fn default() -> Self {
        Self {
            allow_capture: true,
            reused_status: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub branch: String,
    pub main_repo_path: String,
    pub managed_by_aoe: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub cleanup_on_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRepo {
    pub name: String,
    pub source_path: String,
    pub branch: String,
    pub worktree_path: String,
    pub main_repo_path: String,
    pub managed_by_aoe: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub branch: String,
    pub workspace_dir: String,
    pub repos: Vec<WorkspaceRepo>,
    pub created_at: DateTime<Utc>,
    #[serde(default = "default_true")]
    pub cleanup_on_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    pub image: String,
    pub container_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// Additional environment entries (session-specific).
    /// `KEY` = pass through from host, `KEY=VALUE` = set explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_env: Option<Vec<String>>,
    /// Custom instruction text to inject into agent launch command
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_instruction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub title: String,
    pub project_path: String,
    #[serde(default)]
    pub group_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub extra_args: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub yolo_mode: bool,
    /// When set for a supported non-sandboxed tool, launches with its xats
    /// integration. Claude uses development channels, while Codex uses a
    /// pane-local app-server bootstrap.
    #[serde(default)]
    pub cross_agent_team: bool,
    /// Development-channels string appended after
    /// `--dangerously-load-development-channels` when `cross_agent_team` is set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cross_agent_team_channel: String,
    #[serde(default)]
    pub status: Status,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<DateTime<Utc>>,

    // Git worktree integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_info: Option<WorktreeInfo>,

    // Multi-repo workspace integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_info: Option<WorkspaceInfo>,

    // Docker sandbox integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_info: Option<SandboxInfo>,

    // Paired terminal session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_info: Option<TerminalInfo>,

    /// Runtime-only: which profile this instance was loaded from. Not persisted to disk.
    #[serde(default, skip_serializing)]
    pub source_profile: String,

    // Runtime state (not serialized)
    #[serde(skip)]
    pub last_error_check: Option<std::time::Instant>,
    #[serde(skip)]
    pub last_start_time: Option<std::time::Instant>,
    #[serde(skip)]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
    /// When set, indicates this session is a pending fork of another session.
    /// The stored value is the parent agent's session token (e.g. Claude/Codex UUID
    /// or OpenCode `ses_...` id). On first successful launch, `build_base_pane_command`
    /// uses the agent's `fork_template` with this value, and the field is then cleared
    /// so subsequent restarts follow the normal resume path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_pending: Option<String>,
    /// Pre-allocated agent session UUID. When AoE starts a Claude session it
    /// passes `--session-id <uuid>` so we always know which conversation
    /// belongs to this instance. Used as the primary source for `fork_token()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    /// Opaque xats identity key for the primary pane, minted on its first Cross
    /// Agent Team launch and reused on every later one. The agent presents it to
    /// the xats daemon to recover the team and name it registered under, so a
    /// launch that discards the conversation does not also discard the identity.
    /// Never interpreted by AoE. Not inherited by forks (see `create_fork`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xats_identity_key: Option<String>,
    /// Runtime-only flag: set while a multi-pane `R` restart is in flight so a
    /// second `R` press on the same instance is ignored. Cleared once every
    /// tracked pane has been respawned. Never persisted.
    #[serde(skip)]
    pub restart_in_flight: bool,
    #[serde(skip)]
    pub last_spinner_seen: Option<Instant>,
    #[serde(skip)]
    pub spike_start: Option<Instant>,
    #[serde(skip)]
    pub pre_spike_status: Option<Status>,
    #[serde(skip)]
    pub acknowledged: bool,
    /// Runtime-only: when `tool == "shell"` and the user detaches from the
    /// session with an agent (claude/codex/gemini/...) running in the primary
    /// pane, this field caches the detected agent name. The status poller
    /// uses it to dispatch to that agent's content detector instead of the
    /// shell stub. Cleared to `None` when detection returns `shell` or
    /// unknown. Never persisted to disk: on aoe restart every session starts
    /// with `None` and the next detach cycle repopulates it.
    #[serde(skip, default)]
    pub detected_inner_agent: Option<String>,
}

impl Instance {
    pub fn new(title: &str, project_path: &str) -> Self {
        Self {
            id: generate_id(),
            title: title.to_string(),
            project_path: project_path.to_string(),
            group_path: String::new(),
            parent_session_id: None,
            command: String::new(),
            extra_args: String::new(),
            tool: "claude".to_string(),
            yolo_mode: false,
            cross_agent_team: false,
            cross_agent_team_channel: String::new(),
            status: Status::Idle,
            created_at: Utc::now(),
            last_accessed_at: None,
            worktree_info: None,
            workspace_info: None,
            sandbox_info: None,
            terminal_info: None,
            source_profile: String::new(),
            last_error_check: None,
            last_start_time: None,
            last_error: None,
            resume_token: None,
            fork_pending: None,
            agent_session_id: None,
            xats_identity_key: None,
            restart_in_flight: false,
            last_spinner_seen: None,
            spike_start: None,
            pre_spike_status: None,
            acknowledged: false,
            detected_inner_agent: None,
        }
    }

    fn current_profile() -> String {
        std::env::var("AGENT_OF_EMPIRES_PROFILE")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| super::DEFAULT_PROFILE.to_string())
    }

    pub fn is_sub_session(&self) -> bool {
        self.parent_session_id.is_some()
    }

    pub fn is_workspace(&self) -> bool {
        self.workspace_info.is_some()
    }

    pub fn is_sandboxed(&self) -> bool {
        self.sandbox_info.as_ref().is_some_and(|s| s.enabled)
    }

    pub fn is_yolo_mode(&self) -> bool {
        self.yolo_mode
    }

    pub fn supports_cross_agent_team_tool(tool: &str) -> bool {
        matches!(tool, "claude" | "codex")
    }

    /// Whether this instance launches with tool-specific Cross Agent Team behavior.
    pub fn is_cross_agent_team(&self) -> bool {
        self.cross_agent_team
            && !self.is_sandboxed()
            && Self::supports_cross_agent_team_tool(&self.tool)
    }

    /// Whether a pane running `target_agent` takes this instance's Cross Agent
    /// Team integration.
    ///
    /// Which integration a pane needs is decided by the agent running in it, not
    /// by the instance's tool: a Claude pane adopted into a Codex instance needs
    /// Claude's development-channel flag, and a Codex pane adopted into a Claude
    /// instance needs Codex's bootstrap. Keying either on `self.tool` gives an
    /// adopted pane the wrong integration in one direction and none in the other.
    ///
    /// What stays instance-level is whether Cross Agent Team is on at all.
    fn cross_agent_team_pane(&self, target_agent: &str) -> bool {
        self.is_cross_agent_team() && Self::supports_cross_agent_team_tool(target_agent)
    }

    /// Mint this instance's primary-pane xats identity key if Cross Agent Team is
    /// enabled and it has none yet. Write-once: every later launch reuses it, which
    /// is what lets a launch that discards the conversation keep the identity.
    fn ensure_xats_identity_key(&mut self) {
        if self.is_cross_agent_team() && self.xats_identity_key.is_none() {
            self.xats_identity_key = Some(Uuid::new_v4().to_string());
        }
    }

    /// Whether AoE should mint an identity key for this slot before launching it.
    /// Slot 0 is the primary pane, whose key lives on the instance record, and a
    /// slot that already has one keeps it.
    fn slot_needs_identity_key(&self, slot: &crate::db::AgentSlot) -> bool {
        // Slot 0 is skipped because the instance record already holds the key for
        // the instance's own agent -- which is only true when slot 0 is running
        // that agent. An adopted slot 0 running something else is described by
        // neither: the instance's key belongs to a different agent, and skipping
        // it here would leave that pane the only tracked pane with no key at all.
        let instance_record_holds_it = slot.slot == 0 && self.pane_runs_instance_tool(&slot.agent);
        self.is_cross_agent_team() && !instance_record_holds_it && slot.xats_identity_key.is_empty()
    }

    /// Mint and persist an identity key for every adopted slot that has none, so
    /// panes AoE is about to launch carry one. Slot 0 is the primary pane, whose
    /// key lives on the instance record instead.
    ///
    /// This is where a hand-started pane first gets a key: adoption is
    /// observe-first, so AoE never built that pane's original command and could
    /// not have injected one earlier.
    pub fn ensure_slot_identity_keys(
        &self,
        store: &crate::db::Store,
        slots: &mut [crate::db::AgentSlot],
    ) {
        for slot in slots.iter_mut().filter(|s| self.slot_needs_identity_key(s)) {
            let key = Uuid::new_v4().to_string();
            match store.upsert_agent_slot(
                &slot.instance_id,
                slot.slot,
                &slot.agent,
                &slot.native_session_id,
                &slot.cwd,
                &slot.tmux_pane,
                &key,
                slot.last_seen_at,
            ) {
                Ok(()) => slot.xats_identity_key = key,
                Err(e) => tracing::warn!(
                    "Could not persist xats identity key for slot {} of '{}': {}",
                    slot.slot,
                    self.title,
                    e
                ),
            }
        }
    }

    /// The identity key to inject into a pane being launched: the instance's own
    /// for the primary pane, the slot's for an adopted one. `None` when Cross
    /// Agent Team is off, so no variable is injected at all.
    fn xats_identity_key_for_pane<'a>(
        &'a self,
        is_primary: bool,
        slot_identity_key: Option<&'a str>,
    ) -> Option<&'a str> {
        if !self.is_cross_agent_team() {
            return None;
        }
        let key = if is_primary {
            self.xats_identity_key.as_deref()
        } else {
            slot_identity_key
        };
        key.filter(|k| !k.is_empty())
    }

    /// Auto-confirm Claude's startup screens for this instance's own agent pane.
    ///
    /// Callers that launched more than one Claude pane use
    /// [`auto_confirm_panes`](Self::auto_confirm_panes) with the panes they
    /// launched: this entry point answers for the agent pane and nothing else,
    /// which is the whole set only on the single-pane start and respawn paths.
    /// Whether this instance's own agent pane has Claude startup screens to
    /// answer.
    ///
    /// Only a Claude pane raises a Claude question. Sending any other into the
    /// Claude flow does not merely do nothing: with no question to answer and no
    /// Claude input prompt to read as ready, it waits out the whole deadline
    /// synchronously, before every launch of that session.
    ///
    /// `pane_agent` is what the pane runs, which is the instance's tool only
    /// where the caller has just launched that tool into it. A restart reads the
    /// pane instead -- see
    /// [`pane_agent_overriding_instance_tool`](Self::pane_agent_overriding_instance_tool).
    ///
    /// Separate from [`cross_agent_team_pane`](Self::cross_agent_team_pane),
    /// which asks what integration a pane's agent needs.
    fn agent_pane_has_claude_prompts(&self, pane_agent: &str) -> bool {
        pane_agent == "claude" && self.is_cross_agent_team()
    }

    fn run_auto_confirm(&self, pane_agent: &str) {
        if !self.agent_pane_has_claude_prompts(pane_agent) {
            return;
        }
        let session_name = tmux::Session::generate_name(&self.id, &self.title);
        let Some(agent_pane) = tmux::get_agent_pane_id(&session_name) else {
            return;
        };
        self.auto_confirm_panes(&[agent_pane]);
    }

    /// Answer the confirmation screens of the panes named, and of no others.
    ///
    /// The caller passes the panes it just launched Claude into. Sending Enter is
    /// not a read-only act: a pane that is not asking a Claude startup question
    /// is doing something else, and a keystroke sent into it is executed by
    /// whatever that is. So the set is what the caller launched -- never "every
    /// pane in the session", which reaches hand-split shells and panes belonging
    /// to other agents.
    ///
    /// Runs SYNCHRONOUSLY before the caller attaches: at this point the panes
    /// exist and Claude renders into the tmux virtual terminal even with no
    /// client attached, and there is no concurrent `tmux attach` to contend with
    /// the capture/send subprocesses (a background thread would stall once attach
    /// starts). No-ops when the session is not in Cross Agent Team mode.
    ///
    /// Each pane carries its own progress. A shared "everything went quiet"
    /// signal finishes as soon as the fastest pane settles, which abandons any
    /// pane whose prompt has not appeared yet.
    ///
    /// A pane is answered at most once per question, keyed by which question it
    /// is rather than by the screen showing it -- see [`AutoConfirmPrompt`] for
    /// why the screen cannot stand in for the question's identity.
    ///
    /// A pane finishes early on evidence, never on silence. Either every known
    /// question has been answered, so nothing further can be asked, or Claude's
    /// own input prompt is on screen with no question beside it, which says the
    /// startup screens are behind it -- the common case, since a launch usually
    /// raises fewer questions than this code knows about. Absent either, the
    /// pane is watched until the overall deadline, including after a quiet gap,
    /// because a question that has not been asked yet is indistinguishable from
    /// one that will never come. Per-question answering is what makes that wait
    /// safe: watching longer cannot produce a second Enter for a question
    /// already answered.
    fn auto_confirm_panes(&self, panes: &[String]) {
        if !self.cross_agent_team_pane("claude") || panes.is_empty() {
            return;
        }

        struct PaneConfirm<'a> {
            pane: &'a str,
            answered: Vec<AutoConfirmPrompt>,
            settled: bool,
        }

        let mut tracked: Vec<PaneConfirm> = panes
            .iter()
            .map(|pane| PaneConfirm {
                pane: pane.as_str(),
                answered: Vec::new(),
                settled: false,
            })
            .collect();

        let start = Instant::now();
        while start.elapsed() < AUTO_CONFIRM_TIMEOUT {
            let mut answered_this_round = false;

            for entry in tracked.iter_mut().filter(|e| !e.settled) {
                let screen = match tmux::capture_pane_screen(entry.pane) {
                    Ok(screen) => screen,
                    Err(err) => {
                        // The pane is gone or unreadable. Nothing can be
                        // confirmed in it, and retrying costs the deadline that
                        // its siblings still need.
                        tracing::warn!("auto-confirm cannot read pane {}: {}", entry.pane, err);
                        entry.settled = true;
                        continue;
                    }
                };

                let step = auto_confirm_step(&screen, &entry.answered);
                if matches!(step, AutoConfirmStep::NoPrompt) && shows_claude_input_prompt(&screen) {
                    // Claude is up and waiting for input, so its startup screens
                    // are behind it. This is the completion signal; waiting out
                    // the deadline here would make every launch pay for it.
                    entry.settled = true;
                    continue;
                }
                let AutoConfirmStep::Answer(prompt) = step else {
                    // Either nothing is being asked, or what is being asked was
                    // already answered and is waiting to be processed.
                    continue;
                };

                match tmux::send_keys_to_pane_target(entry.pane, &["Enter"]) {
                    Ok(()) => {
                        entry.answered.push(prompt);
                        answered_this_round = true;
                        entry.settled = AUTO_CONFIRM_PROMPTS
                            .iter()
                            .all(|known| entry.answered.contains(known));
                    }
                    Err(err) => {
                        // One unreachable pane must not strand its siblings on
                        // their own prompts.
                        tracing::warn!("auto-confirm send to pane {} failed: {}", entry.pane, err);
                        entry.settled = true;
                    }
                }
            }

            if tracked.iter().all(|entry| entry.settled) {
                return;
            }

            std::thread::sleep(if answered_this_round {
                AUTO_CONFIRM_SEND_INTERVAL
            } else {
                AUTO_CONFIRM_POLL_INTERVAL
            });
        }
    }

    /// The `--dangerously-load-development-channels <channel>` flag for Cross
    /// Agent Team launches, or `None` when the mode is off. Falls back to the
    /// default channel when the stored channel is empty.
    fn claude_cross_agent_team_flag(&self) -> Option<String> {
        if !self.is_cross_agent_team() {
            return None;
        }
        let channel = if self.cross_agent_team_channel.is_empty() {
            "server:cross-agent-teams-channel"
        } else {
            self.cross_agent_team_channel.as_str()
        };
        Some(format!(
            "--dangerously-load-development-channels {}",
            channel
        ))
    }

    /// The binary a pane's command starts with.
    ///
    /// The instance's command override describes `self.tool` and nothing else,
    /// so a pane running a different agent starts from that agent's own binary.
    /// Reading the override for such a pane produces a command that launches the
    /// instance's agent under another agent's integration.
    fn pane_base_command(&self, target_agent: &str) -> String {
        if self.pane_runs_instance_tool(target_agent) {
            self.get_tool_command().to_string()
        } else {
            crate::agents::get_agent(target_agent)
                .map(|a| a.binary)
                .unwrap_or(target_agent)
                .to_string()
        }
    }

    /// Codex config overrides that carry this pane's identity into its hooks.
    ///
    /// Codex executes hooks and tools inside a shared `--remote` app-server
    /// process, which inherits its environment once at daemon start. A hook
    /// therefore sees that daemon's `$TMUX_PANE` -- some other pane, hours old
    /// -- rather than the pane its own conversation is running in, and never
    /// sees `$AOE_INSTANCE_ID` at all. What Codex does forward per session is
    /// config, so both values ride in as `shell_environment_policy.set` entries
    /// (a merge into the user's own table, not a replacement) and the hook
    /// reads them from there.
    ///
    /// `$TMUX_PANE` is expanded by the pane's own shell at launch, where it is
    /// still correct; only the hook's environment is unreliable.
    fn codex_hook_env_overrides(&self) -> String {
        format!(
            " -c \"shell_environment_policy.set.{pane_env}=\\\"$TMUX_PANE\\\"\" \
             -c \"shell_environment_policy.set.AOE_INSTANCE_ID=\\\"{id}\\\"\"",
            pane_env = crate::hooks::AOE_PANE_ENV,
            id = self.id,
        )
    }

    fn codex_xats_bootstrap_command(&self, cmd: &str, base: &str) -> String {
        let suffix = cmd.strip_prefix(base).unwrap_or_default();
        let project_path = shell_escape(&self.project_path);
        let app_server_url = shell_escape(CODEX_XATS_APP_SERVER_URL);
        let codex_command = format!(
            "{base} --remote {app_server_url} -C {project_path} \
             -c \"xats.agent_id=\\\"${{xats_agent_id}}\\\"\"{suffix}"
        );
        let script = format!(
            "if [ -z \"${{TMUX_PANE:-}}\" ]; then \
                 printf '%s\\n' '{missing_pane}' >&2; \
                 exit 1; \
             fi; \
             if ! command -v uuidgen >/dev/null 2>&1; then \
                 printf '%s\\n' '{missing_uuidgen}' >&2; \
                 exit 1; \
             fi; \
             if ! command -v nc >/dev/null 2>&1; then \
                 printf '%s\\n' '{missing_nc}' >&2; \
                 exit 1; \
             fi; \
             if ! command -v npx >/dev/null 2>&1; then \
                 printf '%s\\n' '{missing_npx}' >&2; \
                 exit 1; \
             fi; \
             xats_agent_id=\"$(uuidgen)\" || {{ \
                 printf '%s\\n' '[xats] Failed to generate a Codex agent UUID.' >&2; \
                 exit 1; \
             }}; \
             case \"$xats_agent_id\" in \
                 ????????-????-????-????-????????????) ;; \
                 *) \
                     printf '%s\\n' '{invalid_uuid}' >&2; \
                     exit 1 \
                     ;; \
             esac; \
             case \"$xats_agent_id\" in \
                 *[!0-9A-Fa-f-]*) \
                     printf '%s\\n' '{invalid_uuid}' >&2; \
                     exit 1 \
                     ;; \
                 *) ;; \
             esac; \
             if ! nc -z {host} {port} >/dev/null 2>&1; then \
                 printf '%s\\n' '{app_server_unavailable}' >&2; \
                 exit 1; \
             fi; \
             if ! npx --no-install {package} pre-register-codex-pane \
                 --pane \"$TMUX_PANE\" --agent-id \"$xats_agent_id\"; then \
                 printf '%s\\n' '[xats] Failed to pre-register the Codex pane.' >&2; \
                 exit 1; \
             fi; \
             exec {codex_command}",
            host = CODEX_XATS_APP_SERVER_HOST,
            port = CODEX_XATS_APP_SERVER_PORT,
            package = CODEX_XATS_PACKAGE,
            missing_pane = CODEX_XATS_MISSING_PANE,
            missing_uuidgen = CODEX_XATS_MISSING_UUIDGEN,
            missing_nc = CODEX_XATS_MISSING_NC,
            missing_npx = CODEX_XATS_MISSING_NPX,
            invalid_uuid = CODEX_XATS_INVALID_UUID,
            app_server_unavailable = CODEX_XATS_APP_SERVER_UNAVAILABLE,
        );
        format!("sh -c {}", shell_escape(&script))
    }

    fn has_custom_command(&self) -> bool {
        if !self.extra_args.is_empty() {
            return true;
        }
        self.has_command_override()
    }

    fn has_command_override(&self) -> bool {
        if self.command.is_empty() {
            return false;
        }
        crate::agents::get_agent(&self.tool)
            .map(|a| self.command != a.binary)
            .unwrap_or(true)
    }

    pub fn expects_shell(&self) -> bool {
        crate::tmux::utils::is_shell_command(self.get_tool_command())
    }

    pub fn get_tool_command(&self) -> &str {
        if self.command.is_empty() {
            crate::agents::get_agent(&self.tool)
                .map(|a| a.binary)
                .unwrap_or("bash")
        } else {
            &self.command
        }
    }

    pub fn tmux_session(&self) -> Result<tmux::Session> {
        tmux::Session::new(&self.id, &self.title)
    }

    fn sandbox_display(&self) -> Option<crate::tmux::status_bar::SandboxDisplay> {
        self.sandbox_info.as_ref().and_then(|s| {
            if s.enabled {
                Some(crate::tmux::status_bar::SandboxDisplay {
                    container_name: s.container_name.clone(),
                })
            } else {
                None
            }
        })
    }

    /// Apply all configured tmux options to a session with the given name and title.
    fn apply_session_tmux_options(&self, session_name: &str, display_title: &str, profile: &str) {
        let branch = self
            .worktree_info
            .as_ref()
            .map(|w| w.branch.as_str())
            .or_else(|| self.workspace_info.as_ref().map(|w| w.branch.as_str()));
        let sandbox = self.sandbox_display();
        crate::tmux::status_bar::apply_all_tmux_options(
            session_name,
            display_title,
            branch,
            sandbox.as_ref(),
            profile,
        );
    }

    pub fn start(&mut self) -> Result<()> {
        self.start_with_size(None)
    }

    pub fn start_with_size(&mut self, size: Option<(u16, u16)>) -> Result<()> {
        self.start_with_size_opts(size, false)
    }

    /// Start the session, optionally skipping on_launch hooks (e.g. when they
    /// already ran in the background creation poller).
    pub fn start_with_size_opts(
        &mut self,
        size: Option<(u16, u16)>,
        skip_on_launch: bool,
    ) -> Result<()> {
        self.start_with_size_inner(size, skip_on_launch, SessionLaunch::Agent)
    }

    /// Create the session without launching the agent, leaving its first pane on
    /// the default shell. Used by recovery, which relaunches every pane from its
    /// durable slot afterwards: launching the agent here too would start it twice,
    /// and the first launch carries the conversation id of the session being
    /// recovered, which a real agent refuses to reopen and exits over -- taking
    /// the pane, and with it the single-pane session, down before the real launch
    /// runs.
    pub fn start_placeholder_with_size(&mut self, size: Option<(u16, u16)>) -> Result<()> {
        self.start_with_size_inner(size, false, SessionLaunch::Placeholder)
    }

    fn start_with_size_inner(
        &mut self,
        size: Option<(u16, u16)>,
        skip_on_launch: bool,
        launch: SessionLaunch,
    ) -> Result<()> {
        self.clear_resume_token();
        self.ensure_xats_identity_key();
        let session = self.tmux_session()?;

        if session.exists() {
            return Ok(());
        }

        // Resolve on_launch hooks from the full config chain (global > profile > repo).
        // Repo hooks go through trust verification; global/profile hooks are implicitly trusted.
        let on_launch_hooks = if skip_on_launch {
            None
        } else {
            self.resolve_on_launch_hooks()
        };

        // Install status-detection hooks for agents that support them
        let agent = crate::agents::get_agent(&self.tool);
        if let Some(hook_cfg) = agent.and_then(|a| a.hook_config.as_ref()) {
            if self.is_sandboxed() {
                // For sandboxed sessions, hooks are installed via build_container_config
            } else {
                // Install hooks in the user's home directory settings
                if let Some(home) = dirs::home_dir() {
                    let settings_path = home.join(hook_cfg.settings_rel_path);
                    if let Err(e) =
                        crate::hooks::install_hooks(&settings_path, hook_cfg.events, &self.tool)
                    {
                        tracing::warn!("Failed to install agent hooks: {}", e);
                    }
                }
            }
        }

        // Ensure container is running for sandboxed sessions, then execute hooks
        if self.is_sandboxed() {
            self.get_container_for_instance()?;
            if let Some(ref hook_cmds) = on_launch_hooks {
                self.execute_on_launch_hooks(hook_cmds);
            }
        } else if let Some(ref hook_cmds) = on_launch_hooks {
            self.execute_on_launch_hooks(hook_cmds);
        }

        // Pre-allocate an agent session UUID for tools that support
        // `--session-id`. This lets AoE know the conversation identity
        // from the start (needed for fork, and avoids relying on
        // post-hoc pane scraping or disk scanning).
        if self.agent_session_id.is_none()
            && self.resume_token.is_none()
            && self.fork_pending.is_none()
            && !self.has_command_override()
        {
            if let Some(agent) = crate::agents::get_agent(&self.tool) {
                if agent.session_id_flag.is_some() {
                    self.agent_session_id = Some(Uuid::new_v4().to_string());
                }
            }
        }

        let cmd = match launch {
            SessionLaunch::Agent => self.build_agent_command(None),
            SessionLaunch::Placeholder => None,
        };
        tracing::debug!("agent cmd: {}", cmd.as_ref().map_or("none", |v| v));
        session.create_with_size(
            &self.project_path,
            cmd.as_deref(),
            size,
            !self.expects_shell(),
        )?;

        if launch == SessionLaunch::Agent {
            // The pane was just created running this instance's tool, so there
            // is nothing to read back off it.
            self.run_auto_confirm(&self.tool.clone());
        }

        // Apply all configured tmux options (status bar, mouse, etc.)
        self.apply_tmux_options(&Self::current_profile());

        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());
        self.restart_in_flight = false;
        if launch == SessionLaunch::Agent {
            // First launch of a forked session has been committed to tmux. The
            // agent will now spawn its own session id, so we no longer need the
            // parent's token and subsequent restarts follow the normal resume flow.
            // A placeholder launched no agent, so the token must survive for the
            // per-slot launch that follows.
            self.fork_pending = None;
        }

        Ok(())
    }

    /// Build the agent launch command string. Pure command construction with no
    /// side effects (no hooks, no container lifecycle management).
    ///
    /// Delegates to [`build_pane_command`](Self::build_pane_command) for the
    /// primary agent (`self.tool`, `is_primary = true`) so that the single-pane
    /// start/respawn path and the slot-based multi-pane resume path share one
    /// launch-context decoration pipeline.
    pub fn build_agent_command(&self, resume_token: Option<&str>) -> Option<String> {
        let tool = self.tool.clone();
        self.build_pane_command(&tool, resume_token, true, None)
    }

    /// Whether a pane running `target_agent` is the pane the instance's own
    /// launch context describes. Agent tracking is observe-first, so a slot can
    /// record an agent the instance never launched; the instance's conversation
    /// identity and launch overrides belong to `self.tool` alone.
    fn pane_runs_instance_tool(&self, target_agent: &str) -> bool {
        target_agent == self.tool
    }

    /// The agent running in this instance's own pane, when it is not the agent
    /// the instance's tool describes.
    ///
    /// A restart with no tracked slots rebuilds the pane from the instance's
    /// tool. That is right for a pane still running that tool, and for one a
    /// user handed to a different agent it does not restart the pane -- it
    /// replaces the agent in it. Slot-based recovery already treats the pane's
    /// own agent as authoritative; this is the same rule where there is no slot
    /// to read it from.
    ///
    /// Positive identification only, from two directions. The process must name
    /// a registered agent exactly, and an instance carrying a command override
    /// is skipped entirely: there the pane is *meant* to run something other
    /// than the tool's binary, and the override is what AoE must relaunch. So a
    /// pane whose process says nothing recognizable is restarted exactly as it
    /// was before.
    fn pane_agent_overriding_instance_tool(&self) -> Option<&'static str> {
        if !self.command.is_empty() {
            return None;
        }
        let session_name = tmux::Session::generate_name(&self.id, &self.title);
        let running = tmux::pane_current_command(&session_name)?;
        crate::agents::agent_from_process_name(&running).filter(|agent| *agent != self.tool)
    }

    /// Build the launch command for a single pane, applying the full launch
    /// context (resume flag, YOLO mode, cross-agent-team flag, `AOE_INSTANCE_ID`
    /// for hook-config agents, sandbox `docker exec` wrapping, custom instruction,
    /// and command override). This is the one decoration pipeline shared by the
    /// single-pane start/respawn path and the slot-based multi-pane resume path.
    ///
    /// `target_agent` is the agent that runs in this pane (the instance tool for
    /// the primary pane, or a slot's recorded agent for an adopted or secondary
    /// one). `is_primary` marks the instance's primary pane (slot 0), but the
    /// instance-primary concepts -- the command override (`self.command`),
    /// pre-allocated session id, fork token, `extra_args` and the instance's own
    /// identity key -- describe `self.tool` and nothing else. They are therefore
    /// applied only when the pane's agent *is* that tool: a pane whose slot
    /// recorded a different agent (a hand-started pane AoE only adopted) builds
    /// from its own binary even when it occupies the primary slot.
    ///
    /// `slot_identity_key` supplies an adopted pane's xats identity key. It is
    /// ignored for the instance's own agent pane, which uses the instance's key.
    pub fn build_pane_command(
        &self,
        target_agent: &str,
        resume_token: Option<&str>,
        is_primary: bool,
        slot_identity_key: Option<&str>,
    ) -> Option<String> {
        let agent = crate::agents::get_agent(target_agent);
        let is_primary = is_primary && self.pane_runs_instance_tool(target_agent);

        if self.is_sandboxed() {
            let sandbox = self.sandbox_info.as_ref()?;
            let container = DockerContainer::from_session_id(&self.id);

            let base_cmd = self.build_base_pane_command(agent, resume_token, is_primary);
            let mut tool_cmd = if self.is_yolo_mode() {
                if let Some(yolo) = agent.and_then(|a| a.yolo.as_ref()) {
                    match yolo {
                        crate::agents::YoloMode::CliFlag(flag) => {
                            format!("{} {}", base_cmd, flag)
                        }
                        crate::agents::YoloMode::EnvVar(..)
                        | crate::agents::YoloMode::AlwaysYolo => base_cmd,
                    }
                } else {
                    base_cmd
                }
            } else {
                base_cmd
            };
            if is_primary {
                if let Some(ref instruction) = sandbox.custom_instruction {
                    if !instruction.is_empty() {
                        if let Some(flag_template) = agent.and_then(|a| a.instruction_flag) {
                            let escaped = shell_escape(instruction);
                            let flag = flag_template.replace("{}", &escaped);
                            tool_cmd = format!("{} {}", tool_cmd, flag);
                        }
                    }
                }
            }

            let mut env_args = build_docker_env_args(sandbox);
            env_args = format!("{} -e AOE_INSTANCE_ID={}", env_args, self.id);
            let env_part = format!("{} ", env_args);
            Some(wrap_command_ignore_suspend(
                &container.exec_command(Some(&env_part), &tool_cmd),
            ))
        } else {
            let needs_instance_id = agent.and_then(|a| a.hook_config.as_ref()).is_some();
            let has_override = is_primary && !self.command.is_empty();

            if !has_override {
                agent.filter(|a| a.supports_host_launch).map(|a| {
                    let mut cmd = self.build_base_pane_command(Some(a), resume_token, is_primary);
                    let mut env_vars: Vec<(&str, &str)> = Vec::new();
                    if needs_instance_id {
                        env_vars.push(("AOE_INSTANCE_ID", &self.id));
                    }
                    if self.is_yolo_mode() {
                        if let Some(ref yolo) = a.yolo {
                            match yolo {
                                crate::agents::YoloMode::CliFlag(flag) => {
                                    cmd = format!("{} {}", cmd, flag);
                                }
                                crate::agents::YoloMode::EnvVar(key, value) => {
                                    env_vars.push((key, value));
                                }
                                crate::agents::YoloMode::AlwaysYolo => {}
                            }
                        }
                    }
                    if self.cross_agent_team_pane(target_agent) {
                        match target_agent {
                            "claude" => {
                                if let Some(flag) = self.claude_cross_agent_team_flag() {
                                    cmd = format!("{} {}", cmd, flag);
                                }
                            }
                            "codex" => {
                                let base = self.pane_base_command(target_agent);
                                cmd = self.codex_xats_bootstrap_command(&cmd, &base);
                            }
                            _ => {}
                        }
                    }
                    if let Some(key) =
                        self.xats_identity_key_for_pane(is_primary, slot_identity_key)
                    {
                        env_vars.push((XATS_IDENTITY_KEY_ENV, key));
                    }
                    wrap_command_ignore_suspend_with_env(&cmd, &env_vars)
                })
            } else {
                let mut cmd = self.build_base_pane_command(agent, resume_token, is_primary);
                let mut env_vars: Vec<(&str, &str)> = Vec::new();
                if needs_instance_id {
                    env_vars.push(("AOE_INSTANCE_ID", &self.id));
                }
                if self.is_yolo_mode() {
                    if let Some(ref yolo) = agent.and_then(|a| a.yolo.as_ref()) {
                        match yolo {
                            crate::agents::YoloMode::CliFlag(flag) => {
                                cmd = format!("{} {}", cmd, flag);
                            }
                            crate::agents::YoloMode::EnvVar(key, value) => {
                                env_vars.push((key, value));
                            }
                            crate::agents::YoloMode::AlwaysYolo => {}
                        }
                    }
                }
                if self.cross_agent_team_pane(target_agent) {
                    match target_agent {
                        "claude" => {
                            if let Some(flag) = self.claude_cross_agent_team_flag() {
                                cmd = format!("{} {}", cmd, flag);
                            }
                        }
                        "codex" => {
                            let base = self.pane_base_command(target_agent);
                            cmd = self.codex_xats_bootstrap_command(&cmd, &base);
                        }
                        _ => {}
                    }
                }
                if let Some(key) = self.xats_identity_key_for_pane(is_primary, slot_identity_key) {
                    env_vars.push((XATS_IDENTITY_KEY_ENV, key));
                }
                if self.expects_shell() && env_vars.is_empty() {
                    let escaped_dir = shell_escape(&self.project_path);
                    let shell = crate::session::environment::user_posix_shell();
                    let inner = format!("cd {escaped_dir} && stty susp undef; exec {cmd}");
                    let escaped_inner = inner.replace('\'', "'\\''");
                    return Some(format!("{shell} -lc '{escaped_inner}'"));
                }
                Some(wrap_command_ignore_suspend_with_env(&cmd, &env_vars))
            }
        }
    }

    /// Build the bare tool command (binary + resume/fork/session-id flags +
    /// extra args) for a single pane before launch-context decoration.
    ///
    /// For the instance's own agent pane (`is_primary = true`, already narrowed
    /// by the caller to a pane whose agent is `self.tool`) this honors the
    /// instance command override (`self.command`), `extra_args`, pre-allocated
    /// session id, and fork token, matching the single-pane start/respawn path
    /// byte-for-byte. For every other pane (`is_primary = false`) those
    /// instance-primary concepts do not apply: the command is built from the
    /// pane agent's own binary plus, when present, a resume flag from the
    /// supplied token.
    fn build_base_pane_command(
        &self,
        agent: Option<&crate::agents::AgentDef>,
        resume_token: Option<&str>,
        is_primary: bool,
    ) -> String {
        if !is_primary {
            let mut cmd = agent.map_or_else(|| "bash".to_string(), |a| a.binary.to_string());
            if let (Some(token), Some(resume)) =
                (resume_token, agent.and_then(|a| a.resume.as_ref()))
            {
                let resume_flag = resume.resume_flag.replace("{}", token);
                cmd = format!("{} {}", cmd, resume_flag);
            }
            if agent.is_some_and(|a| a.name == "codex") {
                cmd.push_str(&self.codex_hook_env_overrides());
            }
            return cmd;
        }

        let mut cmd = self.get_tool_command().to_string();
        if let Some(token) = resume_token {
            // A live resume token always wins: once the forked session has spawned
            // and AoE has captured its own post-fork session id, subsequent restarts
            // go through the normal resume path.
            if let Some(resume) = agent
                .and_then(|a| a.resume.as_ref())
                .filter(|_| !self.has_command_override())
            {
                let resume_flag = resume.resume_flag.replace("{}", token);
                cmd = format!("{} {}", cmd, resume_flag);
            }
        } else if let Some(fork_token) = self.fork_pending.as_deref() {
            // First launch of a forked session: use the agent's native fork command
            // with the parent's session token as the source. For Claude we also
            // pre-allocate a new session-id for the fork (like agent-deck does).
            if let Some(template) = agent
                .and_then(|a| a.fork_template)
                .filter(|_| !self.has_command_override())
            {
                let fork_flag = template.replace("{}", fork_token);
                cmd = format!("{} {}", cmd, fork_flag);
            }
            if let (Some(new_id), Some(flag)) = (
                self.agent_session_id.as_deref(),
                agent.and_then(|a| a.session_id_flag),
            ) {
                let id_flag = flag.replace("{}", new_id);
                cmd = format!("{} {}", cmd, id_flag);
            }
        } else if let Some(session_id) = self.agent_session_id.as_deref() {
            // Fresh launch with pre-allocated session identity.
            if let Some(flag) = agent
                .and_then(|a| a.session_id_flag)
                .filter(|_| !self.has_command_override())
            {
                let id_flag = flag.replace("{}", session_id);
                cmd = format!("{} {}", cmd, id_flag);
            }
        }
        if !self.extra_args.is_empty() {
            cmd = format!("{} {}", cmd, self.extra_args);
        }
        // A command override is the user's own program, not necessarily Codex,
        // so it does not get Codex flags appended to it.
        if agent.is_some_and(|a| a.name == "codex") && !self.has_command_override() {
            cmd.push_str(&self.codex_hook_env_overrides());
        }
        cmd
    }

    /// Build a new Instance that will, on its first launch, execute this agent's
    /// native fork-session command against `self` as the parent. Runtime state
    /// (status, timestamps, resume token) is cleared; persistent configuration
    /// (tool, group, worktree, sandbox) is inherited.
    ///
    /// The forked session reuses the parent's `project_path` so both the agent
    /// pane and the optional right shell pane land in the same working directory.
    /// Inherited worktree metadata is marked `cleanup_on_delete = false` so
    /// deleting the fork never destroys the parent's worktree.
    pub fn create_fork(&self, new_title: String, new_group: Option<String>) -> Result<Self> {
        let fork_token = self.fork_token()?;

        let mut fork = self.clone();
        fork.id = generate_id();
        fork.title = new_title;
        fork.parent_session_id = Some(self.id.clone());
        fork.fork_pending = Some(fork_token);

        if let Some(group) = new_group {
            fork.group_path = group;
        }

        // Clear runtime state — the fork is a fresh process lifecycle.
        fork.status = Status::Idle;
        fork.created_at = Utc::now();
        fork.last_accessed_at = None;
        fork.resume_token = None;
        fork.restart_in_flight = false;
        // A fork is a distinct collaborator. Inheriting the parent's identity key
        // would let both panes recover into one xats identity, and the daemon
        // cannot tell that apart from the parent legitimately restarting: the
        // later caller silently takes the identity and the earlier one goes quiet.
        fork.xats_identity_key = None;
        // Pre-allocate a new session UUID for the fork if the tool supports it.
        // This is passed via `--session-id <uuid>` alongside the fork template.
        fork.agent_session_id = crate::agents::get_agent(&self.tool)
            .filter(|a| a.session_id_flag.is_some())
            .map(|_| Uuid::new_v4().to_string());
        fork.last_error = None;
        fork.last_error_check = None;
        fork.last_start_time = None;
        fork.last_spinner_seen = None;
        fork.spike_start = None;
        fork.pre_spike_status = None;
        fork.acknowledged = false;
        fork.terminal_info = None;

        // Inherit worktree without taking ownership of cleanup: the parent
        // still relies on it.
        if let Some(ref mut wt) = fork.worktree_info {
            wt.cleanup_on_delete = false;
        }
        if let Some(ref mut ws) = fork.workspace_info {
            ws.cleanup_on_delete = false;
        }

        // Give the fork its own container name derived from the new id so it
        // does not collide with the parent's container.
        if let Some(ref mut sandbox) = fork.sandbox_info {
            sandbox.container_name = DockerContainer::generate_name(&fork.id);
            sandbox.container_id = None;
            sandbox.created_at = None;
        }

        Ok(fork)
    }

    /// Resolve the parent agent's session token to be used for forking.
    /// Returns an error for tools that do not support forking, for instances
    /// with a user-supplied command override, or for forkable tools that have
    /// not yet produced a session id AoE can capture.
    fn fork_token(&self) -> Result<String> {
        let agent = crate::agents::get_agent(&self.tool)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", self.tool))?;
        if agent.fork_template.is_none() {
            anyhow::bail!(
                "Fork is not supported for agent '{}'. Supported: claude, codex, opencode.",
                self.tool
            );
        }
        if self.has_command_override() {
            anyhow::bail!(
                "Cannot fork a session with a custom command override (command = {:?})",
                self.command
            );
        }

        match self.tool.as_str() {
            "claude" => self
                .agent_session_id
                .clone()
                .or_else(|| self.resume_token.clone())
                .or_else(|| crate::hooks::read_hook_session_id(&self.id))
                .or_else(|| resolve_claude_session_from_disk(&self.project_path))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No active Claude session found. Start and interact with the parent session, then try again."
                    )
                }),
            "codex" => self
                .agent_session_id
                .clone()
                .or_else(|| self.resume_token.clone())
                .or_else(|| crate::hooks::read_hook_session_id(&self.id))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No active codex session to fork yet. Press 'R' (resume restart) on the parent to capture a resume token, then try again."
                    )
                }),
            "opencode" => self.resolve_opencode_session_id(),
            other => anyhow::bail!("Fork is not supported for agent '{}'", other),
        }
    }

    /// Look up the parent opencode session id by querying
    /// `opencode session list --format json` (either on the host or inside the
    /// parent's container, depending on whether the parent is sandboxed), then
    /// picking the most recently updated session whose directory matches
    /// `self.project_path`.
    ///
    /// `resolve_claude_session_from_disk` is a companion free function for Claude.
    fn resolve_opencode_session_id(&self) -> Result<String> {
        use std::process::Command;

        let output = if self.is_sandboxed() {
            let container = DockerContainer::from_session_id(&self.id);
            if !container.is_running().unwrap_or(false) {
                anyhow::bail!(
                    "Cannot fork opencode session: parent container '{}' is not running. \
                     Start the parent session before forking.",
                    container.name
                );
            }
            Command::new("docker")
                .args([
                    "exec",
                    &container.name,
                    "opencode",
                    "session",
                    "list",
                    "--format",
                    "json",
                ])
                .output()
        } else {
            Command::new("opencode")
                .args(["session", "list", "--format", "json"])
                .current_dir(&self.project_path)
                .output()
        };

        let output =
            output.map_err(|e| anyhow::anyhow!("Failed to run `opencode session list`: {}", e))?;
        if !output.status.success() {
            anyhow::bail!(
                "`opencode session list` exited non-zero: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        #[derive(Deserialize)]
        struct OpenCodeSession {
            id: String,
            #[serde(default)]
            directory: Option<String>,
            #[serde(default)]
            path: Option<String>,
            #[serde(default)]
            updated: Option<i64>,
            #[serde(default)]
            created: Option<i64>,
        }

        let sessions: Vec<OpenCodeSession> = serde_json::from_slice(&output.stdout)
            .map_err(|e| anyhow::anyhow!("Failed to parse opencode session list: {}", e))?;

        let target = match std::fs::canonicalize(&self.project_path) {
            Ok(p) => p,
            Err(_) => std::path::PathBuf::from(&self.project_path),
        };

        let best = sessions
            .into_iter()
            .filter_map(|s| {
                let dir = s.directory.clone().or_else(|| s.path.clone())?;
                let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone().into());
                if canonical == target {
                    let ts = s.updated.or(s.created).unwrap_or(0);
                    Some((ts, s.id))
                } else {
                    None
                }
            })
            .max_by_key(|(ts, _)| *ts)
            .map(|(_, id)| id);

        best.ok_or_else(|| {
            anyhow::anyhow!(
                "No opencode session found for directory {}. Start or interact with opencode in this directory before forking.",
                self.project_path
            )
        })
    }

    fn resolve_on_launch_hooks(&self) -> Option<Vec<String>> {
        let profile = super::config::resolve_default_profile();
        let mut resolved = super::profile_config::resolve_config(&profile)
            .map(|c| c.hooks.on_launch)
            .unwrap_or_default();

        match super::repo_config::check_hook_trust(Path::new(&self.project_path)) {
            Ok(super::repo_config::HookTrustStatus::Trusted(hooks))
                if !hooks.on_launch.is_empty() =>
            {
                resolved = hooks.on_launch.clone();
            }
            _ => {}
        }

        if resolved.is_empty() {
            None
        } else {
            Some(resolved)
        }
    }

    fn execute_on_launch_hooks(&self, hook_cmds: &[String]) {
        if self.is_sandboxed() {
            if let Some(ref sandbox) = self.sandbox_info {
                let workdir = self.container_workdir();
                if let Err(e) = super::repo_config::execute_hooks_in_container(
                    hook_cmds,
                    &sandbox.container_name,
                    &workdir,
                ) {
                    tracing::warn!("on_launch hook failed in container: {}", e);
                }
            }
        } else if let Err(e) =
            super::repo_config::execute_hooks(hook_cmds, Path::new(&self.project_path))
        {
            tracing::warn!("on_launch hook failed: {}", e);
        }
    }

    /// Respawn only the agent pane, preserving the tmux session layout.
    /// Runs on-launch hooks, rebuilds the agent command, and respawns the pane.
    pub fn respawn_agent_pane(&mut self) -> Result<()> {
        self.respawn_single_pane(RestartMode::Resume)
    }

    /// Respawn the single primary `@aoe_agent_pane` fresh, without ever consulting
    /// or injecting the instance's stored `resume_token`. Used by the fresh
    /// restart action's no-tracked-slots fallback so a fresh restart never
    /// reinjects history.
    pub fn respawn_agent_pane_fresh(&mut self) -> Result<()> {
        self.respawn_single_pane(RestartMode::Fresh)
    }

    /// For a fresh restart, agents that pre-allocate a conversation id via
    /// `session_id_flag` (e.g. Claude `--session-id`) must NOT reuse the current
    /// `agent_session_id`: the just-killed conversation still owns it and the
    /// agent refuses to start with a session id that is already in use. Allocate a
    /// new UUID so the fresh restart begins a brand-new conversation. No-op for
    /// agents without `session_id_flag` or when a command override is in effect.
    fn reallocate_session_id_for_fresh(&mut self) {
        if self.has_command_override() {
            return;
        }
        let uses_session_id = crate::agents::get_agent(&self.tool)
            .and_then(|a| a.session_id_flag)
            .is_some();
        if uses_session_id {
            self.agent_session_id = Some(Uuid::new_v4().to_string());
        }
    }

    /// Speculatively prepare a brand-new conversation identity for a fresh
    /// restart: reallocate the pre-allocated `--session-id` and drop any persisted
    /// `fork_pending` (a fresh restart must never re-fork a parent). Returns the
    /// previous `(agent_session_id, fork_pending)` so the caller can roll back if
    /// the respawn never actually starts. Returns `None` for `RestartMode::Resume`,
    /// which leaves identity untouched.
    fn begin_fresh_identity(&mut self, mode: RestartMode) -> Option<FreshIdentitySnapshot> {
        if mode != RestartMode::Fresh {
            return None;
        }
        let snapshot = (self.agent_session_id.clone(), self.fork_pending.clone());
        self.reallocate_session_id_for_fresh();
        self.fork_pending = None;
        Some(snapshot)
    }

    /// Roll back a fresh identity prepared by [`begin_fresh_identity`] when the
    /// respawn failed, so a never-launched session id or an abandoned fork token is
    /// not persisted. A successful respawn keeps the new identity (no-op here).
    fn rollback_fresh_identity_on_failure(
        &mut self,
        snapshot: Option<FreshIdentitySnapshot>,
        success: bool,
    ) {
        if let Some((prev_id, prev_fork)) = snapshot {
            if !success {
                self.agent_session_id = prev_id;
                self.fork_pending = prev_fork;
            }
        }
    }

    fn respawn_single_pane(&mut self, mode: RestartMode) -> Result<()> {
        // A fresh restart must not reuse the pre-allocated `--session-id` or re-fork
        // a persisted parent; prepare a new identity, then commit it only if the
        // respawn succeeds (roll back on failure so a phantom id is not persisted).
        let snapshot = self.begin_fresh_identity(mode);
        let result = self.respawn_single_pane_inner(mode);
        self.rollback_fresh_identity_on_failure(snapshot, result.is_ok());
        result
    }

    fn respawn_single_pane_inner(&mut self, mode: RestartMode) -> Result<()> {
        self.ensure_xats_identity_key();
        // `Fresh` bypasses `resolved_resume_token` entirely so the command never
        // carries the stored `resume_token`; `Resume` keeps the existing fallback.
        let effective_resume_token = match mode {
            RestartMode::Resume => self.resolved_resume_token(None),
            RestartMode::Fresh => None,
        };
        let session = self.tmux_session()?;
        if !session.exists() {
            anyhow::bail!("Session does not exist");
        }

        if let Some(ref hook_cmds) = self.resolve_on_launch_hooks() {
            self.execute_on_launch_hooks(hook_cmds);
        }

        // Nothing here belongs to an agent the instance's tool does not
        // describe: no resume token was ever recorded for it, and the
        // instance's own session id and fork token name a different agent's
        // conversation. A fresh launch of the agent that is actually in the pane
        // beats a resumed launch of one that is not.
        let pane_agent = self.pane_agent_overriding_instance_tool();
        let cmd = match pane_agent {
            Some(agent) => self.build_pane_command(agent, None, false, None),
            None => self.build_agent_command(effective_resume_token.as_deref()),
        }
        .ok_or_else(|| anyhow::anyhow!("No agent command available"))?;

        session.kill_agent_pane_process_tree();
        session.respawn_agent_pane(&cmd, &self.project_path, !self.expects_shell())?;

        self.run_auto_confirm(pane_agent.unwrap_or(&self.tool));

        self.apply_tmux_options(&Self::current_profile());

        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());
        self.clear_resume_token();

        Ok(())
    }

    /// Restart every tracked agent pane of this instance from the persisted
    /// `agent_slot` store. Each pane is killed and respawned; in
    /// [`RestartMode::Resume`] the command is built from the pane's own
    /// `native_session_id` (a pane that cannot resume degrades to a fresh restart
    /// of that pane only), while [`RestartMode::Fresh`] forces the no-resume path
    /// for every pane (full launch context, no resume flag). A per-pane failure
    /// does not abort the remaining panes. Returns the per-pane outcomes (one per
    /// slot). When the instance has no tracked slots the caller falls back to the
    /// single-pane respawn behavior.
    pub fn resume_all_tracked_panes(
        &mut self,
        slots: &[crate::db::AgentSlot],
        mode: RestartMode,
    ) -> Vec<PaneResumeOutcome> {
        self.status = Status::Restarting;
        self.last_error = None;
        self.ensure_xats_identity_key();

        // A fresh restart must not reuse the pre-allocated `--session-id` for the
        // primary pane (slot 0 builds with the instance's `agent_session_id`) nor
        // re-fork a persisted parent. Prepare a new identity, but commit it only if
        // slot 0 actually respawns (roll back otherwise, so a phantom id/fork is not
        // persisted by the subsequent save).
        let snapshot = self.begin_fresh_identity(mode);

        if let Some(ref hook_cmds) = self.resolve_on_launch_hooks() {
            self.execute_on_launch_hooks(hook_cmds);
        }

        let mut outcomes = Vec::with_capacity(slots.len());
        let mut primary_respawned = false;
        let mut confirmable_panes: Vec<String> = Vec::new();
        for slot in slots {
            let outcome = self.resume_launch_pane(
                &slot.agent,
                &slot.native_session_id,
                &slot.tmux_pane,
                &slot.cwd,
                slot.slot == 0,
                mode,
                Some(slot.xats_identity_key.as_str()),
            );
            // Every Claude pane this fan-out actually relaunched raises its own
            // startup screens, not just the primary one.
            if slot.agent == "claude" && !matches!(outcome, PaneResumeOutcome::Error(_)) {
                confirmable_panes.push(slot.tmux_pane.clone());
            }
            if slot.slot == 0 && !matches!(outcome, PaneResumeOutcome::Error(_)) {
                primary_respawned = true;
            }
            if let PaneResumeOutcome::Error(ref err) = outcome {
                tracing::warn!(
                    "Failed to resume pane {} (slot {}) for '{}': {}",
                    slot.tmux_pane,
                    slot.slot,
                    self.title,
                    err
                );
            }
            outcomes.push(outcome);
        }

        // Commit the fresh identity only when the primary pane launched; a fresh
        // restart also abandons the stale `resume_token` so a later fork does not
        // reuse the pre-fresh conversation.
        if snapshot.is_some() && primary_respawned {
            self.clear_resume_token();
        }
        self.rollback_fresh_identity_on_failure(snapshot, primary_respawned);

        self.auto_confirm_panes(&confirmable_panes);
        self.apply_tmux_options(&Self::current_profile());

        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());

        outcomes
    }

    /// Whether this instance can be cold-start recovered: it has persisted
    /// `agent_slot` rows but its tmux session no longer exists. Slot presence is
    /// supplied by the caller (read from the store) so detection stays a pure
    /// function of `has_slots` and live tmux state.
    pub fn is_recoverable(&self, has_slots: bool) -> bool {
        is_recoverable_from(
            has_slots,
            self.tmux_session().map(|s| s.exists()).unwrap_or(false),
        )
    }

    /// Rebuild this instance's tmux session from its persisted slots and launch
    /// each pane. The session is recreated through the normal start path so
    /// worktree/sandbox context is restored, then one pane per slot is created in
    /// ascending slot order (slot 0 is the primary `@aoe_agent_pane`, the rest are
    /// split off), each pane is launched via [`resume_launch_pane`], the new pane
    /// ids are written back into `agent_slot.tmux_pane`, and `@aoe_agent_pane` is
    /// re-pinned to slot 0.
    ///
    /// [`RestartMode::Resume`] launches each pane from its `native_session_id`;
    /// [`RestartMode::Fresh`] forces the no-resume path for every pane, keeping
    /// each slot's agent, cwd, and launch context but discarding its conversation.
    /// Fresh recovery runs the same identity transaction as a live fresh restart.
    ///
    /// Per-pane failures are collected into the returned outcomes and never abort
    /// recovery of sibling panes. Once every pane has been launched the rebuild is
    /// given a moment to settle and every slot is checked against the session's
    /// panes (see
    /// [`report_slots_that_did_not_come_back`](Self::report_slots_that_did_not_come_back)),
    /// so a pane that disappears after its relaunch is reported instead of passing
    /// for success. Returns an error only when the session/pane rebuild itself
    /// fails (before any per-pane launch runs) or when the created pane count does
    /// not match the slot count.
    pub fn recover_from_slots(
        &mut self,
        store: &crate::db::Store,
        slots: &[crate::db::AgentSlot],
        mode: RestartMode,
    ) -> Result<Vec<PaneResumeOutcome>> {
        if slots.is_empty() {
            anyhow::bail!("no persisted slots to recover");
        }

        let mut ordered: Vec<crate::db::AgentSlot> = slots.to_vec();
        ordered.sort_by_key(|s| s.slot);

        // Recreate the session shell via the normal start path (restores
        // worktree/sandbox, hooks, tmux options) but WITHOUT launching the agent:
        // every pane, slot 0 included, is launched once by the uniform per-slot
        // loop below. Launching here as well would run the agent against the
        // conversation id being recovered, which it refuses to reopen, and its
        // immediate exit would take the single-pane session down before the real
        // launch runs.
        self.start_placeholder_with_size(crate::terminal::get_size())?;

        // Safe to prepare the fresh identity here: the rebuild above launched no
        // agent, so nothing has claimed the new `--session-id` yet.
        let identity = self.begin_fresh_identity(mode);

        let session_name = tmux::Session::generate_name(&self.id, &self.title);

        // Pair each slot with the new pane created for it, capturing pane ids in
        // slot order at creation time (see `rebuild_recovery_panes`). A slot
        // whose pane could not be created is paired with `None` and surfaced as a
        // per-pane error below without aborting its siblings.
        let paired = rebuild_recovery_panes(&self.title, &session_name, &ordered)?;
        tmux::refresh_session_cache();

        if let Ok(Some(snapshot)) = store.read_layout_snapshot(&self.id) {
            let mapping: std::collections::HashMap<String, String> = paired
                .iter()
                .filter_map(|(slot, pane)| {
                    pane.as_ref()
                        .map(|new_pane| (slot.tmux_pane.clone(), new_pane.clone()))
                })
                .collect();
            match tmux::layout::remap(&snapshot.window_layout, &mapping)
                .and_then(|layout| tmux::apply_window_layout(&session_name, &layout))
            {
                Ok(()) => {}
                Err(e) => tracing::warn!(
                    "Could not restore pane layout for '{}'; using fallback layout: {}",
                    self.title,
                    e
                ),
            }
        }

        let now = crate::db::now_unix();
        let mut outcomes = Vec::with_capacity(paired.len());
        let mut primary_launched = false;
        // Panes this rebuild actually launched Claude into. Collected here rather
        // than derived from `paired` afterwards, because only this loop knows
        // which launches succeeded, and a pane whose launch failed has no Claude
        // in it to raise a startup screen -- handing it to auto-confirm would
        // spend the full timeout waiting for a prompt that cannot come.
        let mut confirmable_panes: Vec<String> = Vec::new();
        for (slot, maybe_pane) in &paired {
            let Some(new_pane) = maybe_pane else {
                outcomes.push(PaneResumeOutcome::Error(format!(
                    "pane creation failed for slot {} (cwd {})",
                    slot.slot, slot.cwd
                )));
                continue;
            };
            let outcome = self.resume_launch_pane(
                &slot.agent,
                &slot.native_session_id,
                new_pane,
                &slot.cwd,
                slot.slot == 0,
                mode,
                Some(slot.xats_identity_key.as_str()),
            );
            if slot.agent == "claude" && !matches!(outcome, PaneResumeOutcome::Error(_)) {
                confirmable_panes.push(new_pane.clone());
            }
            if slot.slot == 0 && !matches!(outcome, PaneResumeOutcome::Error(_)) {
                primary_launched = true;
            }
            if let PaneResumeOutcome::Error(ref err) = outcome {
                tracing::warn!(
                    "Failed to recover pane (slot {}) for '{}': {}",
                    slot.slot,
                    self.title,
                    err
                );
            }
            if let Err(e) = store.upsert_agent_slot(
                &slot.instance_id,
                slot.slot,
                &slot.agent,
                &slot.native_session_id,
                &slot.cwd,
                new_pane,
                &slot.xats_identity_key,
                now,
            ) {
                tracing::error!(
                    "Failed to write back tmux_pane for slot {} of '{}': {}",
                    slot.slot,
                    self.title,
                    e
                );
            }
            outcomes.push(outcome);
        }

        // Commit the fresh identity only when the primary slot launched; a fresh
        // recovery also abandons the stale `resume_token` so a later fork does not
        // reuse the conversation this recovery discarded.
        if identity.is_some() && primary_launched {
            self.clear_resume_token();
        }
        self.rollback_fresh_identity_on_failure(identity, primary_launched);

        // Re-pin @aoe_agent_pane to slot 0's pane (always created: the primary
        // pane) so reconcile and the `R` resume-all flow keep operating on the
        // rebuilt session.
        if let Some((_, Some(primary_pane))) = paired.first() {
            if let Err(e) = tmux::set_agent_pane_id(&session_name, primary_pane) {
                tracing::error!(
                    "Failed to re-pin @aoe_agent_pane for '{}': {}",
                    self.title,
                    e
                );
            }
        }

        self.report_slots_that_did_not_come_back(store, &session_name, &paired, &mut outcomes, now);

        self.auto_confirm_panes(&confirmable_panes);
        self.apply_tmux_options(&Self::current_profile());
        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());

        Ok(outcomes)
    }

    /// Once every slot has been launched, check that each one still has a pane in
    /// the rebuilt session and turn each slot that does not into a per-pane
    /// failure. A pane that is created, respawned and then disappears is invisible
    /// to the launch outcomes, which is how recovery used to hand back fewer panes
    /// than the user had and still report success.
    ///
    /// Each missing slot is also appended to the instance's event log next to the
    /// `adopt`/`capture` entries that recorded it in the first place, so the fact
    /// outlives the in-memory outcome the caller shows once.
    ///
    /// The check deliberately neither retries nor recreates a missing pane:
    /// recovery's job here is to be honest about what came back, and a retry would
    /// mask whatever removed it.
    fn report_slots_that_did_not_come_back(
        &self,
        store: &crate::db::Store,
        session_name: &str,
        paired: &[(crate::db::AgentSlot, Option<String>)],
        outcomes: &mut [PaneResumeOutcome],
        now: i64,
    ) {
        std::thread::sleep(recovery_settle());
        tmux::refresh_session_cache();

        let live: std::collections::HashSet<String> =
            crate::db::reconcile::session_pane_ids(session_name)
                .into_iter()
                .collect();

        for (index, report) in missing_slot_failures(paired, &live) {
            tracing::warn!("Recovery of '{}' lost a pane: {}", self.title, report);
            let slot = &paired[index].0;
            if let Err(e) = store.append_event(
                &slot.instance_id,
                Some(slot.slot),
                "lost",
                Some(&report),
                now,
            ) {
                tracing::error!(
                    "Failed to record lost pane for slot {} of '{}': {}",
                    slot.slot,
                    self.title,
                    e
                );
            }
            // A launch that already failed said why, which is more specific than
            // "it is not here"; keep that reason and only fill in the silent case.
            if !matches!(outcomes[index], PaneResumeOutcome::Error(_)) {
                outcomes[index] = PaneResumeOutcome::Error(report);
            }
        }
    }

    fn clear_resume_token(&mut self) {
        self.resume_token = None;
    }

    fn resolved_resume_token(&self, resume_token: Option<&str>) -> Option<String> {
        resume_token
            .map(std::string::ToString::to_string)
            .or_else(|| self.resume_token.clone())
    }

    fn apply_tmux_options(&self, profile: &str) {
        let name = tmux::Session::generate_name(&self.id, &self.title);
        self.apply_session_tmux_options(&name, &self.title, profile);
        if self.tool == "codex" {
            if let Err(e) = tmux::status_bar::ensure_codex_title_monitor(&name, &self.title) {
                tracing::debug!("Failed to refresh Codex title monitor: {}", e);
            }
        }
    }

    pub fn refresh_agent_tmux_options(&self, profile: &str) {
        self.apply_tmux_options(profile);
    }

    pub fn get_container_for_instance(&mut self) -> Result<containers::DockerContainer> {
        let sandbox = self
            .sandbox_info
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Cannot ensure container for non-sandboxed session"))?;

        let image = &sandbox.image;
        let container = DockerContainer::new(&self.id, image);

        if container.is_running()? {
            container_config::refresh_agent_configs();
            return Ok(container);
        }

        if container.exists()? {
            container_config::refresh_agent_configs();
            container.start()?;
            return Ok(container);
        }

        // Ensure image is available (always pulls to get latest)
        let runtime = containers::get_container_runtime();
        runtime.ensure_image(image)?;

        let config = self.build_container_config()?;
        let container_id = container.create(&config)?;

        if let Some(ref mut sandbox) = self.sandbox_info {
            sandbox.container_id = Some(container_id);
            sandbox.created_at = Some(Utc::now());
        }

        Ok(container)
    }

    /// Get the container working directory for this instance.
    pub fn container_workdir(&self) -> String {
        container_config::compute_volume_paths(Path::new(&self.project_path), &self.project_path)
            .map(|(_, wd)| wd)
            .unwrap_or_else(|_| "/workspace".to_string())
    }

    fn build_container_config(&self) -> Result<crate::containers::ContainerConfig> {
        container_config::build_container_config(
            &self.project_path,
            self.sandbox_info.as_ref().unwrap(),
            &self.tool,
            self.is_yolo_mode(),
            &self.id,
            self.workspace_info.as_ref(),
        )
    }

    pub fn restart(&mut self) -> Result<()> {
        self.restart_with_size(None)
    }

    pub fn restart_with_size(&mut self, size: Option<(u16, u16)>) -> Result<()> {
        let session = self.tmux_session()?;

        if session.exists() {
            session.kill()?;
        }

        // Small delay to ensure tmux cleanup
        std::thread::sleep(std::time::Duration::from_millis(100));

        self.start_with_size(size)
    }

    pub fn kill(&self) -> Result<()> {
        let session = self.tmux_session()?;
        if session.exists() {
            session.kill()?;
        }
        Ok(())
    }

    /// Stop the session: kill the tmux session and stop the Docker container
    /// (if sandboxed). The container is stopped but not removed, so it can be
    /// restarted on re-attach.
    pub fn stop(&self) -> Result<()> {
        self.kill()?;

        if self.is_sandboxed() {
            let container = containers::DockerContainer::from_session_id(&self.id);
            if container.is_running().unwrap_or(false) {
                container.stop()?;
            }
        }

        crate::hooks::cleanup_hook_status_dir(&self.id);

        Ok(())
    }

    pub fn update_status(&mut self) {
        self.update_status_with_options(StatusUpdateOptions::default());
    }

    pub fn update_status_with_options(&mut self, options: StatusUpdateOptions) {
        if matches!(
            self.status,
            Status::Stopped | Status::Restarting | Status::Deleting
        ) {
            return;
        }

        if self.status == Status::Error {
            if let Some(last_check) = self.last_error_check {
                if last_check.elapsed().as_secs() < 30 {
                    return;
                }
            }
        }

        if let Some(start_time) = self.last_start_time {
            if start_time.elapsed().as_secs() < 3 {
                self.status = Status::Starting;
                return;
            }
        }

        let session = match self.tmux_session() {
            Ok(s) => s,
            Err(_) => {
                self.status = Status::Error;
                self.last_error_check = Some(std::time::Instant::now());
                return;
            }
        };

        if !session.exists() {
            self.status = Status::Error;
            self.last_error_check = Some(std::time::Instant::now());
            return;
        }

        let previous_status = self.status;
        let now = Instant::now();

        // --- Detect status for the primary (AoE-created) agent pane ---
        let mut primary_status: Option<Status> = None;

        // Check hook-based status first (more reliable than tmux pane parsing).
        // Only short-circuit when the hook file is fresh: a stale file means
        // the agent missed a `Stop` event (Esc, client-side slash command,
        // crash) and we must fall through to content detection instead of
        // pinning the session to the last hook-reported state.
        match crate::hooks::read_hook_status_with_freshness(&self.id) {
            Some(read) if read.fresh => {
                tracing::trace!("hook status detection '{}': {:?}", self.title, read.status);
                self.clear_spike_state();
                // Trust hook status over shell detection. Wrapper scripts (e.g.
                // Devbox, version managers) run agents via a shell process, so
                // `is_pane_running_shell()` returns true even though the agent is
                // healthy. Only check if the pane is actually dead.
                primary_status = Some(if session.is_pane_dead() {
                    Status::Error
                } else {
                    read.status
                });
            }
            Some(read) => {
                tracing::debug!(
                    "hook stale for '{}' (id={}, value={:?}, age={}s); falling through to content detection",
                    self.title,
                    self.id,
                    read.status,
                    read.age.as_secs()
                );
            }
            None => {}
        }

        let session_name = tmux::Session::generate_name(&self.id, &self.title);

        if primary_status.is_none() {
            if let Some(detected) = tmux::get_cached_pane_info(&session_name)
                .and_then(|info| tmux::status_detection::detect_status_from_title(&info.pane_title))
            {
                self.clear_spike_state();
                self.last_spinner_seen = Some(now);
                primary_status = Some(detected);
            }
        }

        if primary_status.is_none() {
            // When this is a shell session and a previous detach cached an
            // inner agent (e.g. user ran `claude` inside the shell and
            // detached), dispatch to that agent's content detector instead
            // of the shell stub. The capture uses the same cached 50-line
            // capture path as `session.detect_status` to avoid a double
            // capture on the same poll cycle.
            let inner_agent = if self.tool == "shell" {
                self.detected_inner_agent.clone()
            } else {
                None
            };

            let mut detected = if options.allow_capture {
                match inner_agent.as_deref() {
                    Some(agent) => match session.capture_pane_cached(50) {
                        Ok(content) => {
                            let fg_pid = session.get_foreground_pid();
                            tmux::status_detection::detect_status_from_content(
                                &content, agent, fg_pid,
                            )
                        }
                        Err(_) => Status::Idle,
                    },
                    None => match session.detect_status(&self.tool) {
                        Ok(status) => status,
                        Err(_) => Status::Idle,
                    },
                }
            } else {
                options.reused_status.unwrap_or(previous_status)
            };
            tracing::trace!(
                "status detection '{}' (tool={}, inner_agent={:?}, custom_cmd={}, allow_capture={}): {:?}",
                self.title,
                self.tool,
                inner_agent,
                self.has_custom_command(),
                options.allow_capture,
                detected
            );

            if options.allow_capture && detected == Status::Running {
                self.last_spinner_seen = Some(now);
            }

            if options.allow_capture {
                detected = self.apply_spike_detection(detected, previous_status, now);
                detected = self.apply_spinner_grace_period(detected, previous_status, now);
            }

            // Apply shell/dead heuristics for single-pane sessions.
            // When `detected_inner_agent` is Some, we trust the detected
            // agent's content detector: a concrete `Idle` from (e.g.)
            // claude must surface as `Idle`, not be rewritten to `Unknown`
            // by the shell-tool heuristic.
            let is_single_pane = session.pane_count() <= 1;
            let is_shell_stale =
                || is_single_pane && !self.expects_shell() && session.is_pane_running_shell();
            detected = match detected {
                Status::Idle if inner_agent.is_some() => {
                    if session.is_pane_dead() {
                        Status::Error
                    } else {
                        Status::Idle
                    }
                }
                Status::Idle if self.has_custom_command() => {
                    if session.is_pane_dead() || is_shell_stale() {
                        Status::Error
                    } else {
                        Status::Unknown
                    }
                }
                Status::Idle if session.is_pane_dead() || is_shell_stale() => Status::Error,
                other => other,
            };

            primary_status = Some(detected);
        }

        let primary_status = primary_status.unwrap_or(Status::Idle);

        // --- Detect status for extra (user-split) panes and aggregate ---
        let extra_pane_statuses =
            self.detect_extra_pane_statuses(&session_name, options.allow_capture);
        let aggregated = if extra_pane_statuses.is_empty() {
            primary_status
        } else {
            let mut all_statuses = vec![primary_status];
            all_statuses.extend(extra_pane_statuses);
            tmux::status_detection::aggregate_pane_statuses(&all_statuses)
        };

        self.status = self.apply_acknowledged_mapping(aggregated);
        self.last_error = None;
    }

    /// Detect status for extra (user-split) panes beyond the primary agent pane.
    /// Returns statuses only for panes identified as running a known agent (not shell).
    fn detect_extra_pane_statuses(&self, session_name: &str, allow_capture: bool) -> Vec<Status> {
        let all_panes = match tmux::get_all_cached_pane_infos(session_name) {
            Some(panes) if panes.len() > 1 => panes,
            _ => return Vec::new(),
        };

        // Skip pane index 0 (or whichever is the primary agent pane)
        let extra_panes: Vec<_> = all_panes.into_iter().skip(1).collect();
        let mut statuses = Vec::new();

        for pane_info in &extra_panes {
            let agent_type = match tmux::status_detection::detect_agent_type_from_pane(pane_info) {
                Some("shell") | None => continue,
                Some(agent) => agent,
            };

            // Title-based detection (fast, no capture needed)
            if let Some(status) =
                tmux::status_detection::detect_status_from_title(&pane_info.pane_title)
            {
                statuses.push(status);
                continue;
            }

            // Content-based detection (requires capture)
            if allow_capture {
                if let Ok(content) = tmux::Session::capture_pane_by_id(&pane_info.pane_id, 50) {
                    let status = tmux::status_detection::detect_status_from_content(
                        &content, agent_type, None,
                    );
                    statuses.push(status);
                    continue;
                }
            }

            statuses.push(Status::Idle);
        }

        statuses
    }

    fn clear_spike_state(&mut self) {
        self.spike_start = None;
        self.pre_spike_status = None;
    }

    fn apply_acknowledged_mapping(&self, status: Status) -> Status {
        if status == Status::Waiting && self.acknowledged {
            Status::Idle
        } else {
            status
        }
    }

    fn apply_spike_detection(
        &mut self,
        detected: Status,
        previous_status: Status,
        now: Instant,
    ) -> Status {
        if detected != Status::Running {
            self.clear_spike_state();
            return detected;
        }

        if previous_status == Status::Running {
            self.clear_spike_state();
            return Status::Running;
        }

        if self.spike_start.is_some() {
            self.clear_spike_state();
            return Status::Running;
        }

        self.spike_start = Some(now);
        self.pre_spike_status = Some(previous_status);
        previous_status
    }

    fn apply_spinner_grace_period(
        &mut self,
        detected: Status,
        previous_status: Status,
        now: Instant,
    ) -> Status {
        if previous_status == Status::Running
            && detected != Status::Running
            && self
                .last_spinner_seen
                .is_some_and(|seen| now.duration_since(seen) <= Duration::from_millis(500))
        {
            Status::Running
        } else {
            detected
        }
    }

    pub fn capture_output_with_size(
        &self,
        lines: usize,
        width: u16,
        height: u16,
    ) -> Result<String> {
        let session = self.tmux_session()?;
        session.capture_pane_with_size(lines, Some(width), Some(height))
    }
}

fn generate_id() -> String {
    Uuid::new_v4().to_string().replace("-", "")[..16].to_string()
}

/// Format an environment variable assignment as a shell-safe command prefix.
///
/// Uses `shell_escape` (single-quote escaping) so the value is preserved
/// verbatim when parsed by the inner `bash -c '...'` shell created by
/// `wrap_command_ignore_suspend`.
#[cfg(test)]
fn format_env_var_prefix(key: &str, value: &str, cmd: &str) -> String {
    let escaped = shell_escape(value);
    format!("{}={} {}", key, escaped, cmd)
}

/// Wrap a command to disable Ctrl-Z (SIGTSTP) suspension.
///
/// When running agents directly as tmux session commands (without a parent shell),
/// pressing Ctrl-Z suspends the process with no way to recover via job control.
/// This wrapper disables the suspend character at the terminal level before exec'ing
/// the actual command.
///
/// Uses POSIX-standard `stty susp undef` which works on both Linux and macOS.
/// Single quotes in `cmd` are escaped with the `'\''` technique to prevent
/// breaking out of the outer shell wrapper.
///
/// Environment variables are exported before `exec` because `exec VAR=val cmd`
/// is not portable and fails in many shells.
/// Scan the Claude Code projects directory for the most recently modified
/// session file (`.jsonl`) whose project hash matches `project_path`. Returns
/// the UUID portion of the filename (the bare session id) or `None`.
///
/// Claude Code stores session data under `~/.claude/projects/<hash>/` where
/// `<hash>` is the absolute path with `/` replaced by `-`.
fn resolve_claude_session_from_disk(project_path: &str) -> Option<String> {
    let canonical = std::fs::canonicalize(project_path)
        .unwrap_or_else(|_| std::path::PathBuf::from(project_path));
    let path_str = canonical.to_string_lossy();
    // Claude uses the path with `/` replaced by `-` as the project directory
    // name (the leading `-` comes from the initial `/`).
    let project_hash = path_str.replace('/', "-");
    let claude_dir = dirs::home_dir()?
        .join(".claude")
        .join("projects")
        .join(&project_hash);

    if !claude_dir.is_dir() {
        return None;
    }

    let mut best: Option<(std::time::SystemTime, String)> = None;
    if let Ok(entries) = std::fs::read_dir(&claude_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.ends_with(".jsonl") {
                continue;
            }
            let uuid = name_str.trim_end_matches(".jsonl").to_string();
            // Quick sanity check: Claude session ids are UUID-shaped (contains hyphens).
            if !uuid.contains('-') {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if best.as_ref().map_or(true, |(t, _)| modified > *t) {
                        best = Some((modified, uuid));
                    }
                }
            }
        }
    }

    best.map(|(_, id)| id)
}

fn wrap_command_ignore_suspend(cmd: &str) -> String {
    wrap_command_ignore_suspend_with_env(cmd, &[])
}

fn wrap_command_ignore_suspend_with_env(cmd: &str, env_vars: &[(&str, &str)]) -> String {
    let shell = crate::session::environment::user_posix_shell();
    let escaped = cmd.replace('\'', "'\\''");
    // Place env vars before the shell so they're parsed at the outer shell
    // level, avoiding quoting conflicts with the inner single-quoted string.
    let env_prefix = env_vars
        .iter()
        .map(|(k, v)| {
            let escaped_v = v.replace('\'', "'\\''");
            format!("{}='{}' ", k, escaped_v)
        })
        .collect::<String>();
    // Use login shell (-l) so version-manager PATHs (NVM, etc.) are available.
    format!(
        "{}{} -lc 'stty susp undef; exec env {}'",
        env_prefix, shell, escaped
    )
}

/// Whether creating a session also launches its agent. Recovery creates a
/// placeholder because it launches every pane itself, from that pane's durable
/// slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionLaunch {
    Agent,
    Placeholder,
}

/// Whether the multi-pane fan-out resumes each pane from its persisted
/// `native_session_id` (`Resume`) or restarts every pane fresh with no resume
/// flag (`Fresh`). `Fresh` forces the no-resume path for every pane while still
/// building the full launch context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartMode {
    Resume,
    Fresh,
}

/// Snapshot of the identity fields a fresh restart speculatively mutates
/// (`agent_session_id`, `fork_pending`), captured so the restart can roll them
/// back if the respawn never actually starts a new conversation.
type FreshIdentitySnapshot = (Option<String>, Option<String>);

/// Outcome of resuming a single tracked pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneResumeOutcome {
    /// Pane respawned with a resume command built from the persisted id.
    Resumed,
    /// Pane respawned fresh (no resume flag): empty id, agent without
    /// `ResumeConfig`, or unknown agent.
    DegradedToFresh,
    /// Respawn failed; the error message is recorded for the caller.
    Error(String),
}

/// Whether a string is safe to use as a bare command token (binary name) in a
/// tmux respawn command. tmux runs the respawn argument through a shell, so a
/// recorded value with shell metacharacters must never be executed.
fn is_safe_command_token(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl Instance {
    /// Build the launch command for one tracked pane from its recorded agent and
    /// persisted native session id, decorating it with the instance's full launch
    /// context through [`build_pane_command`](Self::build_pane_command). Returns
    /// `Some((command, resumed))` where `resumed` is true only when a resume flag
    /// was appended.
    ///
    /// A known agent with an empty/invalid `native_session_id`, or one that lacks
    /// a `ResumeConfig`, degrades to a fresh launch that still carries the full
    /// launch context (not a bare binary). An unknown agent whose recorded name is
    /// a safe command token degrades to a bare-binary fresh launch (it cannot be
    /// decorated). Returns `None` when no safe command can be built (an unknown
    /// agent whose recorded name is not a safe command token, or a known agent the
    /// launch pipeline declines to launch in this configuration), so the caller
    /// can surface a per-pane error instead of executing it.
    ///
    /// The persisted id and unknown-agent name are validated because the command
    /// is ultimately run through a shell by `tmux respawn-pane`; an unvalidated
    /// value with shell metacharacters would otherwise be a command-injection
    /// vector. Only a `native_session_id` that passes `is_valid_resume_token` is
    /// ever interpolated into the resume flag.
    fn build_pane_resume_plan(
        &self,
        agent: &str,
        native_session_id: &str,
        is_primary: bool,
        mode: RestartMode,
        slot_identity_key: Option<&str>,
    ) -> Option<(String, bool)> {
        let Some(def) = crate::agents::get_agent(agent) else {
            // Unknown agent: only the recorded name can act as the binary, and
            // only if it is a safe command token; otherwise refuse to build a
            // command. Unknown agents cannot be decorated with launch context.
            return is_safe_command_token(agent).then(|| (agent.to_string(), false));
        };

        // `Fresh` forces the no-resume path: still build the full launch context
        // via `build_pane_command`, but never append a resume flag.
        let resumed = mode == RestartMode::Resume
            && def.resume.is_some()
            && is_valid_resume_token(native_session_id);
        let resume_token = resumed.then_some(native_session_id);
        let command =
            self.build_pane_command(def.name, resume_token, is_primary, slot_identity_key)?;
        Some((command, resumed))
    }

    /// Reusable per-pane resume-launch core (shared with cold-start recovery).
    ///
    /// Given a tracked pane's recorded agent, its persisted `native_session_id`,
    /// its `tmux_pane` target, its `cwd`, and whether it is the primary pane, kill
    /// the pane's process tree and respawn it with the command built through
    /// [`build_pane_resume_plan`](Self::build_pane_resume_plan) (full launch
    /// context plus, when a valid token is present, the resume flag). A pane with
    /// no usable resume id (empty/invalid id or an agent without a `ResumeConfig`)
    /// degrades to a fresh launch of that one pane that still carries the launch
    /// context. A pane whose agent name is unknown and not a safe command token,
    /// or whose tmux respawn fails, is returned as [`PaneResumeOutcome::Error`] so
    /// the caller can isolate per-pane failures.
    #[allow(clippy::too_many_arguments)]
    fn resume_launch_pane(
        &self,
        agent: &str,
        native_session_id: &str,
        tmux_pane: &str,
        cwd: &str,
        is_primary: bool,
        mode: RestartMode,
        slot_identity_key: Option<&str>,
    ) -> PaneResumeOutcome {
        // Build (and validate) the command before killing the pane, so a pane we
        // cannot safely respawn is left running rather than killed and abandoned.
        let Some((command, resumed)) = self.build_pane_resume_plan(
            agent,
            native_session_id,
            is_primary,
            mode,
            slot_identity_key,
        ) else {
            return PaneResumeOutcome::Error(format!("unsafe or unknown agent '{agent}'"));
        };

        // The process tree is killed outside tmux (an agent's children can
        // outlive the SIGHUP that `respawn-pane -k` alone would send them), and a
        // pane whose remain-on-exit is off is destroyed by tmux the moment that
        // kill lands -- taking the respawn target with it. Hold the pane open
        // across the kill; the respawn below then writes the setting this pane's
        // agent actually wants.
        //
        // If the pane cannot be held open, the external kill is skipped rather
        // than performed unprotected: `respawn-pane -k` below still replaces
        // what runs in the pane, so the slot comes back either way, and the cost
        // of skipping is orphaned grandchildren rather than a destroyed pane and
        // a lost slot.
        match tmux::set_pane_remain_on_exit(tmux_pane, true) {
            Ok(()) => tmux::kill_pane_process_tree_target(tmux_pane),
            Err(err) => tracing::warn!(
                "Could not hold pane {} open for relaunch, skipping its process-tree kill: {}",
                tmux_pane,
                err
            ),
        }

        if let Err(err) =
            tmux::respawn_pane_target(tmux_pane, &command, cwd, !pane_agent_is_shell(agent))
        {
            return PaneResumeOutcome::Error(err.to_string());
        }

        if resumed {
            PaneResumeOutcome::Resumed
        } else {
            PaneResumeOutcome::DegradedToFresh
        }
    }
}

/// Whether a tracked pane's recorded agent is a plain shell. Shell panes
/// should close when their process exits instead of getting the pane-died
/// fallback (which would respawn another shell).
fn pane_agent_is_shell(agent: &str) -> bool {
    match crate::agents::get_agent(agent) {
        Some(def) => def.name == "shell" || crate::tmux::utils::is_shell_command(def.binary),
        None => crate::tmux::utils::is_shell_command(agent),
    }
}

pub(crate) fn extract_resume_token(output: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(output)?
        .get(1)
        .map(|m| m.as_str().to_string())
}

pub(crate) fn is_valid_resume_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Pure recoverability predicate: an instance is recoverable when it has
/// persisted slots and its tmux session is not currently alive.
fn is_recoverable_from(has_slots: bool, session_alive: bool) -> bool {
    has_slots && !session_alive
}

/// Which of the rebuilt slots no longer have a pane in the session, as
/// `(index into `paired`, report)`. A slot whose pane could not be created in the
/// first place is skipped: it already carries the creation failure and reporting
/// it twice would say the same thing in two voices.
///
/// The report names the slot by the agent and working directory it recorded,
/// which is what the user recognizes; a bare pane id names something that no
/// longer exists and that the user never saw.
fn missing_slot_failures(
    paired: &[(crate::db::AgentSlot, Option<String>)],
    live_panes: &std::collections::HashSet<String>,
) -> Vec<(usize, String)> {
    paired
        .iter()
        .enumerate()
        .filter_map(|(index, (slot, pane))| {
            let pane = pane.as_ref()?;
            if live_panes.contains(pane) {
                return None;
            }
            Some((
                index,
                format!(
                    "slot {} ({} in {}) did not come back",
                    slot.slot, slot.agent, slot.cwd
                ),
            ))
        })
        .collect()
}

/// Recreate one pane per slot and pair each slot with its new pane id in slot
/// order. Slot 0 is the primary pane the start path already created (read back
/// from `@aoe_agent_pane`); slots 1..N are split as a chain from the pane
/// created immediately before them. The chain keeps tmux's pane-list order
/// aligned with durable slot order, which is how `select-layout` assigns panes
/// to layout leaves. A slot whose split fails (e.g. a recorded cwd that no
/// longer exists) is paired with `None` so its siblings still recover instead
/// of the whole rebuild aborting.
fn rebuild_recovery_panes(
    title: &str,
    session_name: &str,
    ordered: &[crate::db::AgentSlot],
) -> Result<Vec<(crate::db::AgentSlot, Option<String>)>> {
    // Slot 0 is the single pane the start path just created. Prefer the pinned
    // `@aoe_agent_pane`, but fall back to listing the session's only pane for
    // start paths that don't pin it (the list is unambiguous before any split).
    let primary_pane = tmux::get_agent_pane_id(session_name)
        .or_else(|| {
            crate::db::reconcile::session_pane_ids(session_name)
                .into_iter()
                .next()
        })
        .ok_or_else(|| anyhow::anyhow!("recovered session '{}' has no primary pane", title))?;

    let mut paired = Vec::with_capacity(ordered.len());
    paired.push((ordered[0].clone(), Some(primary_pane.clone())));
    let mut split_target = primary_pane;

    for slot in ordered.iter().skip(1) {
        // Placeholder pane runs the default shell until the resume flow
        // respawns it with the slot's agent command, so no remain-on-exit yet.
        match tmux::split_window_right_capture_pane(&split_target, &slot.cwd, "", false) {
            Ok(pane_id) => {
                split_target = pane_id.clone();
                paired.push((slot.clone(), Some(pane_id)));
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to create recovery pane for slot {} of '{}' (cwd {}): {}",
                    slot.slot,
                    title,
                    slot.cwd,
                    e
                );
                paired.push((slot.clone(), None));
            }
        }
    }
    Ok(paired)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_instance() {
        let inst = Instance::new("test", "/tmp/test");
        assert_eq!(inst.title, "test");
        assert_eq!(inst.project_path, "/tmp/test");
        assert_eq!(inst.status, Status::Idle);
        assert_eq!(inst.id.len(), 16);
        assert!(inst.resume_token.is_none());
        assert!(inst.last_spinner_seen.is_none());
        assert!(inst.spike_start.is_none());
        assert!(inst.pre_spike_status.is_none());
        assert!(!inst.acknowledged);
    }

    #[test]
    fn test_is_sub_session() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert!(!inst.is_sub_session());

        inst.parent_session_id = Some("parent123".to_string());
        assert!(inst.is_sub_session());
    }

    #[test]
    fn test_all_agents_have_yolo_support() {
        for agent in crate::agents::AGENTS {
            if agent.name == "shell" {
                assert!(
                    agent.yolo.is_none(),
                    "Shell should not have YOLO mode configured"
                );
                continue;
            }
            assert!(
                agent.yolo.is_some(),
                "Agent '{}' should have YOLO mode configured",
                agent.name
            );
        }
    }

    #[test]
    fn test_yolo_mode_helper() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert!(!inst.is_yolo_mode());

        inst.yolo_mode = true;
        assert!(inst.is_yolo_mode());

        inst.yolo_mode = false;
        assert!(!inst.is_yolo_mode());
    }

    #[test]
    fn test_yolo_mode_without_sandbox() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert!(!inst.is_sandboxed());

        inst.yolo_mode = true;
        assert!(inst.is_yolo_mode());
        assert!(!inst.is_sandboxed());
    }

    #[test]
    fn test_yolo_envvar_command_is_quoted() {
        // EnvVar values containing JSON must be shell-escaped to prevent
        // the inner bash from expanding special characters ({, *, ").
        let result = format_env_var_prefix("OPENCODE_PERMISSION", r#"{"*":"allow"}"#, "opencode");
        assert_eq!(result, r#"OPENCODE_PERMISSION='{"*":"allow"}' opencode"#);
    }

    #[test]
    fn test_yolo_envvar_survives_suspend_wrapper() {
        // The full chain: format_env_var_prefix -> wrap_command_ignore_suspend
        // must preserve the JSON value through both quoting layers.
        // Single quotes from shell_escape are escaped by wrap_command_ignore_suspend
        // via the '\'' technique, which correctly round-trips through the shell.
        let cmd = format_env_var_prefix("OPENCODE_PERMISSION", r#"{"*":"allow"}"#, "opencode");
        let wrapped = wrap_command_ignore_suspend(&cmd);
        // The inner single quotes from shell_escape become '\'' in the outer wrapper
        assert!(
            wrapped.contains(r#"OPENCODE_PERMISSION='\''{"*":"allow"}'\'' opencode"#),
            "wrapped command should contain the escaped env var assignment: {}",
            wrapped,
        );
    }

    // Additional tests for is_sandboxed
    #[test]
    fn test_is_sandboxed_without_sandbox_info() {
        let inst = Instance::new("test", "/tmp/test");
        assert!(!inst.is_sandboxed());
    }

    #[test]
    fn test_is_sandboxed_with_disabled_sandbox() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.sandbox_info = Some(SandboxInfo {
            enabled: false,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        assert!(!inst.is_sandboxed());
    }

    #[test]
    fn test_is_sandboxed_with_enabled_sandbox() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        assert!(inst.is_sandboxed());
    }

    // Tests for get_tool_command
    #[test]
    fn test_get_tool_command_default_claude() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        assert_eq!(inst.get_tool_command(), "claude");
    }

    #[test]
    fn test_get_tool_command_opencode() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "opencode".to_string();
        assert_eq!(inst.get_tool_command(), "opencode");
    }

    #[test]
    fn test_get_tool_command_codex() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        assert_eq!(inst.get_tool_command(), "codex");
    }

    #[test]
    fn test_get_tool_command_gemini() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "gemini".to_string();
        assert_eq!(inst.get_tool_command(), "gemini");
    }

    #[test]
    fn test_get_tool_command_unknown_tool() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "unknown".to_string();
        assert_eq!(inst.get_tool_command(), "bash");
    }

    #[test]
    fn test_get_tool_command_custom_command() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.command = "claude --resume abc123".to_string();
        assert_eq!(inst.get_tool_command(), "claude --resume abc123");
    }

    #[test]
    fn test_wrap_command_ignore_suspend_basic() {
        let shell = crate::session::environment::user_posix_shell();
        assert_eq!(
            wrap_command_ignore_suspend("opencode"),
            format!("{shell} -lc 'stty susp undef; exec env opencode'")
        );
    }

    #[test]
    fn test_wrap_command_ignore_suspend_with_env() {
        let shell = crate::session::environment::user_posix_shell();
        let result = wrap_command_ignore_suspend_with_env(
            "opencode",
            &[("OPENCODE_PERMISSION", r#"{"*":"allow"}"#)],
        );
        // Env vars are placed before the shell, not inside the single-quoted string
        assert_eq!(
            result,
            format!(
                r#"OPENCODE_PERMISSION='{{"*":"allow"}}' {shell} -lc 'stty susp undef; exec env opencode'"#
            )
        );
    }

    #[test]
    fn test_wrap_command_ignore_suspend_with_env_no_vars() {
        assert_eq!(
            wrap_command_ignore_suspend_with_env("claude", &[]),
            wrap_command_ignore_suspend("claude"),
        );
    }

    #[test]
    fn test_build_agent_command_inserts_claude_resume_flag_after_binary() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.extra_args = "--model sonnet".to_string();
        inst.yolo_mode = true;

        let cmd = inst
            .build_agent_command(Some("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"))
            .unwrap();

        assert!(
            cmd.starts_with("AOE_INSTANCE_ID='"),
            "expected hook env prefix, got {cmd}"
        );
        let shell = crate::session::environment::user_posix_shell();
        assert!(
            cmd.contains(
                &format!("{shell} -lc 'stty susp undef; exec env claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11 --model sonnet --dangerously-skip-permissions'")
            ),
            "unexpected claude resume command: {cmd}"
        );
    }

    #[test]
    fn test_build_agent_command_inserts_codex_resume_flag_after_binary() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.extra_args = "--model gpt-5".to_string();
        inst.yolo_mode = true;

        let cmd = inst
            .build_agent_command(Some("019d1af9-a899-7df1-8f7d-a244126e5ded"))
            .unwrap();

        // Codex now carries `AOE_INSTANCE_ID` because it has a hook
        // configuration: the status-file half of the hook is gated on that
        // variable, so a Codex launch without it would capture panes and never
        // report status. The resume flag's position is what this test is named
        // for, so it is asserted where it sits rather than at the end: hook
        // config overrides and the YOLO flag follow it.
        assert!(
            cmd.contains(
                "exec env codex resume 019d1af9-a899-7df1-8f7d-a244126e5ded --model gpt-5 "
            ),
            "unexpected codex resume command: {cmd}"
        );
        assert!(
            cmd.ends_with("--dangerously-bypass-approvals-and-sandbox'"),
            "unexpected codex resume command: {cmd}"
        );
        assert!(
            cmd.starts_with("AOE_INSTANCE_ID="),
            "a hook-config agent's launch must carry AOE_INSTANCE_ID: {cmd}"
        );
    }

    /// Codex's hooks run in a shared app-server that inherits its environment
    /// once, at daemon start, so a hook's `$TMUX_PANE` names whatever pane that
    /// daemon happened to start in and `$AOE_INSTANCE_ID` is absent entirely.
    /// Both values must therefore travel as per-session config overrides.
    #[test]
    fn test_codex_launch_carries_pane_identity_as_config_overrides() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();

        let cmd = inst.build_agent_command(None).unwrap();

        assert!(
            cmd.contains("-c \"shell_environment_policy.set.AOE_TMUX_PANE=\\\"$TMUX_PANE\\\"\""),
            "codex must forward its own pane to its hooks: {cmd}"
        );
        assert!(
            cmd.contains(&format!(
                "-c \"shell_environment_policy.set.AOE_INSTANCE_ID=\\\"{}\\\"\"",
                inst.id
            )),
            "codex must forward its instance id to its hooks: {cmd}"
        );
    }

    /// An agent whose hooks run in its own process already sees the right
    /// `$TMUX_PANE`, so it must not be given the Codex workaround.
    #[test]
    fn test_claude_launch_has_no_codex_config_overrides() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();

        let cmd = inst.build_agent_command(None).unwrap();

        assert!(
            !cmd.contains("shell_environment_policy"),
            "only codex needs the app-server workaround: {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_flag_appended_when_enabled() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            cmd.contains(
                "--dangerously-load-development-channels server:cross-agent-teams-channel"
            ),
            "expected dev-channels flag, got {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_flag_absent_when_disabled() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = false;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !cmd.contains("--dangerously-load-development-channels"),
            "did not expect dev-channels flag, got {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_coexists_with_yolo() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = true;
        inst.cross_agent_team = true;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            cmd.contains("--dangerously-skip-permissions"),
            "expected yolo flag, got {cmd}"
        );
        assert!(
            cmd.contains(
                "--dangerously-load-development-channels server:cross-agent-teams-channel"
            ),
            "expected dev-channels flag, got {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_custom_channel() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;
        inst.cross_agent_team_channel = "server:my-channel".to_string();

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            cmd.contains("--dangerously-load-development-channels server:my-channel"),
            "expected custom channel, got {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_no_token_injection() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !cmd.contains("CROSS_AGENT_TEAMS_MCP_TOKEN"),
            "token must be inherited from environment, not injected: {cmd}"
        );
    }

    #[test]
    fn test_is_auto_confirm_screen_dev_channels() {
        let screen = "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development\n    2. Exit";
        assert_eq!(
            auto_confirm_step(screen, &[]),
            AutoConfirmStep::Answer(AutoConfirmPrompt::DevelopmentChannels)
        );
    }

    #[test]
    fn test_is_auto_confirm_screen_trust_folder() {
        let screen = " Quick safety check: Is this a project you created or one you trust?\n ❯ 1. Yes, I trust this folder";
        assert_eq!(
            auto_confirm_step(screen, &[]),
            AutoConfirmStep::Answer(AutoConfirmPrompt::WorkspaceTrust)
        );
    }

    #[test]
    fn test_is_auto_confirm_screen_negative() {
        let screen = "Welcome to Claude Code\n> how can I help?";
        assert_eq!(auto_confirm_step(screen, &[]), AutoConfirmStep::NoPrompt);
    }

    #[test]
    fn test_is_auto_confirm_screen_with_ansi_per_word_coloring() {
        // Claude colors the warning title per word; `tmux capture-pane -e`
        // interleaves SGR codes, splitting the phrase. Stripping must restore it.
        let screen = "\u{1b}[39m  \u{1b}[1m\u{1b}[38;5;211mWARNING:\u{1b}[0m \u{1b}[1m\u{1b}[38;5;211mLoading\u{1b}[0m \u{1b}[1m\u{1b}[38;5;211mdevelopment\u{1b}[0m \u{1b}[1m\u{1b}[38;5;211mchannels\u{1b}[0m";
        assert!(
            !screen.contains("Loading development channels"),
            "raw -e capture should not contain the contiguous phrase"
        );
        assert_eq!(
            auto_confirm_step(screen, &[]),
            AutoConfirmStep::Answer(AutoConfirmPrompt::DevelopmentChannels),
            "after stripping ANSI the phrase must match"
        );
    }

    /// The settle is waited out synchronously before a recovered session is
    /// handed back, so a mistyped override must not be able to turn recovery
    /// into a hang.
    #[test]
    #[serial_test::serial]
    fn test_recovery_settle_override_is_capped() {
        let restore = std::env::var(RECOVERY_SETTLE_ENV).ok();

        std::env::remove_var(RECOVERY_SETTLE_ENV);
        assert_eq!(recovery_settle(), RECOVERY_SETTLE, "no override, no change");

        std::env::set_var(RECOVERY_SETTLE_ENV, "1200");
        assert_eq!(
            recovery_settle(),
            Duration::from_millis(1200),
            "a value inside the ceiling is honored as written"
        );

        std::env::set_var(RECOVERY_SETTLE_ENV, "600000");
        assert_eq!(
            recovery_settle(),
            RECOVERY_SETTLE_MAX,
            "an over-large value is clamped, not obeyed"
        );

        std::env::set_var(RECOVERY_SETTLE_ENV, "not-a-number");
        assert_eq!(
            recovery_settle(),
            RECOVERY_SETTLE,
            "an unparseable value falls back to the default"
        );

        match restore {
            Some(v) => std::env::set_var(RECOVERY_SETTLE_ENV, v),
            None => std::env::remove_var(RECOVERY_SETTLE_ENV),
        }
    }

    /// The screen a question is drawn on changes while the question is still up
    /// -- a spinner tick, a status line, a partial redraw. Answering must key on
    /// the question, so a redraw is not a second question.
    #[test]
    fn test_same_prompt_redrawn_is_answered_once() {
        let redraws = [
            "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development\n  ⠋ 0s",
            "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development\n  ⠙ 0s",
            "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development\n  ⠹ 1s",
            "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development\n  ⠸ 1s",
        ];

        let mut answered: Vec<AutoConfirmPrompt> = Vec::new();
        let mut sends = 0;
        for screen in redraws {
            if let AutoConfirmStep::Answer(prompt) = auto_confirm_step(screen, &answered) {
                answered.push(prompt);
                sends += 1;
            }
        }

        assert_eq!(sends, 1, "four redraws of one question are one question");
        assert_eq!(answered, vec![AutoConfirmPrompt::DevelopmentChannels]);
    }

    /// A second, different question is a second question even though the first
    /// was already answered -- including when it arrives after a quiet gap.
    #[test]
    fn test_second_distinct_prompt_is_answered_after_the_first() {
        let dev =
            "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development";
        let quiet = "  Welcome to Claude Code\n  ⠋ starting";
        let trust = " Quick safety check: Is this a project you created or one you trust?\n ❯ 1. Yes, I trust this folder";

        let mut answered: Vec<AutoConfirmPrompt> = Vec::new();
        let mut sends = 0;
        for screen in [dev, dev, quiet, quiet, trust, trust] {
            if let AutoConfirmStep::Answer(prompt) = auto_confirm_step(screen, &answered) {
                answered.push(prompt);
                sends += 1;
            }
        }

        assert_eq!(sends, 2, "two distinct questions, each answered once");
        assert_eq!(
            answered,
            vec![
                AutoConfirmPrompt::DevelopmentChannels,
                AutoConfirmPrompt::WorkspaceTrust
            ]
        );
    }

    /// A question already answered reports itself as such rather than as absent,
    /// so the caller can tell "waiting for this to be processed" from "nothing
    /// is being asked".
    #[test]
    fn test_answered_prompt_still_on_screen_is_not_no_prompt() {
        let dev =
            "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development";
        assert_eq!(
            auto_confirm_step(dev, &[AutoConfirmPrompt::DevelopmentChannels]),
            AutoConfirmStep::AlreadyAnswered
        );
    }

    #[test]
    fn test_strip_ansi_basic() {
        assert_eq!(strip_ansi("\u{1b}[1mhi\u{1b}[0m there"), "hi there");
        assert_eq!(strip_ansi("plain text"), "plain text");
    }

    #[test]
    fn test_run_auto_confirm_noop_for_non_cross_agent_team() {
        // Must not panic or spawn work when the mode is off.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = false;
        inst.run_auto_confirm("claude");

        // Also a no-op for non-claude even if the flag is set.
        inst.tool = "codex".to_string();
        inst.cross_agent_team = true;
        inst.run_auto_confirm("codex");
    }

    /// A command override says the pane is meant to run something other than
    /// the tool's binary, and that override is what a restart must relaunch.
    /// Reading the agent out of the pane there would discard it.
    #[test]
    fn test_command_override_is_never_second_guessed_from_the_pane() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.command = "codex --some-flag".to_string();
        assert_eq!(
            inst.pane_agent_overriding_instance_tool(),
            None,
            "an instance with a command override restarts that command, whatever \
             the pane's process happens to be named"
        );
    }

    #[test]
    fn test_codex_cross_agent_team_does_not_use_claude_flag() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.cross_agent_team = true;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !cmd.contains("--dangerously-load-development-channels"),
            "dev-channels flag should be claude-only, got {cmd}"
        );
        assert!(
            cmd.contains("pre-register-codex-pane"),
            "expected Codex xats bootstrap, got {cmd}"
        );
    }

    fn codex_xats_instance() -> Instance {
        let mut inst = Instance::new("test", "/tmp/project path");
        inst.tool = "codex".to_string();
        inst.command = "codex".to_string();
        inst.cross_agent_team = true;
        inst
    }

    fn claude_xats_instance() -> Instance {
        let mut inst = Instance::new("test", "/tmp/project path");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;
        inst
    }

    #[test]
    fn test_new_instance_starts_without_an_identity_key() {
        // New-from-selection builds a fresh instance through the builder rather
        // than cloning the source, so a copied key cannot reach it. This pins the
        // property the builder relies on.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;
        assert!(inst.xats_identity_key.is_none());

        inst.ensure_xats_identity_key();
        let fresh = Instance::new("other", "/tmp/test");
        assert!(
            fresh.xats_identity_key.is_none(),
            "a newly built session must not carry another session's identity"
        );
    }

    fn slot(index: i64, key: &str) -> crate::db::AgentSlot {
        crate::db::AgentSlot {
            instance_id: "inst".to_string(),
            slot: index,
            agent: "claude".to_string(),
            native_session_id: "sess".to_string(),
            cwd: "/tmp".to_string(),
            tmux_pane: "%1".to_string(),
            xats_identity_key: key.to_string(),
            last_seen_at: 1,
        }
    }

    #[test]
    fn test_adopted_slot_without_key_needs_one_minted() {
        // A hand-started pane is adopted with no key, because AoE never built its
        // command. The first launch AoE performs for that slot mints one.
        let inst = claude_xats_instance();
        assert!(inst.slot_needs_identity_key(&slot(1, "")));
    }

    #[test]
    fn test_slot_with_key_is_left_alone() {
        let inst = claude_xats_instance();
        assert!(!inst.slot_needs_identity_key(&slot(1, "existing-key")));
    }

    #[test]
    fn test_primary_slot_never_mints_into_the_slot_record() {
        // Slot 0's key lives on the instance record; minting into the slot row too
        // would give the primary pane two homes for one value.
        let inst = claude_xats_instance();
        assert!(!inst.slot_needs_identity_key(&slot(0, "")));
    }

    #[test]
    fn test_no_slot_needs_a_key_without_cross_agent_team() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        assert!(!inst.slot_needs_identity_key(&slot(1, "")));
    }

    #[test]
    fn test_identity_key_minted_once_and_reused() {
        let mut inst = claude_xats_instance();
        assert!(inst.xats_identity_key.is_none());

        inst.ensure_xats_identity_key();
        let first = inst.xats_identity_key.clone().unwrap();
        assert!(!first.is_empty());

        inst.ensure_xats_identity_key();
        assert_eq!(
            inst.xats_identity_key.as_deref(),
            Some(first.as_str()),
            "the key is write-once: a later launch must reuse it"
        );
    }

    #[test]
    fn test_identity_key_not_minted_without_cross_agent_team() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.ensure_xats_identity_key();
        assert!(inst.xats_identity_key.is_none());
    }

    #[test]
    fn test_identity_key_injected_as_env_not_argv() {
        // CRITICAL: argv is world-readable through the process table, the
        // environment is not. The key must never reach the command arguments.
        let mut inst = claude_xats_instance();
        inst.ensure_xats_identity_key();
        let key = inst.xats_identity_key.clone().unwrap();

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            cmd.contains(&format!("XATS_IDENTITY_KEY='{key}'")),
            "identity key must be injected as an environment variable, got: {cmd}"
        );
        let argv = cmd
            .split_once(&format!("XATS_IDENTITY_KEY='{key}'"))
            .map(|(_, rest)| rest)
            .unwrap();
        assert!(
            !argv.contains(&key),
            "identity key must not also appear in the command arguments, got: {cmd}"
        );
    }

    #[test]
    fn test_no_identity_key_env_when_cross_agent_team_disabled() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.xats_identity_key = Some("leftover-key".to_string());

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !cmd.contains("XATS_IDENTITY_KEY"),
            "a disabled session must not inject the variable, got: {cmd}"
        );
    }

    #[test]
    fn test_secondary_pane_uses_slot_key_not_instance_key() {
        let mut inst = claude_xats_instance();
        inst.xats_identity_key = Some("primary-key".to_string());

        let (cmd, _) = inst
            .build_pane_resume_plan("claude", "", false, RestartMode::Fresh, Some("slot-key"))
            .unwrap();
        assert!(cmd.contains("XATS_IDENTITY_KEY='slot-key'"), "got: {cmd}");
        assert!(
            !cmd.contains("primary-key"),
            "an adopted pane must not inherit the primary pane's identity, got: {cmd}"
        );
    }

    #[test]
    fn test_adopted_pane_without_key_injects_nothing() {
        // A hand-started pane AoE never launched has no key until AoE relaunches
        // its slot; until then there is nothing to inject.
        let inst = claude_xats_instance();
        let (cmd, _) = inst
            .build_pane_resume_plan("claude", "", false, RestartMode::Fresh, Some(""))
            .unwrap();
        assert!(!cmd.contains("XATS_IDENTITY_KEY"), "got: {cmd}");
    }

    #[test]
    fn test_codex_identity_key_is_distinct_from_pane_nonce() {
        // The bootstrap nonce is one-shot evidence; the identity key is a durable
        // alias. A codex pane carries both and they must not be the same value.
        let mut inst = codex_xats_instance();
        inst.ensure_xats_identity_key();
        let key = inst.xats_identity_key.clone().unwrap();

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            cmd.contains(&format!("XATS_IDENTITY_KEY='{key}'")),
            "got: {cmd}"
        );
        assert!(
            cmd.contains("xats_agent_id=\"$(uuidgen)\""),
            "the one-shot pane nonce must still be generated per launch, got: {cmd}"
        );
    }

    #[test]
    fn test_fork_does_not_inherit_identity_key() {
        // CRITICAL: this is the only point at which two panes claiming one xats
        // identity can be prevented; the daemon cannot tell a copied key apart
        // from the original pane restarting.
        let mut inst = claude_xats_instance();
        inst.ensure_xats_identity_key();
        inst.resume_token = Some("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11".to_string());

        let fork = inst.create_fork("forked".to_string(), None).unwrap();
        assert!(fork.xats_identity_key.is_none());

        let mut fork = fork;
        fork.ensure_xats_identity_key();
        assert_ne!(
            fork.xats_identity_key, inst.xats_identity_key,
            "a fork must mint its own identity key"
        );
    }

    #[test]
    fn test_cross_agent_team_supported_tool_helpers() {
        assert!(Instance::supports_cross_agent_team_tool("claude"));
        assert!(Instance::supports_cross_agent_team_tool("codex"));
        assert!(!Instance::supports_cross_agent_team_tool("opencode"));

        let mut inst = codex_xats_instance();
        assert!(inst.is_cross_agent_team());
        // A Codex instance takes Codex's integration, and takes Claude's for a
        // Claude pane adopted into it -- the instance's tool decides neither.
        assert!(inst.cross_agent_team_pane("codex"));
        assert!(inst.cross_agent_team_pane("claude"));
        assert!(!inst.cross_agent_team_pane("gemini"));

        inst.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        assert!(!inst.is_cross_agent_team());
    }

    #[test]
    fn test_codex_xats_fresh_command_is_non_yolo_by_default() {
        let cmd = codex_xats_instance().build_agent_command(None).unwrap();

        assert!(cmd.contains("pre-register-codex-pane"));
        assert!(cmd.contains("--remote"));
        assert!(cmd.contains(CODEX_XATS_APP_SERVER_URL));
        assert!(cmd.contains("xats.agent_id="));
        assert!(cmd.contains("/tmp/project path"));
        assert!(!cmd.contains("--dangerously-bypass-approvals-and-sandbox"));
        assert!(!cmd.contains("CROSS_AGENT_TEAMS_MCP_TOKEN"));
    }

    #[test]
    fn test_codex_xats_yolo_command_preserves_yolo_flag() {
        let mut inst = codex_xats_instance();
        inst.yolo_mode = true;

        let cmd = inst.build_agent_command(None).unwrap();

        assert!(cmd.contains("pre-register-codex-pane"));
        assert!(cmd.contains("--dangerously-bypass-approvals-and-sandbox"));
    }

    #[test]
    fn test_codex_xats_resume_preserves_native_token() {
        let token = "019d1af9-a899-7df1-8f7d-a244126e5ded";
        let cmd = codex_xats_instance()
            .build_agent_command(Some(token))
            .unwrap();

        assert!(cmd.contains(&format!("resume {token}")));
        assert!(cmd.find("--remote").unwrap() < cmd.find(&format!("resume {token}")).unwrap());
    }

    #[test]
    fn test_codex_xats_restart_plans_reapply_bootstrap() {
        let token = "019d1af9-a899-7df1-8f7d-a244126e5ded";
        let inst = codex_xats_instance();

        let (resume_cmd, resumed) = inst
            .build_pane_resume_plan("codex", token, true, RestartMode::Resume, None)
            .expect("Codex resume plan");
        assert!(resumed);
        assert!(resume_cmd.contains("pre-register-codex-pane"));
        assert!(resume_cmd.contains(&format!("resume {token}")));

        let (fresh_cmd, resumed) = inst
            .build_pane_resume_plan("codex", token, true, RestartMode::Fresh, None)
            .expect("Codex fresh plan");
        assert!(!resumed);
        assert!(fresh_cmd.contains("pre-register-codex-pane"));
        assert!(!fresh_cmd.contains(&format!("resume {token}")));
    }

    #[test]
    fn test_codex_xats_fork_preserves_parent_token() {
        let token = "019d1af9-a899-7df1-8f7d-a244126e5ded";
        let mut parent = codex_xats_instance();
        parent.resume_token = Some(token.to_string());
        let fork = parent
            .create_fork("fork".to_string(), None)
            .expect("Codex fork should build");

        let cmd = fork.build_agent_command(None).unwrap();

        assert!(cmd.contains(&format!("fork {token}")));
        assert!(cmd.contains("pre-register-codex-pane"));
        assert!(cmd.find("--remote").unwrap() < cmd.find(&format!("fork {token}")).unwrap());
    }

    #[test]
    fn test_codex_xats_bootstrap_has_explicit_failure_diagnostics() {
        let cmd = codex_xats_instance().build_agent_command(None).unwrap();

        for diagnostic in [
            CODEX_XATS_MISSING_PANE,
            CODEX_XATS_MISSING_UUIDGEN,
            CODEX_XATS_MISSING_NC,
            CODEX_XATS_MISSING_NPX,
            CODEX_XATS_INVALID_UUID,
            CODEX_XATS_APP_SERVER_UNAVAILABLE,
            "[xats] Failed to pre-register the Codex pane.",
        ] {
            assert!(
                cmd.contains(diagnostic),
                "missing diagnostic {diagnostic:?}: {cmd}"
            );
        }
        assert!(cmd.contains("exit 1"));
    }

    #[test]
    fn test_codex_cross_agent_team_disabled_uses_normal_command() {
        let mut inst = codex_xats_instance();
        inst.cross_agent_team = false;

        let cmd = inst.build_agent_command(None).unwrap();

        assert!(!cmd.contains("pre-register-codex-pane"));
        assert!(!cmd.contains("--remote"));
        assert!(!cmd.contains("xats.agent_id"));
    }

    #[test]
    fn test_build_agent_command_shell_starts_in_project_path() {
        let shell = crate::session::environment::user_posix_shell();
        let mut inst = Instance::new("test", "/tmp/expected path");
        inst.tool = "shell".to_string();
        inst.command = shell.clone();

        let cmd = inst.build_agent_command(None).unwrap();
        let escaped_dir = shell_escape("/tmp/expected path");
        let expected_inner =
            format!("cd {escaped_dir} && stty susp undef; exec {shell}").replace('\'', "'\\''");
        let expected = format!("{shell} -lc '{expected_inner}'");

        assert_eq!(cmd, expected);
    }

    #[test]
    fn test_resume_config_patterns_match_expected_agent_output() {
        let claude_resume = crate::agents::get_agent("claude")
            .and_then(|agent| agent.resume.as_ref())
            .unwrap();
        let codex_resume = crate::agents::get_agent("codex")
            .and_then(|agent| agent.resume.as_ref())
            .unwrap();

        assert_eq!(
            extract_resume_token(
                "Run claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11 to continue.",
                claude_resume.resume_pattern,
            )
            .as_deref(),
            Some("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11")
        );
        assert_eq!(
            extract_resume_token(
                "Resume with: codex resume 019d1af9-a899-7df1-8f7d-a244126e5ded",
                codex_resume.resume_pattern,
            )
            .as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_build_pane_resume_plan_claude_appends_resume_flag() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan(
                "claude",
                "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(resumed);
        assert!(
            cmd.contains("claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "expected resume flag, got: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_fresh_mode_never_resumes() {
        // Fresh mode must force the no-resume path for every pane even with a
        // valid token: full launch context, but no resume flag.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan(
                "claude",
                "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
                true,
                RestartMode::Fresh,
                None,
            )
            .unwrap();
        assert!(!resumed, "Fresh mode must not resume");
        assert!(
            !cmd.contains("--resume"),
            "Fresh mode must carry no resume flag, got: {cmd}"
        );
        assert!(
            cmd.contains("claude"),
            "Fresh mode must still carry the launch command, got: {cmd}"
        );
    }

    #[test]
    fn test_fresh_single_pane_ignores_stored_resume_token() {
        // Decision 2: the fresh single-pane path must NOT consult the instance's
        // stored resume_token. The Resume path would inject it (via
        // resolved_resume_token); the Fresh path builds the command with None.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.resume_token = Some("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11".to_string());

        // Sanity: the Resume path would reinject the stored token.
        let resume_effective = inst.resolved_resume_token(None);
        let resume_cmd = inst
            .build_agent_command(resume_effective.as_deref())
            .unwrap();
        assert!(
            resume_cmd.contains("--resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "resume path should inject stored token, got: {resume_cmd}"
        );

        // Fresh path: build with None (bypassing resolved_resume_token) -> no
        // resume flag or token, even though resume_token is set.
        let fresh_cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !fresh_cmd.contains("--resume"),
            "fresh single-pane restart must carry no resume flag, got: {fresh_cmd}"
        );
        assert!(
            !fresh_cmd.contains("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "fresh single-pane restart must not inject stored token, got: {fresh_cmd}"
        );
    }

    #[test]
    fn test_fresh_restart_reallocates_session_id() {
        // CRITICAL: a fresh restart must not reuse the pre-allocated --session-id.
        // The just-killed conversation still owns it and Claude refuses to start
        // with a session id that is already in use.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let old_id = "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11".to_string();
        inst.agent_session_id = Some(old_id.clone());

        // Sanity: before reallocation the fresh command carries the OLD id.
        let before = inst.build_agent_command(None).unwrap();
        assert!(
            before.contains(&format!("--session-id {old_id}")),
            "sanity: fresh build reuses old session id before reallocation, got: {before}"
        );

        inst.reallocate_session_id_for_fresh();

        let new_id = inst.agent_session_id.clone().unwrap();
        assert_ne!(
            new_id, old_id,
            "fresh restart must allocate a new session id"
        );
        let after = inst.build_agent_command(None).unwrap();
        assert!(
            after.contains(&format!("--session-id {new_id}")),
            "fresh build must carry the new session id, got: {after}"
        );
        assert!(
            !after.contains(&old_id),
            "fresh build must not carry the old session id, got: {after}"
        );
    }

    #[test]
    fn test_reallocate_session_id_noop_without_session_id_flag() {
        // codex has no session_id_flag -> reallocation is a no-op.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.agent_session_id = Some("keep-me".to_string());
        inst.reallocate_session_id_for_fresh();
        assert_eq!(inst.agent_session_id.as_deref(), Some("keep-me"));
    }

    #[test]
    fn test_fresh_identity_rollback_on_failure_restores_id_and_fork() {
        // CRITICAL: a failed fresh respawn must not persist the never-launched new
        // session id (or a dropped fork); the snapshot is restored.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let old_id = "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11".to_string();
        inst.agent_session_id = Some(old_id.clone());
        inst.fork_pending = Some("parent-token".to_string());

        let snapshot = inst.begin_fresh_identity(RestartMode::Fresh);
        assert!(snapshot.is_some());
        // Speculative mutation happened: new id, fork dropped.
        assert_ne!(inst.agent_session_id.as_deref(), Some(old_id.as_str()));
        assert!(inst.fork_pending.is_none());

        inst.rollback_fresh_identity_on_failure(snapshot, false);
        assert_eq!(inst.agent_session_id.as_deref(), Some(old_id.as_str()));
        assert_eq!(inst.fork_pending.as_deref(), Some("parent-token"));
    }

    #[test]
    fn test_fresh_identity_commit_on_success_keeps_new_id() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let old_id = "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11".to_string();
        inst.agent_session_id = Some(old_id.clone());

        let snapshot = inst.begin_fresh_identity(RestartMode::Fresh);
        let new_id = inst.agent_session_id.clone();
        inst.rollback_fresh_identity_on_failure(snapshot, true);
        assert_eq!(inst.agent_session_id, new_id);
        assert_ne!(inst.agent_session_id.as_deref(), Some(old_id.as_str()));
    }

    #[test]
    fn test_fresh_identity_ignores_pending_fork() {
        // CRITICAL: a fresh restart must not re-fork a persisted parent. After
        // begin_fresh_identity the command carries no fork flag.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.fork_pending = Some("parent-token".to_string());

        // Sanity: with fork_pending set, the command would fork.
        assert!(inst
            .build_agent_command(None)
            .unwrap()
            .contains("--fork-session"));

        inst.begin_fresh_identity(RestartMode::Fresh);
        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !cmd.contains("--fork-session"),
            "fresh restart must not re-fork, got: {cmd}"
        );
        assert!(!cmd.contains("parent-token"), "got: {cmd}");
    }

    #[test]
    fn test_codex_fresh_clears_resume_token_so_fork_does_not_reuse_it() {
        // CRITICAL: codex has no agent_session_id; fork_token() falls back to
        // resume_token. A fresh restart clears it so a later fork does not reuse the
        // pre-fresh conversation.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        // Before: fork would reuse the stale resume token.
        assert_eq!(
            inst.fork_token().ok().as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );

        // A fresh restart clears it (the commit step in the respawn paths).
        inst.clear_resume_token();
        assert_ne!(
            inst.fork_token().ok().as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded"),
            "fork must not reuse the pre-fresh resume token"
        );
    }

    #[test]
    fn test_clean_recovery_commit_clears_token_so_fork_does_not_reuse_it() {
        // CRITICAL: clean recovery discards the conversation, so the identity
        // transaction it runs must leave nothing a later fork could resume from.
        // Mirrors the commit step in `recover_from_slots` (primary slot launched).
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        let identity = inst.begin_fresh_identity(RestartMode::Fresh);
        let primary_launched = true;
        if identity.is_some() && primary_launched {
            inst.clear_resume_token();
        }
        inst.rollback_fresh_identity_on_failure(identity, primary_launched);

        assert!(
            inst.fork_token().is_err(),
            "fork must not resume the conversation clean recovery discarded"
        );
    }

    #[test]
    fn test_clean_recovery_rollback_restores_identity_when_primary_fails() {
        // The primary slot never launched, so the speculative identity must not be
        // persisted: a later launch has to keep using the previous session id.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let old_id = "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11".to_string();
        inst.agent_session_id = Some(old_id.clone());

        let identity = inst.begin_fresh_identity(RestartMode::Fresh);
        let primary_launched = false;
        if identity.is_some() && primary_launched {
            inst.clear_resume_token();
        }
        inst.rollback_fresh_identity_on_failure(identity, primary_launched);

        assert_eq!(inst.agent_session_id.as_deref(), Some(old_id.as_str()));
    }

    #[test]
    fn test_resume_recovery_leaves_identity_untouched() {
        // Resume recovery must behave exactly as before: no reallocation, no
        // dropped fork, no cleared token.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let old_id = "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11".to_string();
        inst.agent_session_id = Some(old_id.clone());
        inst.fork_pending = Some("parent-token".to_string());

        let identity = inst.begin_fresh_identity(RestartMode::Resume);

        assert!(identity.is_none());
        assert_eq!(inst.agent_session_id.as_deref(), Some(old_id.as_str()));
        assert_eq!(inst.fork_pending.as_deref(), Some("parent-token"));
    }

    #[test]
    fn test_build_pane_resume_plan_codex_uses_resume_subcommand() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan(
                "codex",
                "019d1af9-a899-7df1-8f7d-a244126e5ded",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(resumed);
        assert!(
            cmd.contains("codex resume 019d1af9-a899-7df1-8f7d-a244126e5ded"),
            "expected resume subcommand, got: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_empty_id_restarts_fresh() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "", true, RestartMode::Resume, None)
            .unwrap();
        assert!(!resumed);
        assert!(
            !cmd.contains("--resume"),
            "expected no resume flag, got: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_invalid_id_restarts_fresh() {
        // A persisted id with shell metacharacters must never be substituted
        // into the command; it degrades to a fresh restart instead.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "abc; rm -rf ~", true, RestartMode::Resume, None)
            .unwrap();
        assert!(!resumed);
        assert!(
            !cmd.contains("--resume"),
            "expected no resume flag, got: {cmd}"
        );
        assert!(
            !cmd.contains("rm -rf"),
            "unsafe id must not be interpolated: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_agent_without_resume_config_restarts_fresh() {
        // gemini has no ResumeConfig -> fresh launch even with a persisted id.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "gemini".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan("gemini", "gemini-sess-0", true, RestartMode::Resume, None)
            .unwrap();
        assert!(!resumed);
        assert!(
            !cmd.contains("resume"),
            "expected no resume flag, got: {cmd}"
        );
        assert!(
            cmd.contains(crate::agents::get_agent("gemini").unwrap().binary),
            "expected gemini binary, got: {cmd}"
        );
    }

    // --- Instance-primary treatment follows the agent, not the slot position ---

    #[test]
    fn test_adopted_primary_slot_relaunches_as_the_agent_it_recorded() {
        // The reported shape: the instance's own tool stayed a shell because both
        // panes were started by hand and only adopted, so slot 0 records `claude`.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "shell".to_string();
        inst.command = "sh".to_string();

        let (cmd, resumed) = inst
            .build_pane_resume_plan(
                "claude",
                "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(resumed, "expected a resume plan, got fresh");
        assert!(
            cmd.contains("exec env claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "the adopted pane must relaunch as its own agent, got: {cmd}"
        );
        assert!(
            !cmd.contains("exec env sh"),
            "the instance's shell must not replace the adopted agent, got: {cmd}"
        );
    }

    /// Cross Agent Team decoration describes the agent that runs in the pane, not
    /// the instance's tool. A Claude instance's development-channels flag handed
    /// to an adopted Gemini pane is a flag Gemini does not understand.
    #[test]
    fn test_cat_decoration_follows_the_pane_agent_not_the_instance_tool() {
        const FLAG: &str = "--dangerously-load-development-channels";

        // Both command-construction paths, because they decorate separately: an
        // instance with no command override builds from the agent's own binary,
        // one with an override builds from the override. Covering only the first
        // leaves the second free to hand the flag to the wrong agent.
        for command in ["", "claude --some-override"] {
            let mut inst = Instance::new("test", "/tmp/test");
            inst.tool = "claude".to_string();
            inst.command = command.to_string();
            inst.cross_agent_team = true;
            assert!(inst.cross_agent_team_pane("claude"));

            let own = inst
                .build_pane_command("claude", None, true, None)
                .expect("claude pane command");
            assert!(
                own.contains(FLAG),
                "the instance's own agent still gets the flag (command {command:?}), got: {own}"
            );

            let adopted = inst
                .build_pane_command("gemini", None, true, None)
                .expect("gemini pane command");
            assert!(
                !adopted.contains(FLAG),
                "an adopted pane running another agent must not get Claude's flag \
                 (command {command:?}), got: {adopted}"
            );
        }
    }

    /// The completion signal is Claude's own input prompt, so a launch that
    /// raises no question -- or only one of the two -- does not have to wait out
    /// the deadline. The questions are drawn with the same glyph, so this must
    /// never be consulted while one is on screen; that ordering is the caller's,
    /// and these cases pin both halves of it.
    #[test]
    fn test_claude_input_prompt_is_the_ready_signal() {
        // The box Claude draws around its prompt, as a real ready pane shows it
        // -- see `at_a_real_ready_screen_reads_as_ready` for the capture these
        // are modeled on. The glyph on its own is a shape a menu shares.
        let rule = "\u{2500}".repeat(40);
        let ready = format!(
            "  Welcome to Claude Code\n  ~/workspace/aoe\n\n{rule}\n\u{276f} \n{rule}\n  main"
        );
        assert!(shows_claude_input_prompt(&ready));
        assert_eq!(auto_confirm_step(&ready, &[]), AutoConfirmStep::NoPrompt);

        let bare = format!("{rule}\n\u{276f}\n{rule}");
        assert!(shows_claude_input_prompt(&bare));

        let unboxed = "\u{276f}";
        assert!(
            !shows_claude_input_prompt(unboxed),
            "the glyph outside the input box is not the input prompt: menus draw \
             it too, and some of them number nothing"
        );

        let blank = "\n\n   \n";
        assert!(
            !shows_claude_input_prompt(blank),
            "a pane that has rendered nothing is not a pane that is ready"
        );

        let starting = "  Welcome to Claude Code\n  \u{280b} starting";
        assert!(
            !shows_claude_input_prompt(starting),
            "no input prompt yet means not ready"
        );

        // A confirmation screen must fail both halves: it is a question, and it
        // is not readiness. It draws the same glyph, so this held only by the
        // caller's ordering until the predicate learned to tell a menu entry
        // from an input prompt -- see `test_menu_selection_is_not_the_input_prompt`
        // for what that costs when the ordering is the only defence.
        let question = "  WARNING: Loading development channels\n  \u{276f} 1. I am using this for local development";
        assert!(
            !shows_claude_input_prompt(question),
            "a selected menu entry is not an input prompt"
        );
        assert_eq!(
            auto_confirm_step(question, &[]),
            AutoConfirmStep::Answer(AutoConfirmPrompt::DevelopmentChannels),
            "a question is still a question, whatever glyph draws it"
        );
    }

    /// A narrow pane is the shape this whole change exists for, and it is where
    /// the question text arrives split across lines. Verbatim from a real Claude
    /// confirmation screen captured at 36 columns.
    #[test]
    fn test_wrapped_question_is_still_recognized() {
        let wrapped = "  Please use --channels to run a\n  list of approved channels.\n  Channels:\n                         server:cross-agent-teams-channel\n  \u{276f} 1. I am using this for local\n                              development\n    2. Exit\n  Enter to confirm \u{b7} Esc to cancel";

        assert!(
            !wrapped.contains("I am using this for local development"),
            "precondition: the phrase really is split in the captured screen"
        );
        assert_eq!(
            auto_confirm_step(wrapped, &[]),
            AutoConfirmStep::Answer(AutoConfirmPrompt::DevelopmentChannels),
            "a question split by the pane width is still that question"
        );
    }

    /// The glyph is Claude's selection marker, not its input prompt: the theme
    /// picker, the login chooser and the startup confirmations all draw it.
    /// Reading any of them as ready abandons a pane on an unanswered question.
    #[test]
    fn test_menu_selection_is_not_the_input_prompt() {
        assert!(is_claude_input_prompt_line("\u{276f}"));
        assert!(is_claude_input_prompt_line("\u{276f} "));
        assert!(is_claude_input_prompt_line(
            "\u{276f} what should I work on?"
        ));

        assert!(!is_claude_input_prompt_line(
            "\u{276f} 1. I am using this for local development"
        ));
        assert!(!is_claude_input_prompt_line("\u{276f} 2. Exit"));
        assert!(!is_claude_input_prompt_line(
            "\u{276f} 1. Claude account with subscription"
        ));
        assert!(!is_claude_input_prompt_line("no glyph here"));

        // The combination that stranded a pane: the question's text is split by
        // the pane width so no marker matches, and its selected entry then reads
        // as an input prompt.
        let wrapped_question =
            "  \u{276f} 1. I am using this for local\n       development\n    2. Exit";
        assert!(
            !shows_claude_input_prompt(wrapped_question),
            "a menu on screen is not a pane that is done being asked"
        );
    }

    /// A screen can carry the text of a question already answered above the one
    /// now being asked. Reporting the first marker found reports history as the
    /// current state and leaves the real question unanswered.
    #[test]
    fn test_answered_prompt_on_screen_does_not_mask_an_unanswered_one() {
        let both = "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development\n\n                     Quick safety check: Is this a project you created or one you trust?\n ❯ 1. Yes, I trust this folder";

        assert_eq!(
            auto_confirm_step(both, &[AutoConfirmPrompt::DevelopmentChannels]),
            AutoConfirmStep::Answer(AutoConfirmPrompt::WorkspaceTrust),
            "the unanswered question on screen is the one to answer"
        );
        assert_eq!(
            auto_confirm_step(both, &[]),
            AutoConfirmStep::Answer(AutoConfirmPrompt::DevelopmentChannels),
            "with nothing answered yet, the first question present is answered first"
        );
        assert_eq!(
            auto_confirm_step(
                both,
                &[
                    AutoConfirmPrompt::DevelopmentChannels,
                    AutoConfirmPrompt::WorkspaceTrust
                ]
            ),
            AutoConfirmStep::AlreadyAnswered,
            "only when every question present is answered is there nothing to do"
        );
    }

    /// Cross Agent Team integration is decided by the agent in the pane. Both
    /// heterogeneous directions must get the integration their own agent needs,
    /// not merely be spared the one they do not.
    #[test]
    fn test_cat_integration_reaches_both_heterogeneous_directions() {
        const CLAUDE_FLAG: &str = "--dangerously-load-development-channels";

        // A Claude pane adopted into a Codex instance.
        let mut codex_inst = Instance::new("test", "/tmp/test");
        codex_inst.tool = "codex".to_string();
        codex_inst.cross_agent_team = true;
        let adopted_claude = codex_inst
            .build_pane_command("claude", None, false, None)
            .expect("claude pane command");
        assert!(
            adopted_claude.contains(CLAUDE_FLAG),
            "a Claude pane needs Claude's channel flag even in a Codex instance, got: {adopted_claude}"
        );

        // A Codex pane adopted into a Claude instance.
        let mut claude_inst = Instance::new("test", "/tmp/test");
        claude_inst.tool = "claude".to_string();
        claude_inst.cross_agent_team = true;
        let adopted_codex = claude_inst
            .build_pane_command("codex", None, false, None)
            .expect("codex pane command");
        assert!(
            !adopted_codex.contains(CLAUDE_FLAG),
            "a Codex pane must not carry Claude's flag, got: {adopted_codex}"
        );
        assert!(
            adopted_codex.contains(CODEX_XATS_PACKAGE),
            "a Codex pane needs Codex's bootstrap even in a Claude instance, got: {adopted_codex}"
        );
        // The bootstrap being present says nothing about what it launches. The
        // binary it execs is the assertion that matters: built from the
        // instance's tool, this bootstrap runs Claude under Codex's xats
        // integration, and the package name is there either way.
        assert!(
            adopted_codex.contains("codex --remote"),
            "the Codex bootstrap must exec Codex, got: {adopted_codex}"
        );
        // And the spec it asks npx for must carry the tag. `--no-install`
        // resolves against its cache by the exact spec, so a bare name misses
        // the `@latest` entry that xats's own launcher creates -- npx then
        // reports the package missing, refuses to run it, and the pane comes up
        // as a shell. Observed on a machine with the right version cached.
        assert!(
            adopted_codex.contains("cross-agent-teams-mcp@latest"),
            "the bootstrap must ask npx for the tagged spec, not a bare name, \
             got: {adopted_codex}"
        );
        assert!(
            !adopted_codex.contains("claude --remote"),
            "the Codex bootstrap must not exec the instance's own agent, got: {adopted_codex}"
        );
    }

    /// A Codex instance's own agent pane has no Claude questions to answer.
    /// Sending it into the Claude flow costs the full deadline synchronously on
    /// every launch, so the entry point has to know whose pane it speaks for.
    #[test]
    fn test_agent_pane_claude_prompts_follow_the_panes_agent() {
        let mut claude_inst = Instance::new("test", "/tmp/test");
        claude_inst.tool = "claude".to_string();
        claude_inst.cross_agent_team = true;
        assert!(claude_inst.agent_pane_has_claude_prompts("claude"));

        let mut codex_inst = Instance::new("test", "/tmp/test");
        codex_inst.tool = "codex".to_string();
        codex_inst.cross_agent_team = true;
        assert!(
            !codex_inst.agent_pane_has_claude_prompts("codex"),
            "a Codex pane raises no Claude question"
        );
        // The instance's tool does not decide this: a Codex instance whose pane
        // was handed to Claude has Claude's questions to answer, and a Claude
        // instance whose pane runs something else has none.
        assert!(
            codex_inst.agent_pane_has_claude_prompts("claude"),
            "an adopted Claude pane raises Claude's questions whatever the \
             instance's own tool is"
        );
        assert!(
            !claude_inst.agent_pane_has_claude_prompts("codex"),
            "a Claude instance whose pane runs Codex has no Claude question in it"
        );
        // It still takes Claude's integration for an adopted Claude pane: the
        // two questions are different and must not collapse into one predicate.
        assert!(codex_inst.cross_agent_team_pane("claude"));

        let mut off = Instance::new("test", "/tmp/test");
        off.tool = "claude".to_string();
        assert!(
            !off.agent_pane_has_claude_prompts("claude"),
            "Cross Agent Team off means no development-channel question at all"
        );
    }

    /// An adopted slot 0 is described by neither key source unless one of them is
    /// widened: the instance record holds the key for the instance's own agent,
    /// and the slot-key path used to skip slot 0 outright.
    #[test]
    fn test_adopted_slot_zero_needs_its_own_identity_key() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;

        let own = recovered_slot(0, "claude", "/tmp/test", "%0");
        assert!(
            !inst.slot_needs_identity_key(&own),
            "slot 0 running the instance's own agent is covered by the instance record"
        );

        let adopted = recovered_slot(0, "gemini", "/tmp/test", "%0");
        assert!(
            inst.slot_needs_identity_key(&adopted),
            "an adopted slot 0 has no other key source, so it must get its own"
        );

        let secondary = recovered_slot(1, "gemini", "/tmp/test", "%1");
        assert!(
            inst.slot_needs_identity_key(&secondary),
            "a secondary adopted slot keeps needing its own key"
        );
    }

    #[test]
    fn test_mismatched_primary_slot_carries_no_instance_conversation_identity() {
        // A pre-allocated conversation id, a pending fork and the instance's extra
        // arguments all describe `self.tool`; a slot running a different agent must
        // receive none of them.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.agent_session_id = Some("11111111-1111-4111-8111-111111111111".to_string());
        inst.fork_pending = Some("22222222-2222-4222-8222-222222222222".to_string());
        inst.extra_args = "--instance-only".to_string();

        let (cmd, _) = inst
            .build_pane_resume_plan("codex", "", true, RestartMode::Fresh, None)
            .unwrap();
        assert!(
            cmd.contains("exec env codex"),
            "expected the slot's own binary, got: {cmd}"
        );
        assert!(
            !cmd.contains("11111111-1111-4111-8111-111111111111"),
            "the instance's conversation id must not reach another agent, got: {cmd}"
        );
        assert!(
            !cmd.contains("22222222-2222-4222-8222-222222222222"),
            "the instance's fork token must not reach another agent, got: {cmd}"
        );
        assert!(
            !cmd.contains("--instance-only"),
            "the instance's extra args must not reach another agent, got: {cmd}"
        );
    }

    #[test]
    fn test_matching_slot_keeps_the_instances_own_launch_context() {
        // A slot recording the instance's own tool must still build exactly what
        // the single-pane launch path builds: command override plus extra args.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.command = "claude --wrapper".to_string();
        inst.extra_args = "--instance-only".to_string();

        let (cmd, _) = inst
            .build_pane_resume_plan("claude", "", true, RestartMode::Fresh, None)
            .unwrap();
        assert_eq!(
            cmd,
            inst.build_agent_command(None).unwrap(),
            "a matching slot must build the instance's own launch command byte for byte"
        );
        assert!(cmd.contains("--wrapper"), "got: {cmd}");
        assert!(cmd.contains("--instance-only"), "got: {cmd}");
    }

    #[test]
    fn test_matching_slot_keeps_pre_allocated_session_id_and_fork_token() {
        let parent = parent_instance("claude", Some("parent-uuid"));
        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let new_id = fork.agent_session_id.clone().expect("fork allocates an id");

        let (cmd, _) = fork
            .build_pane_resume_plan("claude", "", true, RestartMode::Fresh, None)
            .unwrap();
        assert!(
            cmd.contains("--fork-session"),
            "expected the pending fork to still apply, got: {cmd}"
        );
        assert!(
            cmd.contains(&new_id),
            "expected the pre-allocated session id to still apply, got: {cmd}"
        );
    }

    #[test]
    fn test_a_later_slot_running_the_instance_tool_stays_secondary() {
        // Two slots recording the same agent is the ordinary case (a user running
        // `claude` in both halves of a split). The instance has one conversation
        // id, one pending fork and one identity key, and slot 0 is what names the
        // pane they belong to: a later slot recording the same agent must build
        // from the bare binary and receive none of them.
        let mut inst = claude_xats_instance();
        inst.agent_session_id = Some("11111111-1111-4111-8111-111111111111".to_string());
        inst.fork_pending = Some("22222222-2222-4222-8222-222222222222".to_string());
        inst.extra_args = "--instance-only".to_string();
        inst.xats_identity_key = Some("instance-key".to_string());

        let (cmd, _) = inst
            .build_pane_resume_plan("claude", "", false, RestartMode::Fresh, None)
            .unwrap();
        assert!(
            cmd.contains("exec env claude"),
            "expected the agent's own binary, got: {cmd}"
        );
        assert!(
            !cmd.contains("11111111-1111-4111-8111-111111111111"),
            "a second pane must not claim the instance's conversation id, got: {cmd}"
        );
        assert!(
            !cmd.contains("--fork-session")
                && !cmd.contains("22222222-2222-4222-8222-222222222222"),
            "a second pane must not replay the instance's pending fork, got: {cmd}"
        );
        assert!(
            !cmd.contains("--instance-only"),
            "the instance's extra args describe its own pane only, got: {cmd}"
        );
        assert!(
            !cmd.contains("instance-key"),
            "a second pane must not claim the instance's identity, got: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_unknown_safe_agent_uses_recorded_name_fresh() {
        // An unknown but safe agent name cannot be decorated; it degrades to a
        // bare-binary fresh launch.
        let inst = Instance::new("test", "/tmp/test");
        let (cmd, resumed) = inst
            .build_pane_resume_plan("mystery", "some-id", false, RestartMode::Resume, None)
            .unwrap();
        assert!(!resumed);
        assert_eq!(cmd, "mystery");
    }

    // --- Slots that did not come back after a rebuild ---

    fn recovered_slot(slot: i64, agent: &str, cwd: &str, pane: &str) -> crate::db::AgentSlot {
        crate::db::AgentSlot {
            instance_id: "inst".to_string(),
            slot,
            agent: agent.to_string(),
            native_session_id: String::new(),
            cwd: cwd.to_string(),
            tmux_pane: pane.to_string(),
            xats_identity_key: String::new(),
            last_seen_at: 0,
        }
    }

    #[test]
    fn test_missing_slot_failures_names_the_agent_and_directory() {
        let paired = vec![(
            recovered_slot(1, "claude", "/tmp/other", "%9"),
            Some("%9".to_string()),
        )];
        let live = std::collections::HashSet::new();

        let failures = missing_slot_failures(&paired, &live);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, 0);
        assert!(
            failures[0].1.contains("claude") && failures[0].1.contains("/tmp/other"),
            "the report must name what the user recognizes, got: {}",
            failures[0].1
        );
    }

    #[test]
    fn test_missing_slot_failures_ignores_slots_whose_pane_is_still_there() {
        let paired = vec![
            (
                recovered_slot(0, "claude", "/tmp/project", "%1"),
                Some("%1".to_string()),
            ),
            (
                recovered_slot(1, "shell", "/tmp/other", "%2"),
                Some("%2".to_string()),
            ),
        ];
        let live: std::collections::HashSet<String> = ["%1".to_string()].into_iter().collect();

        let failures = missing_slot_failures(&paired, &live);
        assert_eq!(failures.len(), 1, "only the vanished pane is reported");
        assert_eq!(failures[0].0, 1);
    }

    #[test]
    fn test_missing_slot_failures_skips_a_slot_that_never_got_a_pane() {
        // Pane creation already failed for this slot and was reported as such;
        // saying it twice would only repeat the same fact in a second voice.
        let paired = vec![(recovered_slot(1, "claude", "/tmp/other", "%9"), None)];
        let live = std::collections::HashSet::new();

        assert!(missing_slot_failures(&paired, &live).is_empty());
    }

    #[test]
    fn test_build_pane_resume_plan_unsafe_unknown_agent_is_rejected() {
        // An unknown agent name with shell metacharacters must not be executed.
        let inst = Instance::new("test", "/tmp/test");
        assert!(inst
            .build_pane_resume_plan(
                "evil; rm -rf ~",
                "some-id",
                false,
                RestartMode::Resume,
                None
            )
            .is_none());
    }

    fn sandboxed_instance(tool: &str) -> Instance {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = tool.to_string();
        inst.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        inst
    }

    // --- Slot-resume launch-context preservation (fix-resume-preserves-launch-context) ---

    #[test]
    fn test_slot_resume_yolo_cliflag_keeps_flag_and_resume_token() {
        // A YOLO CliFlag agent (claude) resumed via the slot path must carry both
        // the YOLO flag and the resume flag built from native_session_id.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = true;

        let (cmd, resumed) = inst
            .build_pane_resume_plan(
                "claude",
                "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(resumed, "expected a resume plan, got fresh");
        assert!(
            cmd.contains("claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "expected resume flag from native_session_id, got: {cmd}"
        );
        assert!(
            cmd.contains("--dangerously-skip-permissions"),
            "expected YOLO CliFlag, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_yolo_envvar_sets_env_var() {
        // A YOLO EnvVar agent resumed via the slot path must set the YOLO env var.
        // opencode is sandbox-only on the real host path, so the host EnvVar branch
        // is reached here through a command override equal to the binary (a real,
        // reachable configuration that still exercises the EnvVar decoration).
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "opencode".to_string();
        inst.command = "opencode".to_string();
        inst.yolo_mode = true;

        let cmd = inst
            .build_pane_command("opencode", None, true, None)
            .expect("opencode command override should build");
        assert!(
            cmd.contains("OPENCODE_PERMISSION="),
            "expected YOLO env var, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_hook_agent_sets_instance_id() {
        // A hook-config agent (claude) resumed via the slot path must carry
        // AOE_INSTANCE_ID set to the instance id.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let id = inst.id.clone();

        let (cmd, _resumed) = inst
            .build_pane_resume_plan(
                "claude",
                "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(
            cmd.contains(&format!("AOE_INSTANCE_ID='{id}'")),
            "expected AOE_INSTANCE_ID env, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_sandboxed_is_docker_wrapped() {
        // A sandboxed instance resumed via the slot path must be docker-exec
        // wrapped into the instance's container, not a bare host binary.
        let inst = sandboxed_instance("claude");
        let container = DockerContainer::generate_name(&inst.id);

        let (cmd, resumed) = inst
            .build_pane_resume_plan(
                "claude",
                "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(resumed);
        assert!(
            cmd.contains("exec -it") && cmd.contains(&container),
            "expected docker exec into {container}, got: {cmd}"
        );
        assert!(
            cmd.contains("claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "expected resume flag inside container command, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_non_yolo_has_no_yolo_flag_or_env() {
        // A non-YOLO instance resumed via the slot path must not gain any YOLO
        // flag or YOLO env var.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = false;

        let (cmd, _resumed) = inst
            .build_pane_resume_plan(
                "claude",
                "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(
            !cmd.contains("--dangerously-skip-permissions"),
            "non-YOLO must not carry YOLO flag, got: {cmd}"
        );
        assert!(
            !cmd.contains("OPENCODE_PERMISSION"),
            "non-YOLO must not carry YOLO env, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_degraded_fresh_keeps_launch_context() {
        // A pane with no usable resume token (invalid native_session_id) must
        // still launch fresh WITH full launch context (YOLO flag, hook env),
        // never a bare binary.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = true;
        let id = inst.id.clone();

        let (cmd, resumed) = inst
            .build_pane_resume_plan(
                "claude",
                "not a valid; token",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(!resumed, "invalid token must degrade to fresh");
        assert!(
            !cmd.contains("--resume"),
            "degraded-fresh must not carry a resume flag, got: {cmd}"
        );
        assert!(
            !cmd.contains("not a valid"),
            "unsafe token must not be interpolated, got: {cmd}"
        );
        assert!(
            cmd.contains("--dangerously-skip-permissions"),
            "degraded-fresh must still carry YOLO flag, got: {cmd}"
        );
        assert!(
            cmd.contains(&format!("AOE_INSTANCE_ID='{id}'")),
            "degraded-fresh must still carry hook env, got: {cmd}"
        );
        let binary = crate::agents::get_agent("claude").unwrap().binary;
        assert_ne!(cmd, binary, "degraded-fresh must not be a bare binary");
    }

    #[test]
    fn test_slot_resume_injection_guard_intact() {
        // An unsafe/unknown slot agent name is refused (None); an invalid resume
        // token degrades to fresh without interpolating the raw value.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();

        assert!(
            inst.build_pane_resume_plan(
                "evil; rm -rf ~",
                "some-id",
                true,
                RestartMode::Resume,
                None
            )
            .is_none(),
            "unsafe agent name must be refused"
        );

        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "abc; rm -rf ~", true, RestartMode::Resume, None)
            .unwrap();
        assert!(!resumed);
        assert!(
            !cmd.contains("rm -rf"),
            "invalid resume token must not be interpolated: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_heterogeneous_panes_apply_own_yolo_variant() {
        // A YOLO instance whose slots record different agents must apply each
        // pane's own YoloMode variant: claude (CliFlag) gets the flag; pi
        // (AlwaysYolo) gets neither a flag nor an env var.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = true;

        let (primary, _) = inst
            .build_pane_resume_plan(
                "claude",
                "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
                true,
                RestartMode::Resume,
                None,
            )
            .unwrap();
        assert!(
            primary.contains("--dangerously-skip-permissions"),
            "claude pane must carry its CliFlag, got: {primary}"
        );

        let (secondary, _) = inst
            .build_pane_resume_plan("pi", "ignored", false, RestartMode::Resume, None)
            .unwrap();
        assert!(
            !secondary.contains("--dangerously-skip-permissions"),
            "pi (AlwaysYolo) pane must not carry claude's flag, got: {secondary}"
        );
        assert!(
            secondary.contains("pi"),
            "pi pane must launch the pi binary, got: {secondary}"
        );
    }

    #[test]
    fn test_is_recoverable_slots_and_dead_session() {
        // Has persisted slots AND tmux session dead => recoverable.
        assert!(is_recoverable_from(true, false));
    }

    #[test]
    fn test_is_recoverable_live_session_never_recoverable() {
        // Live session is never recoverable regardless of slots.
        assert!(!is_recoverable_from(true, true));
    }

    #[test]
    fn test_is_recoverable_no_slots_not_recoverable() {
        // No persisted slots => not recoverable even when the session is dead.
        assert!(!is_recoverable_from(false, false));
        assert!(!is_recoverable_from(false, true));
    }

    // Tests for Status enum
    #[test]
    fn test_status_default() {
        let status = Status::default();
        assert_eq!(status, Status::Idle);
    }

    #[test]
    fn test_status_serialization() {
        let statuses = vec![
            Status::Running,
            Status::Waiting,
            Status::Idle,
            Status::Unknown,
            Status::Stopped,
            Status::Error,
            Status::Starting,
            Status::Restarting,
            Status::Deleting,
        ];

        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: Status = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    // Tests for WorktreeInfo
    #[test]
    fn test_worktree_info_serialization() {
        let info = WorktreeInfo {
            branch: "feature/test".to_string(),
            main_repo_path: "/home/user/repo".to_string(),
            managed_by_aoe: true,
            created_at: Utc::now(),
            cleanup_on_delete: true,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: WorktreeInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(info.branch, deserialized.branch);
        assert_eq!(info.main_repo_path, deserialized.main_repo_path);
        assert_eq!(info.managed_by_aoe, deserialized.managed_by_aoe);
    }

    // Tests for SandboxInfo
    #[test]
    fn test_sandbox_info_serialization() {
        let info = SandboxInfo {
            enabled: true,
            container_id: Some("abc123".to_string()),
            image: "myimage:latest".to_string(),
            container_name: "test_container".to_string(),
            created_at: Some(Utc::now()),
            extra_env: Some(vec!["MY_VAR".to_string(), "OTHER_VAR".to_string()]),
            custom_instruction: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: SandboxInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(info.enabled, deserialized.enabled);
        assert_eq!(info.container_id, deserialized.container_id);
        assert_eq!(info.image, deserialized.image);
        assert_eq!(info.container_name, deserialized.container_name);
        assert_eq!(info.extra_env, deserialized.extra_env);
    }

    #[test]
    fn test_sandbox_info_minimal_serialization() {
        // Required fields: enabled, image, container_name
        let json = r#"{"enabled":false,"image":"test-image","container_name":"test"}"#;
        let info: SandboxInfo = serde_json::from_str(json).unwrap();

        assert!(!info.enabled);
        assert_eq!(info.image, "test-image");
        assert_eq!(info.container_name, "test");
        assert!(info.container_id.is_none());
        assert!(info.created_at.is_none());
    }

    // Tests for Instance serialization
    #[test]
    fn test_instance_serialization_roundtrip() {
        let mut inst = Instance::new("Test Project", "/home/user/project");
        inst.tool = "claude".to_string();
        inst.group_path = "work/clients".to_string();
        inst.command = "claude --resume xyz".to_string();

        let json = serde_json::to_string(&inst).unwrap();
        let deserialized: Instance = serde_json::from_str(&json).unwrap();

        assert_eq!(inst.id, deserialized.id);
        assert_eq!(inst.title, deserialized.title);
        assert_eq!(inst.project_path, deserialized.project_path);
        assert_eq!(inst.group_path, deserialized.group_path);
        assert_eq!(inst.tool, deserialized.tool);
        assert_eq!(inst.command, deserialized.command);
    }

    #[test]
    fn test_instance_deserialization_defaults_resume_token_to_none() {
        let json = r#"{
            "id":"deadbeefcafebabe",
            "title":"Test Project",
            "project_path":"/tmp/test-project",
            "status":"idle",
            "created_at":"2024-01-01T00:00:00Z"
        }"#;

        let deserialized: Instance = serde_json::from_str(json).unwrap();

        assert!(deserialized.resume_token.is_none());
    }

    #[test]
    fn test_instance_resume_token_roundtrip() {
        let mut inst = Instance::new("Test Project", "/home/user/project");
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        let json = serde_json::to_string(&inst).unwrap();
        let deserialized: Instance = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.resume_token.as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_instance_serialization_skips_runtime_fields() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.last_error_check = Some(std::time::Instant::now());
        inst.last_start_time = Some(std::time::Instant::now());
        inst.last_error = Some("test error".to_string());

        let json = serde_json::to_string(&inst).unwrap();

        // Runtime fields should not appear in JSON
        assert!(!json.contains("last_error_check"));
        assert!(!json.contains("last_start_time"));
        assert!(!json.contains("last_error"));
    }

    #[test]
    fn test_instance_with_worktree_info() {
        let mut inst = Instance::new("Test", "/tmp/worktree");
        inst.worktree_info = Some(WorktreeInfo {
            branch: "feature/abc".to_string(),
            main_repo_path: "/tmp/main".to_string(),
            managed_by_aoe: true,
            created_at: Utc::now(),
            cleanup_on_delete: true,
        });

        let json = serde_json::to_string(&inst).unwrap();
        let deserialized: Instance = serde_json::from_str(&json).unwrap();

        assert!(deserialized.worktree_info.is_some());
        let wt = deserialized.worktree_info.unwrap();
        assert_eq!(wt.branch, "feature/abc");
        assert!(wt.managed_by_aoe);
    }

    // Test generate_id function properties
    #[test]
    fn test_generate_id_uniqueness() {
        let ids: Vec<String> = (0..100).map(|_| Instance::new("t", "/t").id).collect();
        let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique_ids.len());
    }

    #[test]
    fn test_generate_id_format() {
        let inst = Instance::new("test", "/tmp/test");
        // ID should be 16 hex characters
        assert_eq!(inst.id.len(), 16);
        assert!(inst.id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_has_custom_command_empty() {
        let inst = Instance::new("test", "/tmp/test");
        assert!(!inst.has_custom_command());
    }

    #[test]
    fn test_has_custom_command_treats_extra_args_as_custom() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.extra_args = "--model sonnet".to_string();
        assert!(inst.has_custom_command());
    }

    #[test]
    fn test_has_custom_command_same_as_agent_binary() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.command = "claude".to_string();
        assert!(!inst.has_custom_command());
    }

    #[test]
    fn test_has_custom_command_override() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.command = "my-wrapper".to_string();
        assert!(inst.has_custom_command());
    }

    #[test]
    fn test_has_custom_command_unknown_tool() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "unknown_agent".to_string();
        inst.command = "some-binary".to_string();
        assert!(inst.has_custom_command());
    }

    #[test]
    fn test_extra_args_without_command_override_is_not_a_command_override() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.extra_args = "--model sonnet".to_string();
        assert!(!inst.has_command_override());
    }

    #[test]
    fn test_is_valid_resume_token_accepts_hex_and_hyphen() {
        assert!(is_valid_resume_token(
            "019d1af9-a899-7df1-8f7d-a244126e5ded"
        ));
        assert!(is_valid_resume_token(
            "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"
        ));
    }

    #[test]
    fn test_is_valid_resume_token_rejects_invalid_characters() {
        assert!(!is_valid_resume_token(""));
        assert!(!is_valid_resume_token("abc def"));
        assert!(!is_valid_resume_token("abc$def"));
        assert!(!is_valid_resume_token("resume-token"));
    }

    #[test]
    fn test_expects_shell() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert!(!inst.expects_shell());

        inst.tool = "unknown-tool".to_string();
        inst.command = String::new();
        assert!(inst.expects_shell());

        inst.tool = "claude".to_string();
        inst.command = "bash".to_string();
        assert!(inst.expects_shell());

        inst.command = "my-agent".to_string();
        assert!(!inst.expects_shell());
    }

    #[test]
    fn test_status_unknown_serialization() {
        let status = Status::Unknown;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"unknown\"");
        let deserialized: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, Status::Unknown);
    }

    #[test]
    fn test_restart_in_flight_is_runtime_only() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.restart_in_flight = true;

        let json = serde_json::to_string(&inst).unwrap();
        assert!(!json.contains("restart_in_flight"));
    }

    #[test]
    fn test_clear_resume_token_helper_clears_stored_token() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        inst.clear_resume_token();

        assert!(inst.resume_token.is_none());
    }

    #[test]
    fn test_resolved_resume_token_uses_stored_token_when_explicit_token_missing() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        assert_eq!(
            inst.resolved_resume_token(None).as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_resolved_resume_token_prefers_explicit_token() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.resume_token = Some("stored-token".to_string());

        assert_eq!(
            inst.resolved_resume_token(Some("019d1af9-a899-7df1-8f7d-a244126e5ded"))
                .as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_dead_pane_restart_would_use_stored_resume_token_when_available() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        // The tmux-backed dead-pane restart is covered at runtime. This unit
        // test covers the stored-token selection used by that branch.
        assert_eq!(
            inst.resolved_resume_token(None).as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_acknowledged_waiting_maps_to_idle() {
        let mut inst = Instance::new("test", "/tmp/test");

        assert_eq!(
            inst.apply_acknowledged_mapping(Status::Waiting),
            Status::Waiting
        );

        inst.acknowledged = true;
        assert_eq!(
            inst.apply_acknowledged_mapping(Status::Waiting),
            Status::Idle
        );
        assert_eq!(
            inst.apply_acknowledged_mapping(Status::Running),
            Status::Running
        );
    }

    #[test]
    fn test_spinner_grace_period_holds_running() {
        let mut inst = Instance::new("test", "/tmp/test");
        let now = Instant::now();
        inst.last_spinner_seen = Some(now);

        assert_eq!(
            inst.apply_spinner_grace_period(
                Status::Idle,
                Status::Running,
                now + Duration::from_millis(400)
            ),
            Status::Running
        );
        assert_eq!(
            inst.apply_spinner_grace_period(
                Status::Idle,
                Status::Running,
                now + Duration::from_millis(600)
            ),
            Status::Idle
        );
    }

    #[test]
    fn test_spike_detection_requires_confirmation() {
        let mut inst = Instance::new("test", "/tmp/test");
        let now = Instant::now();

        let first = inst.apply_spike_detection(Status::Running, Status::Idle, now);
        assert_eq!(first, Status::Idle);
        assert!(inst.spike_start.is_some());
        assert_eq!(inst.pre_spike_status, Some(Status::Idle));

        let second = inst.apply_spike_detection(
            Status::Running,
            Status::Idle,
            now + Duration::from_millis(500),
        );
        assert_eq!(second, Status::Running);
        assert!(inst.spike_start.is_none());
        assert!(inst.pre_spike_status.is_none());
    }

    #[test]
    fn test_spike_detection_rejects_transient_running() {
        let mut inst = Instance::new("test", "/tmp/test");
        let now = Instant::now();

        let first = inst.apply_spike_detection(Status::Running, Status::Waiting, now);
        assert_eq!(first, Status::Waiting);
        assert!(inst.spike_start.is_some());

        let second = inst.apply_spike_detection(
            Status::Idle,
            Status::Waiting,
            now + Duration::from_millis(500),
        );
        assert_eq!(second, Status::Idle);
        assert!(inst.spike_start.is_none());
        assert!(inst.pre_spike_status.is_none());
    }

    // --- Fork session tests ----------------------------------------------

    fn parent_instance(tool: &str, token: Option<&str>) -> Instance {
        let mut inst = Instance::new("parent", "/tmp/project");
        inst.tool = tool.to_string();
        inst.group_path = "work".to_string();
        inst.extra_args = "--verbose".to_string();
        inst.yolo_mode = true;
        inst.resume_token = token.map(|s| s.to_string());
        inst
    }

    #[test]
    fn test_create_fork_inherits_parent_fields() {
        let parent = parent_instance("claude", Some("abc-123"));
        let fork = parent
            .create_fork("my-fork".to_string(), Some("experiments".to_string()))
            .expect("fork should succeed");

        assert_ne!(fork.id, parent.id);
        assert_eq!(fork.id.len(), parent.id.len());
        assert_eq!(fork.title, "my-fork");
        assert_eq!(fork.project_path, parent.project_path);
        assert_eq!(fork.tool, parent.tool);
        assert_eq!(fork.extra_args, parent.extra_args);
        assert_eq!(fork.yolo_mode, parent.yolo_mode);
        assert_eq!(fork.group_path, "experiments");
        assert_eq!(fork.parent_session_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(fork.fork_pending.as_deref(), Some("abc-123"));
        // Runtime state is reset.
        assert!(fork.resume_token.is_none());
        assert!(fork.last_error.is_none());
        assert_eq!(fork.status, Status::Idle);
        assert!(!fork.acknowledged);
    }

    #[test]
    fn test_create_fork_defaults_to_parent_group() {
        let parent = parent_instance("codex", Some("xyz"));
        let fork = parent
            .create_fork("sibling".to_string(), None)
            .expect("fork should succeed");
        assert_eq!(fork.group_path, parent.group_path);
    }

    #[test]
    fn test_create_fork_rejects_unsupported_tool() {
        let parent = parent_instance("gemini", Some("ignored"));
        let err = parent
            .create_fork("bad".to_string(), None)
            .expect_err("gemini does not support forking");
        let msg = err.to_string();
        assert!(
            msg.contains("Fork is not supported"),
            "expected unsupported-tool error, got: {msg}"
        );
    }

    #[test]
    fn test_create_fork_rejects_missing_codex_token() {
        let parent = parent_instance("codex", None);
        let err = parent
            .create_fork("too-early".to_string(), None)
            .expect_err("codex without a resume token should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("No active codex session"),
            "expected missing-token error, got: {msg}"
        );
    }

    #[test]
    fn test_create_fork_claude_no_token_falls_back_to_disk() {
        // Claude without resume_token should attempt disk scan.
        // In the test environment there's no Claude project directory,
        // so it should fail with a "No active Claude session" error.
        let parent = parent_instance("claude", None);
        let err = parent
            .create_fork("no-disk".to_string(), None)
            .expect_err("claude without token or disk session should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("No active Claude session"),
            "expected missing-session error, got: {msg}"
        );
    }

    #[test]
    fn test_create_fork_rejects_command_override() {
        let mut parent = parent_instance("claude", Some("abc"));
        parent.command = "claude --some-weird-wrapper".to_string();
        let err = parent
            .create_fork("nope".to_string(), None)
            .expect_err("command override should block fork");
        assert!(err.to_string().contains("custom command override"));
    }

    #[test]
    fn test_create_fork_clears_worktree_cleanup_flag() {
        let mut parent = parent_instance("claude", Some("abc"));
        parent.worktree_info = Some(WorktreeInfo {
            branch: "main".to_string(),
            main_repo_path: "/tmp/project".to_string(),
            managed_by_aoe: true,
            created_at: Utc::now(),
            cleanup_on_delete: true,
        });
        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let wt = fork.worktree_info.expect("worktree inherited");
        assert!(!wt.cleanup_on_delete);
        assert_eq!(wt.branch, "main");
    }

    #[test]
    fn test_create_fork_generates_new_container_name() {
        let mut parent = parent_instance("claude", Some("abc"));
        parent.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: Some("parent-container-id".to_string()),
            image: "ubuntu:latest".to_string(),
            container_name: DockerContainer::generate_name(&parent.id),
            created_at: Some(Utc::now()),
            extra_env: None,
            custom_instruction: None,
        });
        let parent_container_name = parent.sandbox_info.as_ref().unwrap().container_name.clone();

        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let sandbox = fork.sandbox_info.expect("sandbox inherited");
        assert_ne!(sandbox.container_name, parent_container_name);
        assert_eq!(
            sandbox.container_name,
            DockerContainer::generate_name(&fork.id)
        );
        assert!(sandbox.container_id.is_none());
    }

    #[test]
    fn test_build_base_tool_command_uses_fork_template_for_claude() {
        let parent = parent_instance("claude", Some("parent-uuid"));
        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let agent = crate::agents::get_agent("claude");
        let cmd = fork.build_base_pane_command(agent, None, true);
        assert!(
            cmd.contains("claude --resume parent-uuid --fork-session"),
            "expected claude fork command, got: {cmd}"
        );
    }

    #[test]
    fn test_build_base_tool_command_uses_fork_template_for_codex() {
        let parent = parent_instance("codex", Some("parent-uuid"));
        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let agent = crate::agents::get_agent("codex");
        let cmd = fork.build_base_pane_command(agent, None, true);
        assert!(
            cmd.contains("codex fork parent-uuid"),
            "expected codex fork command, got: {cmd}"
        );
    }

    #[test]
    fn test_build_base_tool_command_resume_beats_fork_pending() {
        // Once AoE has captured a real resume token, fork_pending must not
        // override it. This guards the second-launch transition.
        let parent = parent_instance("claude", Some("old-parent-uuid"));
        let mut fork = parent.create_fork("f".to_string(), None).unwrap();
        fork.resume_token = Some("new-fork-uuid".to_string());
        let agent = crate::agents::get_agent("claude");
        let cmd = fork.build_base_pane_command(agent, Some("new-fork-uuid"), true);
        assert!(
            cmd.contains("--resume new-fork-uuid") && !cmd.contains("--fork-session"),
            "expected plain resume command, got: {cmd}"
        );
    }

    #[test]
    fn test_deserialize_instance_defaults_fork_pending_none() {
        let json = r#"{
            "id":"abc","title":"t","project_path":"/p","command":"",
            "created_at":"2020-01-01T00:00:00Z"
        }"#;
        let inst: Instance = serde_json::from_str(json).expect("parseable");
        assert!(inst.fork_pending.is_none());
    }

    // --- Hook status freshness gating tests ---
    //
    // These tests exercise the decision that `update_status_with_options` uses
    // to choose between trusting the hook file and falling through to
    // content-based detection. The gate itself lives in
    // `crate::hooks::read_hook_status_with_freshness`; here we verify that
    // fresh files produce an authoritative read, stale files fall through,
    // and missing files are reported as absent.
    //
    // We cannot drive the full `update_status_with_options` path in a unit
    // test because it requires a live tmux session, so the gate is the
    // smallest unit that captures the behavior change introduced by this
    // feature.

    fn write_hook_status_for(instance_id: &str, value: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new("/tmp/aoe-hooks").join(instance_id);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("status");
        std::fs::write(&path, value).unwrap();
        path
    }

    fn set_mtime_seconds_ago(path: &std::path::Path, seconds: u64) {
        // Shell out to `touch -t` which is POSIX and works on macOS and Linux
        // without adding a new crate dependency for tests.
        use chrono::{Local, TimeZone};
        let target = Local::now() - chrono::Duration::seconds(seconds as i64);
        let stamp = Local
            .timestamp_opt(target.timestamp(), 0)
            .single()
            .unwrap()
            .format("%Y%m%d%H%M.%S")
            .to_string();
        let status = std::process::Command::new("touch")
            .args(["-t", &stamp, path.to_str().unwrap()])
            .status()
            .expect("touch should run");
        assert!(status.success(), "touch -t failed for {:?}", path);
    }

    #[test]
    fn test_update_status_fresh_hook_running_short_circuits() {
        let id = "test_upd_fresh_hook_running";
        let path = write_hook_status_for(id, "running");
        let read = crate::hooks::read_hook_status_with_freshness(id).expect("file present");
        assert!(read.fresh, "just-written file must be fresh");
        assert_eq!(read.status, Status::Running);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn test_update_status_stale_hook_running_falls_through() {
        let id = "test_upd_stale_hook_running";
        let path = write_hook_status_for(id, "running");
        set_mtime_seconds_ago(&path, 120);
        let read = crate::hooks::read_hook_status_with_freshness(id).expect("file present");
        assert!(!read.fresh, "file older than window must be stale");
        assert_eq!(read.status, Status::Running);
        assert!(read.age.as_secs() >= 60, "age should reflect mtime");
        // The poller is expected to ignore `read.status` when !fresh and let
        // content detection drive the final result.
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn test_update_status_stale_hook_waiting_falls_through() {
        let id = "test_upd_stale_hook_waiting";
        let path = write_hook_status_for(id, "waiting");
        set_mtime_seconds_ago(&path, 3600);
        let read = crate::hooks::read_hook_status_with_freshness(id).expect("file present");
        assert!(!read.fresh);
        assert_eq!(read.status, Status::Waiting);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn test_update_status_missing_hook_file_is_absent() {
        // With no hook file at all, the reader reports None so the poller
        // will proceed to title fast-path / content detection unchanged.
        assert!(
            crate::hooks::read_hook_status_with_freshness("test_upd_missing_hook_file").is_none()
        );
    }

    // --- detected_inner_agent tests ----------------------------------------
    //
    // These cover the shell-session-in-memory inner-agent discovery field.
    // The end-to-end status dispatch path requires a live tmux session so
    // the full `update_status_with_options` is exercised in e2e tests; here
    // we focus on the pure in-process decisions that gate behavior on the
    // field's value.

    #[test]
    fn test_detected_inner_agent_default_is_none() {
        let inst = Instance::new("test", "/tmp/test");
        assert!(inst.detected_inner_agent.is_none());
    }

    /// Simulates the attach-return path's normalization logic. The real
    /// writer in `src/tui/app.rs::attach_session` uses the same rule:
    /// `Some("shell") | None` → `None`, `Some(x)` → `Some(x.to_string())`.
    fn normalize_detected_agent(detected: Option<&str>) -> Option<String> {
        match detected {
            Some("shell") | None => None,
            Some(agent) => Some(agent.to_string()),
        }
    }

    #[test]
    fn test_detected_inner_agent_normalization_clears_on_shell_or_none() {
        assert_eq!(normalize_detected_agent(Some("shell")), None);
        assert_eq!(normalize_detected_agent(None), None);
    }

    #[test]
    fn test_detected_inner_agent_normalization_stores_known_agents() {
        assert_eq!(
            normalize_detected_agent(Some("claude")),
            Some("claude".to_string())
        );
        assert_eq!(
            normalize_detected_agent(Some("codex")),
            Some("codex".to_string())
        );
    }

    #[test]
    fn test_detected_inner_agent_not_serialized() {
        // Field MUST be `#[serde(skip)]` so a round trip to disk drops it.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.detected_inner_agent = Some("claude".to_string());
        let json = serde_json::to_string(&inst).expect("serialize");
        assert!(
            !json.contains("detected_inner_agent"),
            "field leaked into serialized JSON: {json}"
        );
        let restored: Instance = serde_json::from_str(&json).expect("deserialize");
        assert!(
            restored.detected_inner_agent.is_none(),
            "deserialize must produce None, got {:?}",
            restored.detected_inner_agent
        );
    }

    /// Captures the dispatch rule used by `update_status_with_options`: when
    /// `tool == "shell"` and `detected_inner_agent = Some(X)`, content
    /// detection routes through `detect_status_from_content(_, X, _)`. This
    /// verifies the routing plus concrete running/idle fixtures for claude.
    #[test]
    fn test_detect_dispatch_uses_inner_agent_for_claude_running() {
        let inst = {
            let mut i = Instance::new("test", "/tmp/test");
            i.tool = "shell".to_string();
            i.detected_inner_agent = Some("claude".to_string());
            i
        };
        let agent = inst.detected_inner_agent.as_deref().expect("set above");
        let content = "Some output\n\u{280b} Working on task...\n";
        let status =
            crate::tmux::status_detection::detect_status_from_content(content, agent, None);
        assert_eq!(status, Status::Running);
    }

    #[test]
    fn test_detect_dispatch_uses_inner_agent_for_claude_idle() {
        let inst = {
            let mut i = Instance::new("test", "/tmp/test");
            i.tool = "shell".to_string();
            i.detected_inner_agent = Some("claude".to_string());
            i
        };
        let agent = inst.detected_inner_agent.as_deref().expect("set above");
        let content = "Done.\n\n\u{276f} \n";
        let status =
            crate::tmux::status_detection::detect_status_from_content(content, agent, None);
        assert_eq!(status, Status::Idle);
    }

    /// Isolates the post-detection status rewrite. Task 3.3 requires that
    /// a concrete `Idle` from a real agent detector (when the session has
    /// `detected_inner_agent = Some(_)`) surface as `Idle`, NOT be
    /// rewritten to `Unknown` by the shell/custom-command heuristic.
    fn apply_idle_rewrite(
        detected: Status,
        inner_agent_set: bool,
        has_custom_command: bool,
        pane_dead: bool,
        shell_stale: bool,
    ) -> Status {
        match detected {
            Status::Idle if inner_agent_set => {
                if pane_dead {
                    Status::Error
                } else {
                    Status::Idle
                }
            }
            Status::Idle if has_custom_command => {
                if pane_dead || shell_stale {
                    Status::Error
                } else {
                    Status::Unknown
                }
            }
            Status::Idle if pane_dead || shell_stale => Status::Error,
            other => other,
        }
    }

    #[test]
    fn test_idle_rewrite_preserves_idle_for_detected_agent() {
        assert_eq!(
            apply_idle_rewrite(Status::Idle, true, true, false, false),
            Status::Idle,
        );
    }

    #[test]
    fn test_idle_rewrite_dead_pane_with_detected_agent_becomes_error() {
        assert_eq!(
            apply_idle_rewrite(Status::Idle, true, true, true, false),
            Status::Error,
        );
    }

    #[test]
    fn test_idle_rewrite_shell_without_detected_agent_becomes_unknown() {
        // Current shell-session default: `has_custom_command` true, no
        // detected inner agent, alive pane, not shell-stale → Unknown.
        assert_eq!(
            apply_idle_rewrite(Status::Idle, false, true, false, false),
            Status::Unknown,
        );
    }

    #[test]
    fn test_idle_rewrite_agent_running_passes_through() {
        // Non-Idle statuses are untouched regardless of flags.
        assert_eq!(
            apply_idle_rewrite(Status::Running, true, true, false, false),
            Status::Running,
        );
        assert_eq!(
            apply_idle_rewrite(Status::Waiting, false, true, false, false),
            Status::Waiting,
        );
    }

    #[test]
    fn test_detected_inner_agent_not_mutated_by_update_status_short_circuit() {
        // The polling path must never write to `detected_inner_agent`.
        // Here we drive `update_status_with_options` down its early-exit
        // branch (Stopped status) and confirm the field is preserved.
        let mut inst = Instance::new("test_no_mutate_on_stopped", "/tmp/test");
        inst.tool = "shell".to_string();
        inst.detected_inner_agent = Some("claude".to_string());
        inst.status = Status::Stopped;
        inst.update_status_with_options(StatusUpdateOptions::default());
        assert_eq!(inst.detected_inner_agent.as_deref(), Some("claude"));
    }
    // =======================================================================
    // INDEPENDENT ACCEPTANCE (tester). Two things the author said were never
    // checked: what a real Claude actually puts on screen, and what the
    // capture command production uses returns when the text does not fit.
    // =======================================================================

    /// A real Claude 2.1.220 screen, captured from a live pane on a private
    /// tmux socket with an isolated HOME. Not synthesized.
    const REAL_LOGIN_SCREEN: &str = include_str!("testdata_real_claude_login.txt");
    const REAL_THEME_SCREEN: &str = include_str!("testdata_real_claude_theme.txt");
    const REAL_READY_SCREEN: &str = include_str!("testdata_real_claude_ready.txt");

    /// The other half of the criterion. The menu fixtures prove it rejects what
    /// it must reject, which a criterion that never fires also does. This is the
    /// screen it exists to recognize: a launched Claude past its startup
    /// screens, captured from a live pane that was never spoken to.
    #[test]
    fn at_a_real_ready_screen_reads_as_ready() {
        assert!(
            shows_claude_input_prompt(REAL_READY_SCREEN),
            "a real Claude waiting for input must read as ready; otherwise every \
             pane waits out the full deadline and the criterion is decoration"
        );
        let step = auto_confirm_step(REAL_READY_SCREEN, &[]);
        assert!(
            matches!(step, AutoConfirmStep::NoPrompt),
            "a ready screen asks nothing, got {step:?}"
        );
    }

    /// Not every menu numbers its options. Rejecting numbered entries is a
    /// statement about how the menus this code has seen happen to be drawn, and
    /// a pane settled here is a pane whose question no one answers.
    #[test]
    fn at_an_unnumbered_menu_option_is_not_an_input_prompt() {
        let screen = "Do you want to proceed?\n\
                      \u{276f} Yes\n\
                      \u{0020} No, and tell Claude what to do differently\n";
        assert!(
            !shows_claude_input_prompt(screen),
            "an unnumbered menu option must not read as Claude waiting for input"
        );
    }

    /// `shows_claude_input_prompt` is the signal that finishes a pane early:
    /// paired with `NoPrompt` it declares the startup screens behind it. A menu
    /// Claude is waiting on is the opposite of that, and Claude draws menu
    /// options with the same glyph as its input prompt.
    #[test]
    fn at_real_claude_login_menu_is_not_an_input_prompt() {
        assert!(
            !shows_claude_input_prompt(REAL_LOGIN_SCREEN),
            "a real login-method menu must not read as Claude waiting for input; \
             paired with NoPrompt it settles the pane while a menu is still up"
        );
    }

    #[test]
    fn at_real_claude_theme_menu_is_not_an_input_prompt() {
        assert!(
            !shows_claude_input_prompt(REAL_THEME_SCREEN),
            "a real theme-picker screen must not read as Claude waiting for input"
        );
    }

    /// The composite the production loop actually evaluates.
    #[test]
    fn at_a_menu_screen_does_not_settle_the_pane() {
        for (name, screen) in [("login", REAL_LOGIN_SCREEN), ("theme", REAL_THEME_SCREEN)] {
            let step = auto_confirm_step(screen, &[]);
            let settles =
                matches!(step, AutoConfirmStep::NoPrompt) && shows_claude_input_prompt(screen);
            assert!(
                !settles,
                "the {name} screen settles the pane: Claude is still on a menu, so \
                 'startup screens are behind it' is not true yet"
            );
        }
    }

    /// The marker table is matched against the pane's screen as tmux returns it.
    /// `capture-pane` without `-J` returns one string per screen row, so a line
    /// too long for the pane is returned already broken -- and a marker broken
    /// mid-string is not `contains`ed. Built from a real pane, not a synthesized
    /// string, because the wrap is the thing under test.
    #[test]
    #[serial_test::serial]
    fn at_a_narrow_pane_still_recognizes_the_dev_channels_prompt() {
        if crate::tmux::tmux_command()
            .arg("-V")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("Skipping test: tmux not available");
            return;
        }
        crate::tmux::isolate_tmux_socket();

        let session = format!("aoe_test_narrow_confirm_{}", std::process::id());
        // The option line as the author's own report quotes it, in a pane the
        // width of one half of a split on a laptop.
        let line = "  \u{276f} 1. I am using this for local development";
        let created = crate::tmux::tmux_command()
            .args([
                "new-session",
                "-d",
                "-s",
                &session,
                "-x",
                "40",
                "-y",
                "12",
                &format!("sh -c 'printf \"{line}\\n\"; while :; do sleep 60; done'"),
            ])
            .output()
            .expect("tmux new-session");
        assert!(created.status.success());
        struct Guard(String);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = crate::tmux::tmux_command()
                    .args(["kill-session", "-t", &self.0])
                    .output();
            }
        }
        let _guard = Guard(session.clone());
        std::thread::sleep(Duration::from_millis(600));

        let pane = String::from_utf8_lossy(
            &crate::tmux::tmux_command()
                .args(["display-message", "-t", &session, "-p", "#{pane_id}"])
                .output()
                .expect("display-message")
                .stdout,
        )
        .trim()
        .to_string();

        let screen = crate::tmux::capture_pane_screen(&pane).expect("capture");
        assert!(
            screen.contains("I am using this for"),
            "precondition: the pane really is showing the prompt line, got {screen:?}"
        );
        let step = auto_confirm_step(&screen, &[]);

        // The impact, stated as the loop states it: `NoPrompt` plus a screen that
        // reads as ready settles the pane. On this screen that means the pane is
        // declared finished with an unanswered question still on it.
        assert!(
            !(matches!(step, AutoConfirmStep::NoPrompt) && shows_claude_input_prompt(&screen)),
            "the pane is settled while its question is unanswered: the marker did \
             not survive the pane width, and the question's own option glyph then \
             read as Claude waiting for input. screen={screen:?}"
        );
        assert_eq!(
            step,
            AutoConfirmStep::Answer(AutoConfirmPrompt::DevelopmentChannels),
            "the prompt is on screen, so it must be answered; a pane too narrow to \
             fit the marker on one row must not read as 'no question here'. \
             screen={screen:?}"
        );
    }
}
