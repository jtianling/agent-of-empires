//! Session instance definition and operations

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::containers::{self, ContainerRuntimeInterface, DockerContainer};
use crate::tmux;

use super::container_config;
use super::environment::{build_docker_env_args, shell_escape};
use super::PaneConfig;

pub(crate) trait PaneConfigTarget {
    fn resolve_for(self, instance: &Instance) -> PaneConfig;
}

#[cfg(test)]
mod pane_level_command_tests {
    use super::*;

    #[test]
    fn yolo_is_read_from_the_target_pane_only() {
        let instance = Instance::new("test", "/tmp");
        let enabled = PaneConfig::new("claude", "/tmp/left", true, false);
        let disabled = PaneConfig::new("claude", "/tmp/right", false, false);

        let enabled_command = instance
            .build_pane_command(&enabled, None, false, None)
            .unwrap();
        let disabled_command = instance
            .build_pane_command(&disabled, None, false, None)
            .unwrap();

        assert!(enabled_command.contains("--dangerously-skip-permissions"));
        assert!(!disabled_command.contains("--dangerously-skip-permissions"));
    }

    #[test]
    fn cross_agent_team_is_read_from_the_target_pane_only() {
        let mut instance = Instance::new("test", "/tmp");
        instance.cross_agent_team_channel = "test-channel".to_string();
        let enabled = PaneConfig::new("claude", "/tmp/right", false, true);
        let disabled = PaneConfig::new("claude", "/tmp/left", false, false);

        let enabled_command = instance
            .build_pane_command(&enabled, None, false, Some("right-key"))
            .unwrap();
        let disabled_command = instance
            .build_pane_command(&disabled, None, false, Some("left-key"))
            .unwrap();

        assert!(enabled_command.contains("test-channel"));
        assert!(enabled_command.contains("right-key"));
        assert!(!disabled_command.contains("test-channel"));
        assert!(!disabled_command.contains("left-key"));
    }

    #[test]
    fn secondary_identity_keys_are_independent() {
        let instance = Instance::new("test", "/tmp");
        let pane = PaneConfig::new("codex", "/tmp/right", false, true);

        let first = instance.build_extra_pane_config_command(&pane).unwrap();
        let second = instance.build_extra_pane_config_command(&pane).unwrap();

        assert!(!first.identity_key.is_empty());
        assert!(!second.identity_key.is_empty());
        assert_ne!(first.identity_key, second.identity_key);
        assert!(first.command.contains("pre-register-codex-pane"));
    }

    #[test]
    fn opencode_registry_launches_secondary_through_runtime_wrapper() {
        let instance = Instance::new("test", "/tmp");
        let pane = PaneConfig::new("opencode", "/tmp", false, false);
        let command = instance
            .build_pane_command(&pane, None, false, None)
            .unwrap();

        assert!(command.contains("__opencode-runtime"));
        assert!(command.contains("--slot 1"));
        assert!(!command.contains("opencode session list"));
    }

    #[test]
    fn an_agent_launch_requires_a_built_command() {
        let error = require_launch_command(SessionLaunch::Agent, None, "opencode").unwrap_err();
        assert!(format!("{error:#}").contains("Could not build opencode launch command"));
        assert_eq!(
            require_launch_command(SessionLaunch::Placeholder, None, "opencode",).unwrap(),
            None
        );
    }

    #[test]
    fn invalid_opencode_attach_args_cannot_degrade_to_a_shell_launch() {
        let mut instance = Instance::new("test", "/tmp");
        instance.tool = "opencode".to_string();
        instance.extra_args = "--model anthropic/test".to_string();
        let pane = PaneConfig::new("opencode", "/tmp", false, false);
        let runtime = ExactSessionRuntimeContext {
            shape: crate::agents::ExactSessionRuntime::OwnedServer,
            server_base_url: String::new(),
            slot: 0,
            generation: 1,
            native_session_id: String::new(),
            identity_key: String::new(),
        };
        let command =
            instance.build_pane_command_with_runtime(&pane, None, true, None, Some(&runtime));

        let error = require_launch_command(SessionLaunch::Agent, command, "opencode").unwrap_err();
        assert!(format!("{error:#}").contains("Could not build opencode launch command"));
    }

    #[test]
    fn same_cwd_opencode_slots_keep_exact_runtime_values() {
        let instance = Instance::new("test", "/tmp/shared");
        let pane = PaneConfig::new("opencode", "/tmp/shared", false, true);
        let left = ExactSessionRuntimeContext {
            shape: crate::agents::ExactSessionRuntime::OwnedServer,
            server_base_url: String::new(),
            slot: 0,
            generation: 4,
            native_session_id: "ses_left".to_string(),
            identity_key: "left-key".to_string(),
        };
        let right = ExactSessionRuntimeContext {
            shape: crate::agents::ExactSessionRuntime::OwnedServer,
            server_base_url: String::new(),
            slot: 1,
            generation: 9,
            native_session_id: "ses_right".to_string(),
            identity_key: "right-key".to_string(),
        };

        let left_command = instance
            .build_pane_command_with_runtime(
                &pane,
                Some("ses_left"),
                true,
                Some("left-key"),
                Some(&left),
            )
            .unwrap();
        let right_command = instance
            .build_pane_command_with_runtime(
                &pane,
                Some("ses_right"),
                false,
                Some("right-key"),
                Some(&right),
            )
            .unwrap();

        assert!(left_command.contains("--slot 0 --generation 4"));
        assert!(left_command.contains("--resume-session"));
        assert!(left_command.contains("ses_left"));
        assert!(!left_command.contains("left-key"));
        assert!(!left_command.contains("XATS_IDENTITY_KEY"));
        assert!(!left_command.contains("ses_right"));
        assert!(right_command.contains("--slot 1 --generation 9"));
        assert!(right_command.contains("--resume-session"));
        assert!(right_command.contains("ses_right"));
        assert!(!right_command.contains("right-key"));
        assert!(!right_command.contains("XATS_IDENTITY_KEY"));
        assert!(!right_command.contains("ses_left"));
    }

    fn kimi_runtime(slot: i64, session_id: &str, identity_key: &str) -> ExactSessionRuntimeContext {
        ExactSessionRuntimeContext {
            shape: crate::agents::ExactSessionRuntime::SharedServer,
            server_base_url: "http://127.0.0.1:58627".to_string(),
            slot,
            generation: 0,
            native_session_id: session_id.to_string(),
            identity_key: identity_key.to_string(),
        }
    }

    /// Two kimi panes in one directory attach to their own conversations on the
    /// one shared server, and neither command mentions the other's session.
    #[test]
    #[serial_test::serial]
    fn same_cwd_kimi_panes_attach_to_their_own_sessions() {
        std::env::set_var(crate::kimi::COMMAND_ENV, "/opt/kimi-dev/kimi");
        let instance = Instance::new("test", "/tmp/shared");
        let pane = PaneConfig::new("kimi", "/tmp/shared", false, true);
        let left = kimi_runtime(0, "session_left", "left-key");
        let right = kimi_runtime(1, "session_right", "right-key");

        let left_command = instance
            .build_kimi_pane_command(&pane, None, Some(&left), false)
            .unwrap();
        let right_command = instance
            .build_kimi_pane_command(&pane, None, Some(&right), false)
            .unwrap();
        std::env::remove_var(crate::kimi::COMMAND_ENV);

        assert!(left_command.contains("--session 'session_left'"));
        assert!(!left_command.contains("session_right"));
        assert!(right_command.contains("--session 'session_right'"));
        assert!(!right_command.contains("session_left"));
        for command in [&left_command, &right_command] {
            assert!(command.contains("KIMI_XATS_BASE_URL='http://127.0.0.1:58627'"));
            assert!(command.contains("KIMI_REMOTE='auto'"));
            assert!(command.contains("'/opt/kimi-dev/kimi'"));
        }
        assert!(left_command.contains("KIMI_XATS_SESSION_ID='session_left'"));
        assert!(right_command.contains("KIMI_XATS_SESSION_ID='session_right'"));
    }

    /// A command override names the binary of a shared-server pane; it does not
    /// hand the pane back its own conversation choice. The session, the server
    /// and the engine mode are still AoE's, so the override has to run on the
    /// kimi command path rather than replacing it -- otherwise AoE would mint a
    /// session and commit its coordinates for a TUI that attaches elsewhere.
    #[test]
    #[serial_test::serial]
    fn a_command_override_names_the_kimi_binary_without_losing_the_launch_context() {
        std::env::remove_var(crate::kimi::COMMAND_ENV);
        let mut instance = Instance::new("test", "/tmp/shared");
        instance.tool = "kimi".to_string();
        instance.command = "/opt/kimi-dev/kimi".to_string();
        let pane = PaneConfig::new("kimi", "/tmp/shared", false, true);
        let runtime = kimi_runtime(0, "session_only", "super-secret-key");

        let command = instance
            .build_pane_command_with_runtime(
                &pane,
                None,
                true,
                Some("super-secret-key"),
                Some(&runtime),
            )
            .unwrap();

        // The wrapper re-escapes the inner command, so match on the words rather
        // than on their quoting.
        assert!(command.contains("/opt/kimi-dev/kimi"), "{command}");
        assert!(command.contains("--session "), "{command}");
        assert!(command.contains("KIMI_XATS_SESSION_ID="), "{command}");
        assert!(command.contains("session_only"), "{command}");
        assert!(command.contains("KIMI_XATS_BASE_URL="), "{command}");
        assert!(command.contains("127.0.0.1:58627"), "{command}");
        assert!(command.contains("KIMI_REMOTE="), "{command}");
        assert!(command.contains(&format!("-u {XATS_IDENTITY_KEY_ENV}")));
        assert!(!command.contains("super-secret-key"));
    }

    /// The key exists on the slot and is used for the xats commit, and reaches
    /// neither the command nor the environment the pane can read. A kimi tool
    /// process inherits the shared server's environment, so a key in any pane
    /// would be readable by every kimi agent on the machine.
    #[test]
    #[serial_test::serial]
    fn a_kimi_pane_never_carries_the_identity_key() {
        std::env::set_var(crate::kimi::COMMAND_ENV, "/opt/kimi-dev/kimi");
        let instance = Instance::new("test", "/tmp/shared");
        let pane = PaneConfig::new("kimi", "/tmp/shared", true, true);
        let runtime = kimi_runtime(0, "session_only", "super-secret-key");

        let command = instance
            .build_pane_command_with_runtime(
                &pane,
                None,
                true,
                Some("super-secret-key"),
                Some(&runtime),
            )
            .unwrap();
        std::env::remove_var(crate::kimi::COMMAND_ENV);

        assert!(!command.contains("super-secret-key"));
        assert!(!command.contains(&format!("{XATS_IDENTITY_KEY_ENV}=")));
        // Inherited values cannot survive either: every injected name, and the
        // key itself, is removed before anything is set.
        assert!(command.contains(&format!("-u {XATS_IDENTITY_KEY_ENV}")));
        assert!(command.contains("-u KIMI_XATS_SESSION_ID"));
        assert!(command.contains("-u KIMI_XATS_BASE_URL"));
        assert!(command.contains("-u KIMI_REMOTE"));
        assert!(command.contains("--yolo"));
        // Nothing registers with xats on the pane's behalf: the commit already
        // happened, and no bootstrap call follows it into the command.
        assert!(!command.contains("register"));
        assert!(!command.contains("reconnect"));
    }

    /// `Shift+R` attaches to the stored conversation and reports a real resume;
    /// `Shift+C` carries the freshly minted one. Both keep the slot's key, and
    /// neither command exposes it.
    #[test]
    #[serial_test::serial]
    fn kimi_resume_and_fresh_differ_only_in_the_session_they_attach() {
        std::env::set_var(crate::kimi::COMMAND_ENV, "/opt/kimi-dev/kimi");
        let instance = Instance::new("test", "/tmp/shared");
        let pane = PaneConfig::new("kimi", "/tmp/shared", false, true);

        let resumed_runtime = kimi_runtime(0, "session_old", "stable-key");
        let (resumed_command, resumed) = instance
            .build_pane_resume_plan_with_runtime(
                &pane,
                "session_old",
                true,
                RestartMode::Resume,
                Some("stable-key"),
                Some(&resumed_runtime),
            )
            .unwrap();

        let fresh_runtime = kimi_runtime(0, "session_new", "stable-key");
        let (fresh_command, fresh_resumed) = instance
            .build_pane_resume_plan_with_runtime(
                &pane,
                "session_new",
                true,
                RestartMode::Fresh,
                Some("stable-key"),
                Some(&fresh_runtime),
            )
            .unwrap();
        std::env::remove_var(crate::kimi::COMMAND_ENV);

        assert!(resumed);
        assert!(resumed_command.contains("session_old"));
        assert!(!resumed_command.contains("session_new"));
        assert!(!fresh_resumed);
        assert!(fresh_command.contains("session_new"));
        assert!(!fresh_command.contains("session_old"));
        for command in [&resumed_command, &fresh_command] {
            assert!(!command.contains("stable-key"));
        }
    }

    /// A kimi pane whose conversation was never prepared has no command at all,
    /// and a sandboxed one is refused rather than turned into a shell.
    #[test]
    fn kimi_without_preparation_or_on_a_sandbox_builds_nothing() {
        let instance = Instance::new("test", "/tmp/shared");
        let pane = PaneConfig::new("kimi", "/tmp/shared", false, true);
        assert_eq!(instance.build_pane_command(&pane, None, true, None), None);

        let mut sandboxed = Instance::new("test", "/tmp/shared");
        sandboxed.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        let runtime = kimi_runtime(0, "session_only", "");
        assert_eq!(
            sandboxed.build_pane_command_with_runtime(&pane, None, true, None, Some(&runtime)),
            None
        );
    }

    #[test]
    fn fresh_resume_plan_keeps_target_pane_flags() {
        let instance = Instance::new("test", "/tmp");
        let pane = PaneConfig::new("claude", "/tmp/right", true, true);

        let (command, resumed) = instance
            .build_pane_resume_plan(&pane, "", false, RestartMode::Fresh, Some("right-key"))
            .unwrap();

        assert!(!resumed);
        assert!(command.contains("--dangerously-skip-permissions"));
        assert!(command.contains("right-key"));
    }

    #[test]
    fn pane_flag_matrix_does_not_leak_between_siblings() {
        let mut instance = Instance::new("test", "/tmp");
        instance.cross_agent_team_channel = "pane-channel".to_string();
        for (left_yolo, left_team, right_yolo, right_team) in [
            (true, false, false, true),
            (false, true, true, false),
            (true, true, true, true),
            (false, false, false, false),
        ] {
            let left = PaneConfig::new("claude", "/tmp/left", left_yolo, left_team);
            let right = PaneConfig::new("claude", "/tmp/right", right_yolo, right_team);
            let left_command = instance
                .build_pane_command(&left, None, true, Some("left-key"))
                .unwrap();
            let right_command = instance
                .build_pane_command(&right, None, false, Some("right-key"))
                .unwrap();

            assert_eq!(
                left_command.contains("--dangerously-skip-permissions"),
                left_yolo
            );
            assert_eq!(left_command.contains("pane-channel"), left_team);
            assert_eq!(
                right_command.contains("--dangerously-skip-permissions"),
                right_yolo
            );
            assert_eq!(right_command.contains("pane-channel"), right_team);
        }
    }
}

impl PaneConfigTarget for &PaneConfig {
    fn resolve_for(self, _instance: &Instance) -> PaneConfig {
        self.clone()
    }
}

#[cfg(test)]
impl PaneConfigTarget for &str {
    fn resolve_for(self, instance: &Instance) -> PaneConfig {
        PaneConfig::new(
            self,
            instance.project_path.clone(),
            instance.yolo_mode,
            instance.cross_agent_team && !instance.is_sandboxed(),
        )
    }
}

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

/// Every startup question auto-confirm knows how to answer. A pane that does not
/// need identity recovery can finish after answering all of them. A pane that
/// does need recovery must still wait for Claude's input prompt before AoE can
/// safely submit reconnect.
const AUTO_CONFIRM_PROMPTS: &[AutoConfirmPrompt] = &[
    AutoConfirmPrompt::DevelopmentChannels,
    AutoConfirmPrompt::WorkspaceTrust,
];

fn settles_after_answer(reclaims_identity: bool, answered: &[AutoConfirmPrompt]) -> bool {
    !reclaims_identity
        && AUTO_CONFIRM_PROMPTS
            .iter()
            .all(|known| answered.contains(known))
}

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

/// Where the xats identity key a launch injects into a pane came from.
///
/// Only a key that predates the launch can name an identity to reclaim. One
/// minted for this launch has no history behind it, and a pane running without a
/// key has nothing to reclaim with -- so the three cases are kept apart rather
/// than collapsed into "has a key".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKeyOrigin {
    /// The pane carries no key: Cross Agent Team is off, the session is
    /// sandboxed, or the key could not be persisted.
    Absent,
    /// This launch minted the key.
    Minted,
    /// The key predates this launch.
    Existing,
}

impl IdentityKeyOrigin {
    /// Classify a key this launch did not mint.
    fn of_existing_key(key: &str) -> Self {
        if key.is_empty() {
            Self::Absent
        } else {
            Self::Existing
        }
    }

    /// Whether the pane may still answer to an xats identity established before
    /// this launch.
    fn reclaims_identity(self) -> bool {
        matches!(self, Self::Existing)
    }
}

/// One slot's key origin, defaulting to absent for a slot the caller did not
/// report on. Not knowing is not evidence that the pane owns an identity.
fn slot_identity_origin(origins: &HashMap<i64, IdentityKeyOrigin>, slot: i64) -> IdentityKeyOrigin {
    origins
        .get(&slot)
        .copied()
        .unwrap_or(IdentityKeyOrigin::Absent)
}

/// A pane a launch just started, paired with whether it may reclaim an xats
/// identity once Claude is accepting input.
///
/// The flag travels with the pane because it is only knowable at launch time,
/// and it is per pane because one relaunch can mint a key for one pane while
/// reusing another's.
pub(crate) struct LaunchedClaudePane {
    pane: String,
    reclaims_identity: bool,
}

/// What a pane is asked, verbatim, to reclaim its xats identity. The word is the
/// xats reconnect tool's own trigger, and what a user types by hand today.
const XATS_RECONNECT_REQUEST: &str = "reconnect";

/// Ask a relaunched Claude to reclaim the xats identity its key still names.
///
/// Submitted as a real user turn because Claude's xats binding lives inside its
/// MCP session: no launch argument can declare it from outside, and the startup
/// hint that would prompt Claude to do it arrives once and goes unanswered. The
/// key is already in the pane's environment, so the request carries no arguments
/// and AoE never learns the agent name it restores.
///
/// A failure is logged and dropped. The pane is then exactly where it is today:
/// interactive, and one manual `reconnect` away from its identity.
fn submit_xats_reconnect(pane: &str) {
    if let Err(err) = tmux::submit_text_to_pane_target(pane, XATS_RECONNECT_REQUEST) {
        tracing::warn!(
            "Could not ask pane {} to reclaim its xats identity: {}",
            pane,
            err
        );
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

/// Environment variable naming the local Codex app-server. This is xats's own
/// variable rather than an AoE one: xats resolves its Codex endpoint from it, so
/// a separate AoE name would let the two be pointed at different servers while
/// the user believed they had configured one.
const CODEX_XATS_APP_SERVER_URL_ENV: &str = "CROSS_AGENT_TEAMS_CODEX_WS_URL";
/// xats also accepts a JSON array of endpoints here and probes them for a given
/// thread. AoE has to commit to one endpoint at launch, before any thread
/// exists, so it can only follow a single-element array.
const CODEX_XATS_APP_SERVER_URLS_ENV: &str = "CROSS_AGENT_TEAMS_CODEX_WS_URLS";
const CODEX_XATS_APP_SERVER_DEFAULT_URL: &str = "ws://127.0.0.1:8799";

/// A resolved app-server endpoint. The availability gate and the `--remote`
/// argument both read from one of these so they cannot name different servers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexAppServerEndpoint {
    url: String,
    /// Host in the form `nc` wants: an IPv6 literal appears here without the
    /// brackets the URL form requires.
    host: String,
    port: u16,
}

/// Accept what xats accepts: a URL that parses, over `ws` or `wss`. A path is
/// allowed and preserved, because xats preserves it too.
///
/// Matching xats's acceptance set is the point. A value xats takes and AoE
/// refuses would put the two on different servers while the user believed they
/// had configured one -- the same silent split this whole resolution exists to
/// remove, just entering through a different door.
///
/// The URL is kept as written rather than as the parser re-serializes it: a
/// round trip through `Url` appends a trailing slash to an authority-only URL,
/// which would silently change the `--remote` argument AoE has always passed.
///
/// The host still has to survive a character check, because it is interpolated
/// into a generated `sh -c` script. That is an injection guard, and it is
/// deliberately separate from the question of what endpoints are acceptable.
fn parse_codex_app_server_url(raw: &str) -> Option<CodexAppServerEndpoint> {
    let raw = raw.trim();
    let parsed = url::Url::parse(raw).ok()?;
    if !matches!(parsed.scheme(), "ws" | "wss") {
        return None;
    }

    let host = parsed.host_str()?;
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
    {
        return None;
    }

    Some(CodexAppServerEndpoint {
        url: raw.to_string(),
        host: host.to_string(),
        port: parsed.port_or_known_default()?,
    })
}

/// Resolve the endpoint, or return the diagnostic explaining why not.
///
/// There is no falling back to the default on a bad value. A user who
/// configured something AoE will not take must find out from the pane, not from
/// a line in AoE's debug log: the symptom of a silent fallback shows up on the
/// xats side, as a Codex that connected but cannot be resumed, and nobody
/// debugging that goes looking through AoE's warnings. A pane that refuses to
/// start is loud; two systems quietly talking to different servers is not.
fn codex_app_server_endpoint() -> Result<CodexAppServerEndpoint, String> {
    resolve_codex_app_server_endpoint(
        std::env::var(CODEX_XATS_APP_SERVER_URL_ENV).ok(),
        std::env::var(CODEX_XATS_APP_SERVER_URLS_ENV).ok(),
    )
}

/// Takes the two variables' values rather than reading them, so tests never
/// mutate the process environment. Setting it there would also change what every
/// concurrently running test that builds a Codex command sees.
fn resolve_codex_app_server_endpoint(
    single: Option<String>,
    list: Option<String>,
) -> Result<CodexAppServerEndpoint, String> {
    if let Some(raw) = single {
        return parse_codex_app_server_url(&raw).ok_or_else(|| {
            format!("[xats] {CODEX_XATS_APP_SERVER_URL_ENV} is not a ws:// or wss:// URL: {raw}")
        });
    }

    if let Some(raw) = list {
        return codex_app_server_endpoint_from_list(&raw);
    }

    Ok(
        parse_codex_app_server_url(CODEX_XATS_APP_SERVER_DEFAULT_URL)
            .expect("the default Codex app-server URL parses"),
    )
}

/// xats probes every endpoint in this list to find the one holding a thread.
/// AoE picks its endpoint before any thread exists, so it can only follow a list
/// that leaves nothing to pick.
fn codex_app_server_endpoint_from_list(raw: &str) -> Result<CodexAppServerEndpoint, String> {
    let ambiguous = || {
        format!(
            "[xats] {CODEX_XATS_APP_SERVER_URLS_ENV} must hold exactly one endpoint for AoE \
             to launch against, or set {CODEX_XATS_APP_SERVER_URL_ENV} instead: {raw}"
        )
    };

    let entries: Vec<String> = serde_json::from_str(raw).map_err(|_| ambiguous())?;
    let [only] = entries.as_slice() else {
        return Err(ambiguous());
    };

    parse_codex_app_server_url(only).ok_or_else(|| {
        format!("[xats] {CODEX_XATS_APP_SERVER_URLS_ENV} is not a ws:// or wss:// URL: {only}")
    })
}
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
/// TTL for the Codex pane pre-registration row, in seconds. The daemon's
/// documented ceiling; its 120s default TTL can expire before a Codex cold
/// start finishes, closing the poke-back window the identity recovery needs.
const CODEX_XATS_PREREGISTER_TTL_SECONDS: u32 = 600;

const CODEX_XATS_MISSING_PANE: &str = "[xats] Missing TMUX_PANE for Codex pre-registration.";
const CODEX_XATS_MISSING_UUIDGEN: &str =
    "[xats] Missing uuidgen required for Codex pre-registration.";
const CODEX_XATS_MISSING_NC: &str = "[xats] Missing nc required to check the Codex app-server.";
const CODEX_XATS_MISSING_NPX: &str = "[xats] Missing npx required for Codex pre-registration.";
const CODEX_XATS_INVALID_UUID: &str = "[xats] uuidgen returned an invalid Codex agent UUID.";
/// Terminal: a failed pre-registration is never retried without the pane's
/// identity key. The key is the only thing by which the daemon recognizes which
/// identity a pane belongs to, so a keyless registration produces a pane that
/// looks healthy and is never prompted to re-register -- and neither observed
/// failure (npx cannot resolve the package; the daemon refuses to displace a
/// live keyed row) would clear on an immediate second attempt anyway.
const CODEX_XATS_PREREGISTER_FAILED: &str = "[xats] Failed to pre-register the Codex pane.";
fn codex_xats_app_server_unavailable(url: &str) -> String {
    format!("[xats] Codex app-server is not listening on {url}.")
}

/// A pane command that reports why the bootstrap could not be built and exits
/// non-zero, which is how every other Cross Agent Team precondition in this file
/// already fails.
fn codex_xats_aborted_command(diagnostic: &str) -> String {
    let script = format!("printf '%s\\n' {} >&2; exit 1", shell_escape(diagnostic));
    format!("sh -c {}", shell_escape(&script))
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub branch: String,
    pub main_repo_path: String,
    pub managed_by_aoe: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub cleanup_on_delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub primary_pane: PaneConfig,
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
            primary_pane: PaneConfig::new("claude", project_path, false, false),
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
        self.primary_pane.yolo_mode
    }

    pub fn supports_cross_agent_team_tool(tool: &str) -> bool {
        crate::agents::supports_cross_agent_team(tool)
    }

    /// The exact-session runtime AoE prepares for `pane`, if the registry gives
    /// it one. A sandboxed instance has none: AoE prepares a conversation from
    /// the host, and nothing it prepares there describes what runs in the
    /// container.
    fn pane_exact_session_runtime(
        &self,
        pane: &PaneConfig,
    ) -> Option<crate::agents::ExactSessionRuntime> {
        if self.is_sandboxed() {
            return None;
        }
        crate::agents::exact_session_runtime(&pane.tool)
    }

    /// Whether this instance launches with tool-specific Cross Agent Team behavior.
    pub fn is_cross_agent_team(&self) -> bool {
        self.primary_pane.cross_agent_team
            && !self.is_sandboxed()
            && Self::supports_cross_agent_team_tool(&self.primary_pane.tool)
    }

    pub fn primary_pane_config(&self) -> &PaneConfig {
        &self.primary_pane
    }

    fn legacy_primary_pane_config(&self) -> PaneConfig {
        let mut pane = PaneConfig::new(
            self.tool.clone(),
            self.project_path.clone(),
            self.yolo_mode,
            self.cross_agent_team,
        );
        if self.worktree_info.is_some() || self.workspace_info.is_some() {
            pane.worktree = Some(super::PaneWorktreeInfo {
                worktree_path: self
                    .worktree_info
                    .as_ref()
                    .map(|_| self.project_path.clone()),
                worktree: self.worktree_info.clone(),
                workspace: self.workspace_info.clone(),
            });
        }
        pane
    }

    pub fn hydrate_legacy_primary_pane(&mut self) {
        if self.primary_pane.tool.is_empty() || self.primary_pane.working_dir.is_empty() {
            self.primary_pane = self.legacy_primary_pane_config();
        } else {
            self.primary_pane = std::mem::take(&mut self.primary_pane).normalized();
        }
        self.sync_legacy_primary_fields();
    }

    pub fn set_primary_pane_config(&mut self, config: PaneConfig) {
        self.primary_pane = config.normalized();
        self.sync_legacy_primary_fields();
    }

    pub fn sync_primary_pane_from_legacy(&mut self) {
        self.primary_pane = self.legacy_primary_pane_config();
        self.sync_legacy_primary_fields();
    }

    fn sync_legacy_primary_fields(&mut self) {
        self.tool = self.primary_pane.tool.clone();
        self.project_path = self.primary_pane.working_dir.clone();
        self.yolo_mode = self.primary_pane.yolo_mode;
        self.cross_agent_team = self.primary_pane.cross_agent_team;
        self.worktree_info = self.primary_pane.worktree_info().cloned();
        self.workspace_info = self.primary_pane.workspace_info().cloned();
    }

    /// Mint this instance's primary-pane xats identity key if Cross Agent Team is
    /// enabled and it has none yet. Write-once: every later launch reuses it, which
    /// is what lets a launch that discards the conversation keep the identity.
    ///
    /// Reports where the key the launch will inject came from, which is the only
    /// moment that distinction exists: afterwards the key is simply present, and
    /// nothing on it says whether this launch wrote it.
    fn ensure_xats_identity_key(&mut self) -> IdentityKeyOrigin {
        if !self.is_cross_agent_team() {
            return IdentityKeyOrigin::Absent;
        }
        match self.xats_identity_key.as_deref() {
            None => {
                self.xats_identity_key = Some(Uuid::new_v4().to_string());
                IdentityKeyOrigin::Minted
            }
            Some(key) => IdentityKeyOrigin::of_existing_key(key),
        }
    }

    /// Whether AoE should mint an identity key for this slot before launching it.
    fn slot_needs_identity_key(&self, slot: &crate::db::AgentSlot) -> bool {
        !self.is_sandboxed()
            && slot.cross_agent_team
            && Self::supports_cross_agent_team_tool(&slot.agent)
            && slot.xats_identity_key.is_empty()
    }

    /// Mint and persist an identity key for every adopted slot that has none, so
    /// panes AoE is about to launch carry one.
    ///
    /// This is where a hand-started pane first gets a key: adoption is
    /// observe-first, so AoE never built that pane's original command and could
    /// not have injected one earlier.
    ///
    /// Reports each slot's key origin, keyed by slot index rather than by
    /// position: callers reorder the slots before launching them.
    pub fn ensure_slot_identity_keys(
        &self,
        store: &crate::db::Store,
        slots: &mut [crate::db::AgentSlot],
    ) -> HashMap<i64, IdentityKeyOrigin> {
        slots
            .iter_mut()
            .map(|slot| {
                let origin = if self.slot_needs_identity_key(slot) {
                    Self::mint_slot_identity_key(store, slot, &self.title)
                } else {
                    IdentityKeyOrigin::of_existing_key(&slot.xats_identity_key)
                };
                (slot.slot, origin)
            })
            .collect()
    }

    /// Mint and persist one slot's key.
    ///
    /// A key that could not be persisted is reported as absent, not as minted: it
    /// will not be there next launch, so nothing may be built on it having been
    /// this pane's. The slot keeps its empty key and the next launch mints again.
    fn mint_slot_identity_key(
        store: &crate::db::Store,
        slot: &mut crate::db::AgentSlot,
        title: &str,
    ) -> IdentityKeyOrigin {
        let key = Uuid::new_v4().to_string();
        match store.upsert_agent_slot_config(
            &slot.instance_id,
            slot.slot,
            &slot.pane_config(),
            &slot.native_session_id,
            &slot.tmux_pane,
            &key,
            slot.last_seen_at,
        ) {
            Ok(()) => {
                slot.xats_identity_key = key;
                IdentityKeyOrigin::Minted
            }
            Err(e) => {
                tracing::warn!(
                    "Could not persist xats identity key for slot {} of '{}': {}",
                    slot.slot,
                    title,
                    e
                );
                IdentityKeyOrigin::Absent
            }
        }
    }

    /// The identity key to inject into a pane being launched. Durable slot state
    /// wins; the instance value is only the bootstrap source before slot 0 exists.
    fn xats_identity_key_for_pane<'a>(
        &'a self,
        pane: &PaneConfig,
        is_primary: bool,
        slot_identity_key: Option<&'a str>,
    ) -> Option<&'a str> {
        if !pane.cross_agent_team || !Self::supports_cross_agent_team_tool(&pane.tool) {
            return None;
        }
        let key = slot_identity_key.or_else(|| {
            is_primary
                .then_some(self.xats_identity_key.as_deref())
                .flatten()
        });
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
    fn agent_pane_has_claude_prompts(&self, target: impl PaneConfigTarget) -> bool {
        let pane = target.resolve_for(self);
        !self.is_sandboxed() && pane.tool == "claude" && pane.cross_agent_team
    }

    /// Whether a pane this launch started should be asked to reclaim an xats
    /// identity once it is ready.
    ///
    /// Same gate as the startup screens: only a non-sandboxed Claude pane in
    /// Cross Agent Team mode has an xats identity at all. On top of that, only a
    /// key the launch reused can name one -- a pane whose key was minted here is
    /// launching for the first time, and asking it to reclaim would either find
    /// nothing or take a name the user has not chosen yet.
    fn reclaims_xats_identity(&self, pane: &PaneConfig, origin: IdentityKeyOrigin) -> bool {
        self.agent_pane_has_claude_prompts(pane) && origin.reclaims_identity()
    }

    fn launched_claude_pane(
        &self,
        pane_id: String,
        config: &PaneConfig,
        origin: IdentityKeyOrigin,
    ) -> LaunchedClaudePane {
        LaunchedClaudePane {
            reclaims_identity: self.reclaims_xats_identity(config, origin),
            pane: pane_id,
        }
    }

    fn run_auto_confirm(&self, target: impl PaneConfigTarget, origin: IdentityKeyOrigin) {
        let pane = target.resolve_for(self);
        if !self.agent_pane_has_claude_prompts(&pane) {
            return;
        }
        let session_name = tmux::Session::generate_name(&self.id, &self.title);
        let Some(agent_pane) = tmux::get_agent_pane_id(&session_name) else {
            return;
        };
        self.auto_confirm_panes(&[self.launched_claude_pane(agent_pane, &pane, origin)]);
    }

    pub fn auto_confirm_launched_pane(&self, pane_id: &str, pane: &PaneConfig) {
        if !self.agent_pane_has_claude_prompts(pane) {
            return;
        }
        // This entry point serves a pane being added, whose key was minted for
        // it moments ago (see `build_extra_pane_command`). There is no earlier
        // identity behind it, and the one a sibling holds is not this pane's to
        // reclaim.
        self.auto_confirm_panes(&[self.launched_claude_pane(
            pane_id.to_string(),
            pane,
            IdentityKeyOrigin::Minted,
        )]);
    }

    #[cfg(test)]
    fn cross_agent_team_pane(&self, agent: &str) -> bool {
        self.cross_agent_team && !self.is_sandboxed() && Self::supports_cross_agent_team_tool(agent)
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
    ///
    /// The readiness signal is also what gates the xats reconnect request: a pane
    /// that carried its key in from a previous launch is asked to reclaim its
    /// identity there, and only there. Having run out of known questions does not
    /// stand in for it -- that says only that AoE has nothing left to ask, while
    /// Claude may still be starting, and a request submitted then is delivered to
    /// whatever the pane is doing instead. A pane that never becomes ready is
    /// left alone, as it is today.
    fn auto_confirm_panes(&self, panes: &[LaunchedClaudePane]) {
        if panes.is_empty() {
            return;
        }

        struct PaneConfirm<'a> {
            pane: &'a str,
            reclaims_identity: bool,
            answered: Vec<AutoConfirmPrompt>,
            settled: bool,
        }

        let mut tracked: Vec<PaneConfirm> = panes
            .iter()
            .map(|entry| PaneConfirm {
                pane: entry.pane.as_str(),
                reclaims_identity: entry.reclaims_identity,
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
                    if entry.reclaims_identity {
                        // Settled panes are skipped from here on, so this runs
                        // at most once per pane.
                        submit_xats_reconnect(entry.pane);
                    }
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
                        entry.settled =
                            settles_after_answer(entry.reclaims_identity, &entry.answered);
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
    fn claude_cross_agent_team_flag(&self, pane: &PaneConfig) -> Option<String> {
        if !pane.cross_agent_team {
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
    /// The instance's command override describes the instance's own agent pane
    /// and nothing else, so a pane running a different agent -- or a second
    /// pane running the same one -- starts from that agent's own binary.
    /// Reading the override for such a pane produces a command that launches
    /// the instance's own program in a pane that is not it, which for Codex
    /// means bootstrapping the override under a second conversation.
    fn pane_base_command(&self, target_agent: &str, is_primary: bool) -> String {
        if is_primary && self.pane_runs_instance_tool(target_agent) {
            self.get_tool_command().to_string()
        } else {
            crate::agents::get_agent(target_agent)
                .map(|a| a.binary)
                .unwrap_or(target_agent)
                .to_string()
        }
    }

    fn codex_xats_bootstrap_command(&self, cmd: &str, base: &str, working_dir: &str) -> String {
        match codex_app_server_endpoint() {
            Ok(endpoint) => {
                self.codex_xats_bootstrap_command_for(cmd, base, working_dir, &endpoint)
            }
            Err(diagnostic) => {
                tracing::warn!("{}", diagnostic);
                codex_xats_aborted_command(&diagnostic)
            }
        }
    }

    /// Takes the endpoint rather than resolving it, so tests can assert on a
    /// specific one without setting a process-global environment variable that
    /// every other test building a Codex command would also see.
    ///
    /// The `exec` in front of the Codex command is part of the contract with
    /// xats, not a way to save a process. This script runs as a non-interactive
    /// `sh -c`, so there is no job control and children stay in the shell's own
    /// process group. `exec` makes Codex replace that shell, so it inherits its
    /// pid -- which is the pane's process group leader. xats identifies a pane's
    /// carrier by matching Codex's argv and then folding the npm shim and its
    /// native child together by taking the group leader; with `exec` the leader
    /// is one of the matches, and folding succeeds. Wrap the command instead of
    /// exec-ing it and the leader becomes `sh`, which matches nothing, so xats
    /// finds no leader among the matches and never binds the pane. It reports
    /// nothing: the pane registers, and delivery silently never arrives.
    fn codex_xats_bootstrap_command_for(
        &self,
        cmd: &str,
        base: &str,
        working_dir: &str,
        endpoint: &CodexAppServerEndpoint,
    ) -> String {
        let suffix = cmd.strip_prefix(base).unwrap_or_default();
        let working_dir = shell_escape(working_dir);
        let app_server_url = shell_escape(&endpoint.url);
        let codex_command = format!(
            "{base} --remote {app_server_url} -C {working_dir} \
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
             pre_register_failed=; \
             if [ -n \"${{{identity_env}:-}}\" ]; then \
                 npx --no-install {package} pre-register-codex-pane \
                     --pane \"$TMUX_PANE\" --agent-id \"$xats_agent_id\" \
                     --identity-key-env {identity_env} --ttl {ttl} \
                     || pre_register_failed=1; \
             else \
                 npx --no-install {package} pre-register-codex-pane \
                     --pane \"$TMUX_PANE\" --agent-id \"$xats_agent_id\" \
                     --ttl {ttl} \
                     || pre_register_failed=1; \
             fi; \
             if [ -n \"${{pre_register_failed:-}}\" ]; then \
                 printf '%s\\n' '{prereg_failed}' >&2; \
                 exit 1; \
             fi; \
             exec {codex_command}",
            host = shell_escape(&endpoint.host),
            port = endpoint.port,
            package = CODEX_XATS_PACKAGE,
            identity_env = XATS_IDENTITY_KEY_ENV,
            ttl = CODEX_XATS_PREREGISTER_TTL_SECONDS,
            missing_pane = CODEX_XATS_MISSING_PANE,
            missing_uuidgen = CODEX_XATS_MISSING_UUIDGEN,
            missing_nc = CODEX_XATS_MISSING_NC,
            missing_npx = CODEX_XATS_MISSING_NPX,
            invalid_uuid = CODEX_XATS_INVALID_UUID,
            prereg_failed = CODEX_XATS_PREREGISTER_FAILED,
            app_server_unavailable = codex_xats_app_server_unavailable(&endpoint.url),
        );
        format!("sh -c {}", shell_escape(&script))
    }

    fn has_custom_command(&self) -> bool {
        if !self.extra_args.is_empty() {
            return true;
        }
        self.has_command_override()
    }

    pub fn has_command_override(&self) -> bool {
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
        let identity_origin = self.ensure_xats_identity_key();
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

        let exact_runtime = match self
            .pane_exact_session_runtime(&self.primary_pane)
            .filter(|_| launch == SessionLaunch::Agent)
        {
            Some(shape) => {
                self.reject_command_override_for(shape, self.has_command_override())?;
                let store = crate::db::Store::open_with_schema(&Self::current_profile())?;
                Some(self.prepare_exact_session_runtime(
                    &store,
                    0,
                    &self.primary_pane,
                    "",
                    self.xats_identity_key.as_deref().unwrap_or(""),
                    RestartMode::Fresh,
                )?)
            }
            None => None,
        };
        let cmd = match launch {
            SessionLaunch::Agent => match exact_runtime.as_ref() {
                Some(runtime) => self.build_pane_command_with_runtime(
                    &self.primary_pane,
                    None,
                    true,
                    Some(runtime.identity_key.as_str()).filter(|key| !key.is_empty()),
                    Some(runtime),
                ),
                None => self.build_agent_command(None),
            },
            SessionLaunch::Placeholder => None,
        };
        let cmd = require_launch_command(launch, cmd, &self.primary_pane.tool)?;
        tracing::debug!(
            "agent cmd: {}",
            cmd.as_ref().map_or_else(
                || "none".to_string(),
                |v| crate::tmux::redact_identity_key(v)
            )
        );
        session.create_with_size(
            &self.project_path,
            cmd.as_deref(),
            size,
            !self.expects_shell(),
        )?;

        if launch == SessionLaunch::Agent {
            if let Err(error) = self.record_primary_launch_slot(exact_runtime.as_ref()) {
                let _ = session.kill();
                return Err(error);
            }
            // The pane was just created running this instance's tool, so there
            // is nothing to read back off it.
            self.run_auto_confirm(&self.primary_pane, identity_origin);
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

    fn record_primary_launch_slot(
        &self,
        exact_runtime: Option<&ExactSessionRuntimeContext>,
    ) -> Result<()> {
        let profile = Self::current_profile();
        let session_name = tmux::Session::generate_name(&self.id, &self.title);
        let pane_id = tmux::get_agent_pane_id(&session_name)
            .ok_or_else(|| anyhow::anyhow!("primary pane id was not recorded"))?;
        let store = crate::db::Store::open_with_schema(&profile)?;
        if let Some(runtime) = exact_runtime {
            runtime.bind_launched_pane(&store, &self.id, &self.primary_pane, &pane_id)
        } else {
            store.record_launched_slot_config_if_absent(
                &self.id,
                0,
                &self.primary_pane,
                &pane_id,
                self.xats_identity_key.as_deref().unwrap_or(""),
                crate::db::now_unix(),
            )
        }
    }

    /// Build the agent launch command string. Pure command construction with no
    /// side effects (no hooks, no container lifecycle management).
    ///
    /// Delegates to [`build_pane_command`](Self::build_pane_command) for the
    /// primary agent (`self.tool`, `is_primary = true`) so that the single-pane
    /// start/respawn path and the slot-based multi-pane resume path share one
    /// launch-context decoration pipeline.
    pub fn build_agent_command(&self, resume_token: Option<&str>) -> Option<String> {
        self.build_pane_command(&self.primary_pane, resume_token, true, None)
    }

    /// Build the launch command for an extra agent pane AoE adds beside the
    /// instance's own agent pane: the new session dialog's right pane, or
    /// `aoe session add-agent-pane`.
    ///
    /// Built as a non-primary pane, which is what keeps the instance's own
    /// launch context out of it. The command override, pre-allocated session
    /// id, fork token and identity key each describe one pane; a second live
    /// pane wearing them would share that pane's conversation and identity, and
    /// two panes behind one identity is the state the identity design cannot
    /// recover from. Everything that follows from the session's settings --
    /// sandboxing, YOLO, Cross Agent Team decoration -- applies exactly as it
    /// does when AoE relaunches a tracked pane, because this is that builder.
    ///
    /// `shell` is built here rather than through it: the registry entry names
    /// no launchable binary, and a shell pane produces no capture.
    ///
    /// The identity key is minted here rather than supplied by the caller. AoE
    /// builds this pane's command and so is the only party that can bind an
    /// identity to it before it first registers; a pane that reaches the daemon
    /// keyless is what the daemon's seat matching treats as claimable. The key
    /// is freshly minted and never the instance's own: two live panes behind one
    /// identity is the state the recovery design cannot resolve, and launch is
    /// the only moment at which it is preventable.
    /// `cwd` is the directory the pane will be split into. Only a shell pane
    /// needs it in the command itself; an agent pane inherits it from the split.
    pub fn build_extra_pane_config_command(&self, pane: &PaneConfig) -> Option<ExtraPaneLaunch> {
        if pane.tool == "shell" {
            return Some(ExtraPaneLaunch {
                command: self.build_extra_shell_pane_command(&pane.working_dir),
                agent: pane.tool.clone(),
                native_session_id: String::new(),
                identity_key: String::new(),
                prepared_slot: None,
                prepared_generation: None,
            });
        }
        let identity_key = if pane.cross_agent_team
            && Self::supports_cross_agent_team_tool(&pane.tool)
            && !self.is_sandboxed()
        {
            Uuid::new_v4().to_string()
        } else {
            String::new()
        };
        let command = self.build_pane_command(
            pane,
            None,
            false,
            Some(identity_key.as_str()).filter(|k| !k.is_empty()),
        )?;
        Some(ExtraPaneLaunch {
            command,
            agent: pane.tool.clone(),
            native_session_id: String::new(),
            identity_key,
            prepared_slot: None,
            prepared_generation: None,
        })
    }

    pub fn prepare_extra_pane_config_command(
        &self,
        profile: &str,
        session_name: &str,
        pane: &PaneConfig,
    ) -> Result<ExtraPaneLaunch> {
        let Some(shape) = self.pane_exact_session_runtime(pane) else {
            return self
                .build_extra_pane_config_command(pane)
                .ok_or_else(|| anyhow::anyhow!("No launch command for pane tool '{}'", pane.tool));
        };
        let store = crate::db::Store::open_with_schema(profile)?;
        let identity_key = if pane.cross_agent_team {
            Uuid::new_v4().to_string()
        } else {
            String::new()
        };
        let live_snapshot_at = crate::db::now_unix();
        let live_pane_ids = crate::db::reconcile::live_session_pane_ids(session_name)?;
        let (slot, prepared) = store.prepare_new_exact_session_slot(
            &self.id,
            pane,
            &identity_key,
            &live_pane_ids,
            live_snapshot_at,
            crate::db::now_unix(),
        )?;
        let mut runtime = ExactSessionRuntimeContext {
            shape,
            server_base_url: String::new(),
            slot,
            generation: prepared.generation,
            native_session_id: prepared.native_session_id,
            identity_key: prepared.xats_identity_key,
        };
        if let Err(error) = self.claim_new_extra_pane_runtime(&store, pane, &mut runtime) {
            let cleanup = store.rollback_unbound_exact_session_slot(
                &self.id,
                &pane.tool,
                slot,
                runtime.generation,
                &runtime.identity_key,
                &runtime.native_session_id,
            );
            return Err(append_rollback_error(error, cleanup));
        }
        let command = self.build_pane_command_with_runtime(
            pane,
            None,
            false,
            Some(runtime.identity_key.as_str()).filter(|key| !key.is_empty()),
            Some(&runtime),
        );
        let command = match command {
            Some(command) => command,
            None => {
                let error =
                    anyhow::anyhow!("Could not build {} runtime command", pane.tool.as_str());
                let cleanup = store.rollback_unbound_exact_session_slot(
                    &self.id,
                    &pane.tool,
                    slot,
                    runtime.generation,
                    &runtime.identity_key,
                    &runtime.native_session_id,
                );
                return Err(append_rollback_error(error, cleanup));
            }
        };
        Ok(ExtraPaneLaunch {
            command,
            agent: pane.tool.clone(),
            native_session_id: runtime.native_session_id,
            identity_key: runtime.identity_key,
            prepared_slot: Some(slot),
            prepared_generation: Some(runtime.generation),
        })
    }

    pub fn rollback_prepared_extra_pane(
        &self,
        profile: &str,
        launch: &ExtraPaneLaunch,
    ) -> Result<()> {
        let (Some(slot), Some(generation)) = (launch.prepared_slot, launch.prepared_generation)
        else {
            return Ok(());
        };
        crate::db::Store::open_with_schema(profile)?.rollback_unbound_exact_session_slot(
            &self.id,
            &launch.agent,
            slot,
            generation,
            &launch.identity_key,
            &launch.native_session_id,
        )
    }

    fn prepare_exact_session_runtime(
        &self,
        store: &crate::db::Store,
        slot: i64,
        pane: &PaneConfig,
        tmux_pane: &str,
        identity_key: &str,
        mode: RestartMode,
    ) -> Result<ExactSessionRuntimeContext> {
        let shape = self.pane_exact_session_runtime(pane).ok_or_else(|| {
            anyhow::anyhow!(
                "runtime preparation requires a host pane whose agent has an exact session runtime"
            )
        })?;
        // Validate user input before advancing the slot generation.
        if slot == 0 && self.pane_runs_instance_tool(&pane.tool) && !self.extra_args.is_empty() {
            validate_exact_runtime_extra_args(shape, &self.extra_args)
                .with_context(|| format!("validating managed {} launch arguments", pane.tool))?;
        }
        let now = crate::db::now_unix();
        store.record_launched_slot_config_if_absent(
            &self.id,
            slot,
            pane,
            tmux_pane,
            identity_key,
            now,
        )?;
        let existing = store
            .read_slots_for_instance(&self.id)?
            .into_iter()
            .find(|row| row.slot == slot)
            .ok_or_else(|| anyhow::anyhow!("{} slot {slot} was not provisioned", pane.tool))?;
        let identity_key = if existing.xats_identity_key.is_empty() {
            identity_key
        } else {
            &existing.xats_identity_key
        };
        store.upsert_agent_slot_config(
            &self.id,
            slot,
            pane,
            &existing.native_session_id,
            tmux_pane,
            identity_key,
            now,
        )?;
        if mode == RestartMode::Resume {
            validate_exact_session_id(shape, &existing.native_session_id).with_context(|| {
                format!("{} slot {slot} has no valid session to resume", pane.tool)
            })?;
        }
        self.serialize_shared_server_replacement(shape, mode, tmux_pane)?;
        match shape {
            crate::agents::ExactSessionRuntime::OwnedServer => {
                self.prepare_owned_server_runtime(store, slot, pane, mode)
            }
            crate::agents::ExactSessionRuntime::SharedServer => {
                let mut runtime = ExactSessionRuntimeContext {
                    shape,
                    server_base_url: String::new(),
                    slot,
                    generation: existing.xats_runtime_generation,
                    native_session_id: existing.native_session_id,
                    identity_key: identity_key.to_string(),
                };
                self.prepare_shared_server_session(store, pane, tmux_pane, &mut runtime, mode)?;
                Ok(runtime)
            }
        }
    }

    /// Advance the fence of a slot whose server AoE owns, then reserve it.
    ///
    /// The conversation itself is minted later by the runtime wrapper against
    /// the server it starts; the generation returned here is what lets that
    /// wrapper prove the slot still belongs to this launch when it writes back.
    fn prepare_owned_server_runtime(
        &self,
        store: &crate::db::Store,
        slot: i64,
        pane: &PaneConfig,
        mode: RestartMode,
    ) -> Result<ExactSessionRuntimeContext> {
        let prepared = store.prepare_opencode_runtime(
            &self.id,
            slot,
            match mode {
                RestartMode::Fresh => crate::db::RuntimePreparationMode::Fresh,
                RestartMode::Resume => crate::db::RuntimePreparationMode::Resume,
            },
        )?;
        debug_assert!(
            mode != RestartMode::Resume
                || crate::opencode_runtime::validate_session_id(&prepared.native_session_id)
                    .is_ok()
        );
        if pane.cross_agent_team {
            if prepared.xats_identity_key.is_empty() {
                anyhow::bail!("OpenCode Cross Agent Team slot {slot} has no identity key");
            }
            crate::opencode_xats::reserve(&prepared.xats_identity_key, prepared.generation)?;
        }
        Ok(ExactSessionRuntimeContext {
            shape: crate::agents::ExactSessionRuntime::OwnedServer,
            server_base_url: String::new(),
            slot,
            generation: prepared.generation,
            native_session_id: prepared.native_session_id,
            identity_key: prepared.xats_identity_key,
        })
    }

    /// A managed pane whose server AoE owns is launched through AoE's own
    /// runtime wrapper, so a custom command would start something that wrapper
    /// does not manage. A shared-server pane is the opposite case: the override
    /// is how the user names the binary AoE must launch.
    fn reject_command_override_for(
        &self,
        shape: crate::agents::ExactSessionRuntime,
        has_override: bool,
    ) -> Result<()> {
        if has_override && shape == crate::agents::ExactSessionRuntime::OwnedServer {
            anyhow::bail!("Managed host OpenCode does not support a custom command override");
        }
        Ok(())
    }

    #[cfg(test)]
    fn build_extra_pane_command(&self, target_agent: &str, cwd: &str) -> Option<ExtraPaneLaunch> {
        let mut pane = target_agent.resolve_for(self);
        pane.working_dir = cwd.to_string();
        self.build_extra_pane_config_command(&pane)
    }

    /// Record the durable slot of an extra pane this instance has just launched,
    /// so the key minted for it survives into every later relaunch and the pane
    /// is inside the restart fan-out before its first capture.
    ///
    /// The launched pane's working directory travels on `pane`, because it
    /// describes that pane. The instance's own directory is passed separately
    /// for the primary record, which describes the pane AoE did not just
    /// launch. The two are equal only when the pane was launched into the
    /// session's directory.
    ///
    /// A managed shell pane is recorded even when it inherits the session's
    /// directory. It produces no capture, so this launch-time record is its only
    /// durable lifecycle source for restart and cold recovery.
    ///
    /// A failure here is reported rather than logged. The pane is already
    /// running under the key it was launched with, but nothing holds that key,
    /// so the next relaunch mints a different one and the identity the pane just
    /// registered becomes unrecoverable. That outcome is invisible from the pane
    /// itself, which is why the caller has to hear about it.
    pub fn record_launched_extra_pane(
        &self,
        profile: &str,
        session_name: &str,
        pane: &crate::db::reconcile::LaunchedPane<'_>,
    ) -> Result<()> {
        crate::db::Store::open_with_schema(profile)
            .and_then(|store| {
                crate::db::reconcile::record_launched_extra_pane(
                    &store,
                    &self.id,
                    session_name,
                    &self.primary_pane,
                    self.xats_identity_key.as_deref().unwrap_or(""),
                    pane,
                )
            })
            .with_context(|| {
                format!(
                    "pane {} of '{}' is running but its identity key was not recorded; \
                     it will not survive a restart",
                    pane.pane_id, self.title
                )
            })
    }

    /// The user's shell for an extra pane, in `cwd`.
    ///
    /// The `cd` is defense in depth: the split already inherits the directory,
    /// and a login shell that resets it would otherwise land the pane
    /// elsewhere.
    fn build_extra_shell_pane_command(&self, cwd: &str) -> String {
        let outer_shell = crate::session::environment::user_posix_shell();
        let inner = if self.is_sandboxed() && self.sandbox_info.is_some() {
            let container = DockerContainer::from_session_id(&self.id);
            let workdir = self.container_workdir();
            let docker_cmd = container.exec_command(Some(&format!("-w {}", workdir)), &outer_shell);
            format!("stty susp undef; exec {}", docker_cmd)
        } else {
            let escaped_dir = shell_escape(cwd);
            let interactive_shell = crate::session::environment::user_shell();
            format!(
                "cd {} && stty susp undef; exec {}",
                escaped_dir,
                shell_escape(&interactive_shell)
            )
        };
        format!(
            "{} -lc '{}'",
            shell_escape(&outer_shell),
            inner.replace('\'', "'\\''")
        )
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
    /// `slot_identity_key` supplies the durable xats identity key for the target
    /// slot. The instance value is only a bootstrap source before slot 0 exists.
    pub(crate) fn build_pane_command(
        &self,
        target: impl PaneConfigTarget,
        resume_token: Option<&str>,
        is_primary: bool,
        slot_identity_key: Option<&str>,
    ) -> Option<String> {
        self.build_pane_command_with_runtime(
            target,
            resume_token,
            is_primary,
            slot_identity_key,
            None,
        )
    }

    fn build_pane_command_with_runtime(
        &self,
        target: impl PaneConfigTarget,
        resume_token: Option<&str>,
        is_primary: bool,
        slot_identity_key: Option<&str>,
        exact_runtime: Option<&ExactSessionRuntimeContext>,
    ) -> Option<String> {
        let pane = target.resolve_for(self);
        let agent = crate::agents::get_agent(&pane.tool);
        let is_primary = is_primary && self.pane_runs_instance_tool(&pane.tool);

        if self.is_sandboxed() {
            // A shared-server agent's conversation is minted on a server that
            // lives on the host, so nothing AoE could prepare would describe
            // what runs in the container. Refusing here keeps the launch from
            // reaching session minting, and no caller substitutes a shell.
            if agent.and_then(|a| a.exact_session_runtime)
                == Some(crate::agents::ExactSessionRuntime::SharedServer)
            {
                tracing::error!(
                    "Sandboxed '{}' is not supported: its conversation lives on \
                     the host's shared server",
                    pane.tool
                );
                return None;
            }
            let sandbox = self.sandbox_info.as_ref()?;
            let container = DockerContainer::from_session_id(&self.id);

            let base_cmd = self.build_base_pane_command(agent, resume_token, is_primary);
            let mut tool_cmd = if pane.yolo_mode {
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
            // A shared-server pane keeps its own command path even under an
            // override: the override names the binary, but the session, the
            // server and the engine mode still have to reach the pane, and the
            // base command carries none of them.
            let has_override = is_primary
                && self.has_command_override()
                && self
                    .shared_server_command_override(&pane, is_primary)
                    .is_none();

            if !has_override {
                agent.filter(|a| a.supports_host_launch).and_then(|a| {
                    let mut cmd = match a.exact_session_runtime {
                        Some(crate::agents::ExactSessionRuntime::OwnedServer) => self
                            .build_opencode_runtime_command(
                                &pane,
                                resume_token,
                                is_primary,
                                exact_runtime,
                            )?,
                        Some(crate::agents::ExactSessionRuntime::SharedServer) => self
                            .build_kimi_pane_command(
                                &pane,
                                resume_token,
                                exact_runtime,
                                is_primary,
                            )?,
                        None => self.build_base_pane_command(Some(a), resume_token, is_primary),
                    };
                    let mut env_vars: Vec<(&str, &str)> = Vec::new();
                    if needs_instance_id {
                        env_vars.push(("AOE_INSTANCE_ID", &self.id));
                    }
                    if pane.yolo_mode {
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
                    if pane.cross_agent_team {
                        match pane.tool.as_str() {
                            "claude" => {
                                if let Some(flag) = self.claude_cross_agent_team_flag(&pane) {
                                    cmd = format!("{} {}", cmd, flag);
                                }
                            }
                            "codex" => {
                                let base = self.pane_base_command(&pane.tool, is_primary);
                                cmd = self.codex_xats_bootstrap_command(
                                    &cmd,
                                    &base,
                                    &pane.working_dir,
                                );
                            }
                            _ => {}
                        }
                    }
                    if crate::agents::identity_key_in_pane_env(&pane.tool) {
                        if let Some(key) =
                            self.xats_identity_key_for_pane(&pane, is_primary, slot_identity_key)
                        {
                            env_vars.push((XATS_IDENTITY_KEY_ENV, key));
                        }
                    }
                    Some(wrap_command_ignore_suspend_with_env(&cmd, &env_vars))
                })
            } else {
                let mut cmd = self.build_base_pane_command(agent, resume_token, is_primary);
                let mut env_vars: Vec<(&str, &str)> = Vec::new();
                if needs_instance_id {
                    env_vars.push(("AOE_INSTANCE_ID", &self.id));
                }
                if pane.yolo_mode {
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
                if pane.cross_agent_team {
                    match pane.tool.as_str() {
                        "claude" => {
                            if let Some(flag) = self.claude_cross_agent_team_flag(&pane) {
                                cmd = format!("{} {}", cmd, flag);
                            }
                        }
                        "codex" => {
                            let base = self.pane_base_command(&pane.tool, is_primary);
                            cmd = self.codex_xats_bootstrap_command(&cmd, &base, &pane.working_dir);
                        }
                        _ => {}
                    }
                }
                if crate::agents::identity_key_in_pane_env(&pane.tool) {
                    if let Some(key) =
                        self.xats_identity_key_for_pane(&pane, is_primary, slot_identity_key)
                    {
                        env_vars.push((XATS_IDENTITY_KEY_ENV, key));
                    }
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

    /// Terminate a shared-server pane and confirm its exit before its slot is
    /// re-prepared.
    ///
    /// A fresh conversation abandons the session the running pane still holds.
    /// Nothing revokes the old coordinates on the kimi side -- an abandoned
    /// session keeps answering probes -- so the only thing that stops the old
    /// pane from re-committing over the new ones is that it is already gone when
    /// the new session is minted. Ordering is the whole defence, which is why it
    /// happens here rather than inside the relaunch that follows.
    fn serialize_shared_server_replacement(
        &self,
        shape: crate::agents::ExactSessionRuntime,
        mode: RestartMode,
        tmux_pane: &str,
    ) -> Result<()> {
        if shape != crate::agents::ExactSessionRuntime::SharedServer
            || mode != RestartMode::Fresh
            || tmux_pane.is_empty()
        {
            return Ok(());
        }
        tmux::set_pane_remain_on_exit(tmux_pane, true).with_context(|| {
            format!("holding pane {tmux_pane} open while replacing its kimi conversation")
        })?;
        tmux::kill_pane_process_tree_target(tmux_pane);
        for _ in 0..PANE_EXIT_ATTEMPTS {
            match crate::process::get_pane_pid(tmux_pane) {
                None => return Ok(()),
                Some(pid) if crate::process::is_unsafe_kill_root(pid) => return Ok(()),
                Some(_) => std::thread::sleep(PANE_EXIT_POLL),
            }
        }
        anyhow::bail!(
            "pane {tmux_pane} did not exit, so a new kimi conversation would be \
             minted while the old pane can still claim it"
        )
    }

    /// Mint or verify this pane's kimi session, persist it, then commit its
    /// delivery coordinates.
    ///
    /// The order is load bearing. The session reaches the durable slot before
    /// anything else can observe it, and the commit is the last xats-affecting
    /// step before the pane process starts, so the agent's own `reconnect` finds
    /// coordinates AoE already fixed rather than racing them.
    fn prepare_shared_server_session(
        &self,
        store: &crate::db::Store,
        pane: &PaneConfig,
        tmux_pane: &str,
        runtime: &mut ExactSessionRuntimeContext,
        mode: RestartMode,
    ) -> Result<()> {
        if pane.cross_agent_team && runtime.identity_key.is_empty() {
            anyhow::bail!(
                "kimi Cross Agent Team slot {} has no identity key",
                runtime.slot
            );
        }
        let launch = crate::kimi::prepare_session(&crate::kimi::PaneRequest {
            working_directory: std::path::PathBuf::from(&pane.working_dir),
            cross_agent_team: pane.cross_agent_team,
            mode: match mode {
                RestartMode::Resume => crate::kimi::SessionMode::Resume,
                RestartMode::Fresh => crate::kimi::SessionMode::Fresh,
            },
            durable_session_id: runtime.native_session_id.clone(),
            command_override: self
                .shared_server_command_override(pane, runtime.slot == 0)
                .map(str::to_string),
        })?;
        store.upsert_agent_slot_config(
            &self.id,
            runtime.slot,
            pane,
            &launch.session_id,
            tmux_pane,
            &runtime.identity_key,
            crate::db::now_unix(),
        )?;
        let previous_session_id =
            std::mem::replace(&mut runtime.native_session_id, launch.session_id.clone());
        runtime.server_base_url = launch.base_url.clone();
        if pane.cross_agent_team {
            crate::kimi::commit_delivery(&runtime.identity_key, &previous_session_id, &launch)?;
        }
        Ok(())
    }

    /// Claim the xats runtime of a freshly provisioned extra pane slot.
    ///
    /// An owned-server slot reserves its generation and lets its runtime wrapper
    /// mint the conversation against the server it starts. A shared-server slot
    /// has no fence to reserve, so AoE mints the conversation here and commits
    /// its coordinates itself.
    fn claim_new_extra_pane_runtime(
        &self,
        store: &crate::db::Store,
        pane: &PaneConfig,
        runtime: &mut ExactSessionRuntimeContext,
    ) -> Result<()> {
        match runtime.shape {
            crate::agents::ExactSessionRuntime::OwnedServer => {
                if pane.cross_agent_team {
                    crate::opencode_xats::reserve(&runtime.identity_key, runtime.generation)?;
                }
                Ok(())
            }
            crate::agents::ExactSessionRuntime::SharedServer => {
                self.prepare_shared_server_session(store, pane, "", runtime, RestartMode::Fresh)
            }
        }
    }

    /// The kimi pane command: the configured binary attaching to the exact
    /// session AoE minted, with the shared server and engine mode in the
    /// environment.
    ///
    /// The identity key appears in neither, by construction: it is not a
    /// parameter of this function. A kimi tool process is spawned by the shared
    /// server and inherits that server's environment, so a key that reached any
    /// pane would be readable by every kimi agent on the machine.
    /// The command override that names this pane's shared-server binary, when
    /// the pane is the instance's own and the user set one.
    ///
    /// A shared-server agent is the one case where an override is not a way to
    /// bypass what AoE prepared: AoE still owns the session, the server and the
    /// engine mode, and the override only says which binary attaches to them.
    /// An extra pane has no override of its own and names its binary through
    /// [`crate::kimi::COMMAND_ENV`] instead.
    fn shared_server_command_override(&self, pane: &PaneConfig, is_primary: bool) -> Option<&str> {
        let shared = self.pane_exact_session_runtime(pane)
            == Some(crate::agents::ExactSessionRuntime::SharedServer);
        (shared
            && is_primary
            && self.pane_runs_instance_tool(&pane.tool)
            && self.has_command_override())
        .then(|| self.get_tool_command())
    }

    fn build_kimi_pane_command(
        &self,
        pane: &PaneConfig,
        resume_session: Option<&str>,
        runtime: Option<&ExactSessionRuntimeContext>,
        is_primary: bool,
    ) -> Option<String> {
        let runtime = runtime?;
        if runtime.server_base_url.is_empty() {
            return None;
        }
        let session_id = resume_session
            .filter(|value| !value.is_empty())
            .unwrap_or(runtime.native_session_id.as_str());
        if crate::kimi::validate_session_id(session_id).is_err() {
            return None;
        }
        let words = crate::kimi::command_words(
            pane.cross_agent_team,
            self.shared_server_command_override(pane, is_primary),
        )
        .map_err(|error| tracing::error!("Cannot build kimi pane command: {error:#}"))
        .ok()?;
        let injected = [
            (crate::kimi::BASE_URL_ENV, runtime.server_base_url.as_str()),
            (crate::kimi::SESSION_ID_ENV, session_id),
            (crate::kimi::REMOTE_MODE_ENV, crate::kimi::REMOTE_MODE_VALUE),
        ];
        // The launch wrapper already execs through `env`, so this command adds
        // only its arguments. Remove before setting, and drop the identity key
        // on the way through:
        // a kimi tool process is spawned by the shared server, so any of these
        // inherited from elsewhere would describe another pane's session.
        let mut parts: Vec<String> = Vec::new();
        for name in injected
            .iter()
            .map(|(name, _)| *name)
            .chain([crate::xats_control::IDENTITY_KEY_ENV])
        {
            parts.push("-u".to_string());
            parts.push(name.to_string());
        }
        for (name, value) in injected {
            parts.push(format!("{name}={}", shell_escape(value)));
        }
        parts.extend(words.iter().map(|word| shell_escape(word)));
        parts.push("--session".to_string());
        parts.push(shell_escape(session_id));
        if !self.extra_args.is_empty() && self.pane_runs_instance_tool(&pane.tool) {
            let extra = crate::kimi::parse_and_validate_extra_args(&self.extra_args)
                .map_err(|error| tracing::error!("Invalid kimi extra args: {error:#}"))
                .ok()?;
            parts.extend(extra.iter().map(|arg| shell_escape(arg)));
        }
        Some(parts.join(" "))
    }

    fn build_opencode_runtime_command(
        &self,
        pane: &PaneConfig,
        resume_session: Option<&str>,
        is_primary: bool,
        runtime: Option<&ExactSessionRuntimeContext>,
    ) -> Option<String> {
        let fallback = ExactSessionRuntimeContext {
            shape: crate::agents::ExactSessionRuntime::OwnedServer,
            server_base_url: String::new(),
            slot: if is_primary { 0 } else { 1 },
            generation: 0,
            native_session_id: resume_session.unwrap_or_default().to_string(),
            identity_key: String::new(),
        };
        let runtime = runtime.unwrap_or(&fallback);
        if pane.cross_agent_team && runtime.generation == 0 {
            return None;
        }
        let executable = std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "aoe".to_string());
        let mut parts = vec![
            shell_escape(&executable),
            "__opencode-runtime".to_string(),
            "--instance-id".to_string(),
            shell_escape(&self.id),
            "--slot".to_string(),
            runtime.slot.to_string(),
            "--generation".to_string(),
            runtime.generation.to_string(),
            "--working-directory".to_string(),
            shell_escape(&pane.working_dir),
        ];
        let exact_session = resume_session
            .filter(|value| !value.is_empty())
            .or_else(|| {
                (!runtime.native_session_id.is_empty())
                    .then_some(runtime.native_session_id.as_str())
            });
        if let Some(session_id) = exact_session {
            if crate::opencode_runtime::validate_session_id(session_id).is_err() {
                return None;
            }
            parts.push("--resume-session".to_string());
            parts.push(shell_escape(session_id));
        }
        if pane.cross_agent_team {
            parts.push("--cross-agent-team".to_string());
        }
        if is_primary && !self.extra_args.is_empty() {
            let extra = crate::opencode_runtime::parse_and_validate_extra_args(&self.extra_args)
                .map_err(|error| {
                    tracing::error!("Invalid OpenCode extra args: {error:#}");
                })
                .ok()?;
            if !extra.is_empty() {
                parts.push("--".to_string());
                parts.extend(extra.iter().map(|arg| shell_escape(arg)));
            }
        }
        Some(parts.join(" "))
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
        if let Some(metadata) = fork.primary_pane.worktree.as_mut() {
            if let Some(worktree) = metadata.worktree.as_mut() {
                worktree.cleanup_on_delete = false;
            }
            if let Some(workspace) = metadata.workspace.as_mut() {
                workspace.cleanup_on_delete = false;
            }
        }
        fork.sync_legacy_primary_fields();

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
                "Fork is not supported for agent '{}'. Supported: claude, codex, and sandboxed opencode.",
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
            "opencode" if !self.is_sandboxed() => anyhow::bail!(
                "Fork is not supported for managed host OpenCode until exact-session runtime fork is available"
            ),
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
        let identity_origin = self.ensure_xats_identity_key();
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
        let pane = match pane_agent {
            Some(agent) => {
                let mut pane = self.primary_pane.clone();
                pane.tool = agent.to_string();
                pane
            }
            None => self.primary_pane.clone(),
        };
        let exact_runtime = if let Some(shape) = self.pane_exact_session_runtime(&pane) {
            self.reject_command_override_for(
                shape,
                pane_agent.is_none() && self.has_command_override(),
            )?;
            let profile = Self::current_profile();
            let store = crate::db::Store::open_with_schema(&profile)?;
            let pane_id =
                tmux::get_agent_pane_id(&tmux::Session::generate_name(&self.id, &self.title))
                    .ok_or_else(|| anyhow::anyhow!("primary pane id was not recorded"))?;
            if !store
                .read_slots_for_instance(&self.id)?
                .iter()
                .any(|slot| slot.slot == 0)
            {
                store.upsert_agent_slot_config(
                    &self.id,
                    0,
                    &pane,
                    effective_resume_token.as_deref().unwrap_or(""),
                    &pane_id,
                    self.xats_identity_key.as_deref().unwrap_or(""),
                    crate::db::now_unix(),
                )?;
            }
            Some(self.prepare_exact_session_runtime(
                &store,
                0,
                &pane,
                &pane_id,
                self.xats_identity_key.as_deref().unwrap_or(""),
                mode,
            )?)
        } else {
            None
        };
        let cmd = match exact_runtime.as_ref() {
            Some(runtime) => self.build_pane_command_with_runtime(
                &pane,
                Some(runtime.native_session_id.as_str()).filter(|id| !id.is_empty()),
                pane_agent.is_none(),
                Some(runtime.identity_key.as_str()).filter(|key| !key.is_empty()),
                Some(runtime),
            ),
            None => match pane_agent {
                Some(_) => self.build_pane_command(&pane, None, false, None),
                None => self.build_agent_command(effective_resume_token.as_deref()),
            },
        }
        .ok_or_else(|| anyhow::anyhow!("No agent command available"))?;

        session.kill_agent_pane_process_tree();
        session.respawn_agent_pane(&cmd, &self.project_path, !self.expects_shell())?;

        self.run_auto_confirm(&pane, identity_origin);

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
    ///
    /// `identity_origins` is what the caller's [`ensure_slot_identity_keys`]
    /// reported, keyed by slot index: a slot missing from it is treated as
    /// keyless, so an unread store costs a reconnect rather than sending one to a
    /// pane that may not own the identity.
    ///
    /// [`ensure_slot_identity_keys`]: Self::ensure_slot_identity_keys
    pub fn resume_all_tracked_panes(
        &mut self,
        slots: &[crate::db::AgentSlot],
        mode: RestartMode,
        identity_origins: &HashMap<i64, IdentityKeyOrigin>,
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
        let mut confirmable_panes: Vec<LaunchedClaudePane> = Vec::new();
        for slot in slots {
            let pane = slot.pane_config();
            let mut native_session_id = self.slot_resume_source(slot, mode);
            let mut identity_key = slot.xats_identity_key.clone();
            let exact_runtime = if let Some(shape) = self.pane_exact_session_runtime(&pane) {
                if let Err(error) = self.reject_command_override_for(
                    shape,
                    slot.slot == 0 && self.has_command_override(),
                ) {
                    outcomes.push(PaneResumeOutcome::Error(format!("{error:#}")));
                    continue;
                }
                let store = match crate::db::Store::open_with_schema(&Self::current_profile()) {
                    Ok(store) => store,
                    Err(error) => {
                        outcomes.push(PaneResumeOutcome::Error(format!("{error:#}")));
                        continue;
                    }
                };
                match self.prepare_exact_session_runtime(
                    &store,
                    slot.slot,
                    &pane,
                    &slot.tmux_pane,
                    &slot.xats_identity_key,
                    mode,
                ) {
                    Ok(runtime) => {
                        native_session_id = runtime.native_session_id.clone();
                        identity_key = runtime.identity_key.clone();
                        Some(runtime)
                    }
                    Err(error) => {
                        outcomes.push(PaneResumeOutcome::Error(format!("{error:#}")));
                        continue;
                    }
                }
            } else {
                None
            };
            let outcome = self.resume_launch_pane(
                &pane,
                &native_session_id,
                &slot.tmux_pane,
                slot.slot == 0,
                mode,
                Some(identity_key.as_str()).filter(|key| !key.is_empty()),
                exact_runtime.as_ref(),
            );
            // Every Claude pane this fan-out actually relaunched raises its own
            // startup screens, not just the primary one.
            if pane.tool == "claude"
                && pane.cross_agent_team
                && !matches!(outcome, PaneResumeOutcome::Error(_))
            {
                confirmable_panes.push(self.launched_claude_pane(
                    slot.tmux_pane.clone(),
                    &pane,
                    slot_identity_origin(identity_origins, slot.slot),
                ));
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
        identity_origins: &HashMap<i64, IdentityKeyOrigin>,
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
        let mut confirmable_panes: Vec<LaunchedClaudePane> = Vec::new();
        for (slot, maybe_pane) in &paired {
            let Some(new_pane) = maybe_pane else {
                outcomes.push(PaneResumeOutcome::Error(format!(
                    "pane creation failed for slot {} (cwd {})",
                    slot.slot, slot.cwd
                )));
                continue;
            };
            let pane = slot.pane_config();
            let mut native_session_id = slot.native_session_id.clone();
            let mut identity_key = slot.xats_identity_key.clone();
            let exact_runtime = if let Some(shape) = self.pane_exact_session_runtime(&pane) {
                if let Err(error) = self.reject_command_override_for(
                    shape,
                    slot.slot == 0 && self.has_command_override(),
                ) {
                    outcomes.push(PaneResumeOutcome::Error(format!("{error:#}")));
                    continue;
                }
                match self.prepare_exact_session_runtime(
                    store,
                    slot.slot,
                    &pane,
                    new_pane,
                    &slot.xats_identity_key,
                    mode,
                ) {
                    Ok(runtime) => {
                        native_session_id = runtime.native_session_id.clone();
                        identity_key = runtime.identity_key.clone();
                        Some(runtime)
                    }
                    Err(error) => {
                        outcomes.push(PaneResumeOutcome::Error(format!("{error:#}")));
                        continue;
                    }
                }
            } else {
                None
            };
            let outcome = self.resume_launch_pane(
                &pane,
                &native_session_id,
                new_pane,
                slot.slot == 0,
                mode,
                Some(identity_key.as_str()).filter(|key| !key.is_empty()),
                exact_runtime.as_ref(),
            );
            if pane.tool == "claude"
                && pane.cross_agent_team
                && !matches!(outcome, PaneResumeOutcome::Error(_))
            {
                confirmable_panes.push(self.launched_claude_pane(
                    new_pane.clone(),
                    &pane,
                    slot_identity_origin(identity_origins, slot.slot),
                ));
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
            if let Err(e) = store.upsert_agent_slot_config(
                &slot.instance_id,
                slot.slot,
                &pane,
                &native_session_id,
                new_pane,
                &identity_key,
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

    /// The conversation a slot resumes from during the fan-out restart.
    ///
    /// A slot record written at launch carries no native session id until the
    /// pane's first capture. For slot 0 the instance's stored resume token
    /// describes that same pane and is the only source in that window: it is
    /// scraped from the primary pane's own output when the agent exits and
    /// prints a resume hint, which happens before any capture exists. A restart
    /// with no tracked panes already consults it, and once every launched pane
    /// has a record from launch this is the path that runs in that window
    /// instead.
    ///
    /// Slot 0 alone, and only while it runs the instance's own tool: the token
    /// names that agent's conversation and nothing else. `Fresh` resumes
    /// nothing.
    fn slot_resume_source(&self, slot: &crate::db::AgentSlot, mode: RestartMode) -> String {
        let stands_in = mode == RestartMode::Resume
            && slot.slot == 0
            && slot.native_session_id.is_empty()
            && self.pane_runs_instance_tool(&slot.agent);
        if stands_in {
            return self.resume_token.clone().unwrap_or_default();
        }
        slot.native_session_id.clone()
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

fn require_launch_command(
    launch: SessionLaunch,
    command: Option<String>,
    tool: &str,
) -> Result<Option<String>> {
    if launch == SessionLaunch::Agent && command.is_none() {
        anyhow::bail!("Could not build {tool} launch command");
    }
    Ok(command)
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

/// Bounded wait for a replaced pane's process tree to actually be gone.
const PANE_EXIT_ATTEMPTS: usize = 50;
const PANE_EXIT_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Snapshot of the identity fields a fresh restart speculatively mutates
/// (`agent_session_id`, `fork_pending`), captured so the restart can roll them
/// back if the respawn never actually starts a new conversation.
type FreshIdentitySnapshot = (Option<String>, Option<String>);

/// The launch command of an extra agent pane and the identity key minted for it.
///
/// The two travel together because the key has to be persisted on the pane's
/// slot record as well as injected into its process: a key that only reaches the
/// environment is minted afresh on every relaunch, which reads to the daemon as
/// a new agent rather than a returning one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraPaneLaunch {
    pub command: String,
    /// The agent this pane was provisioned for, so a rollback can name the slot
    /// it is allowed to remove.
    pub agent: String,
    /// The conversation the provisioned slot holds. Empty unless AoE minted one
    /// before launch; it completes the token a rollback has to match.
    pub native_session_id: String,
    /// Empty when the pane gets no identity: Cross Agent Team is off, or the
    /// pane runs a shell, which registers no identity at all.
    pub identity_key: String,
    pub prepared_slot: Option<i64>,
    pub prepared_generation: Option<i64>,
}

fn append_rollback_error(error: anyhow::Error, cleanup: Result<()>) -> anyhow::Error {
    match cleanup {
        Ok(()) => error,
        Err(cleanup_error) => anyhow::anyhow!(
            "{error:#}. Failed to roll back prepared OpenCode slot: {cleanup_error:#}"
        ),
    }
}

/// One pane's AoE-prepared conversation, carried from preparation through
/// command building to the post-launch slot binding.
///
/// `shape` travels with it because who owns the server decides how the pane is
/// launched and what a failure may clean up; `generation` fences only an
/// [`ExactSessionRuntime::OwnedServer`] runtime, where AoE both prepares and
/// terminates the server, and stays at its stored value otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExactSessionRuntimeContext {
    shape: crate::agents::ExactSessionRuntime,
    /// The shared server this pane's conversation lives on. Empty for an
    /// owned-server runtime, whose wrapper allocates its own loopback port.
    server_base_url: String,
    slot: i64,
    generation: i64,
    native_session_id: String,
    identity_key: String,
}

/// Validate a durable conversation id against the runtime that owns it.
fn validate_exact_session_id(shape: crate::agents::ExactSessionRuntime, value: &str) -> Result<()> {
    match shape {
        crate::agents::ExactSessionRuntime::OwnedServer => {
            crate::opencode_runtime::validate_session_id(value)
        }
        crate::agents::ExactSessionRuntime::SharedServer => crate::kimi::validate_session_id(value),
    }
}

/// Validate the instance's extra launch arguments against the runtime that owns
/// the pane. Each runtime already decided the session, the server and the engine
/// mode, so each refuses the arguments that would change them.
fn validate_exact_runtime_extra_args(
    shape: crate::agents::ExactSessionRuntime,
    value: &str,
) -> Result<Vec<String>> {
    match shape {
        crate::agents::ExactSessionRuntime::OwnedServer => {
            crate::opencode_runtime::parse_and_validate_extra_args(value)
        }
        crate::agents::ExactSessionRuntime::SharedServer => {
            crate::kimi::parse_and_validate_extra_args(value)
        }
    }
}

impl ExactSessionRuntimeContext {
    /// Attach the tmux pane AoE just created to this prepared slot.
    ///
    /// An [`ExactSessionRuntime::OwnedServer`] slot is still conversationless at
    /// this point -- its runtime wrapper records the session once the server it
    /// owns has minted one -- so the binding is guarded by the generation that
    /// prepared it. A [`ExactSessionRuntime::SharedServer`] slot already carries
    /// the session AoE minted before launch, so the same guard would never
    /// match and the pane id is written alongside it instead.
    fn bind_launched_pane(
        &self,
        store: &crate::db::Store,
        instance_id: &str,
        pane: &PaneConfig,
        pane_id: &str,
    ) -> Result<()> {
        match self.shape {
            crate::agents::ExactSessionRuntime::OwnedServer => store.bind_prepared_slot_pane(
                instance_id,
                self.slot,
                self.generation,
                &self.identity_key,
                pane_id,
                crate::db::now_unix(),
            ),
            crate::agents::ExactSessionRuntime::SharedServer => store.upsert_agent_slot_config(
                instance_id,
                self.slot,
                pane,
                &self.native_session_id,
                pane_id,
                &self.identity_key,
                crate::db::now_unix(),
            ),
        }
    }
}

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
    #[cfg(test)]
    fn build_pane_resume_plan(
        &self,
        target: impl PaneConfigTarget,
        native_session_id: &str,
        is_primary: bool,
        mode: RestartMode,
        slot_identity_key: Option<&str>,
    ) -> Option<(String, bool)> {
        self.build_pane_resume_plan_with_runtime(
            target,
            native_session_id,
            is_primary,
            mode,
            slot_identity_key,
            None,
        )
    }

    fn build_pane_resume_plan_with_runtime(
        &self,
        target: impl PaneConfigTarget,
        native_session_id: &str,
        is_primary: bool,
        mode: RestartMode,
        slot_identity_key: Option<&str>,
        exact_runtime: Option<&ExactSessionRuntimeContext>,
    ) -> Option<(String, bool)> {
        let pane = target.resolve_for(self);
        let Some(def) = crate::agents::get_agent(&pane.tool) else {
            // Unknown agent: only the recorded name can act as the binary, and
            // only if it is a safe command token; otherwise refuse to build a
            // command. Unknown agents cannot be decorated with launch context.
            return PaneConfig::is_safe_tool_name(&pane.tool).then(|| (pane.tool.clone(), false));
        };

        // `Fresh` forces the no-resume path: still build the full launch context
        // via `build_pane_command`, but never append a resume flag.
        let resumed = mode == RestartMode::Resume
            && def.resume.is_some()
            && is_valid_resume_token(native_session_id);
        let resume_token = resumed.then_some(native_session_id);
        let command = self.build_pane_command_with_runtime(
            &pane,
            resume_token,
            is_primary,
            slot_identity_key,
            exact_runtime,
        )?;
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
        pane: &PaneConfig,
        native_session_id: &str,
        tmux_pane: &str,
        is_primary: bool,
        mode: RestartMode,
        slot_identity_key: Option<&str>,
        exact_runtime: Option<&ExactSessionRuntimeContext>,
    ) -> PaneResumeOutcome {
        if let Some(shape) = self.pane_exact_session_runtime(pane) {
            if mode == RestartMode::Resume
                && validate_exact_session_id(shape, native_session_id).is_err()
            {
                return PaneResumeOutcome::Error(format!(
                    "{} pane has no valid session to resume: '{}'",
                    pane.tool, native_session_id
                ));
            }
        }
        // A shell slot is relaunched the way it was launched. The registry
        // entry's binary is the literal `shell`, which names no program, so the
        // launch path builds this pane through `build_extra_shell_pane_command`
        // and the resume path has to do the same.
        let plan = if pane_agent_is_shell(&pane.tool) {
            Some((
                self.build_extra_shell_pane_command(&pane.working_dir),
                false,
            ))
        } else {
            // Build (and validate) the command before killing the pane, so a pane
            // we cannot safely respawn is left running rather than killed and
            // abandoned.
            self.build_pane_resume_plan_with_runtime(
                pane,
                native_session_id,
                is_primary,
                mode,
                slot_identity_key,
                exact_runtime,
            )
        };
        let Some((command, resumed)) = plan else {
            return PaneResumeOutcome::Error(format!("unsafe or unknown agent '{}'", pane.tool));
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

        if let Err(err) = tmux::respawn_pane_target(
            tmux_pane,
            &command,
            &pane.working_dir,
            !pane_agent_is_shell(&pane.tool),
        ) {
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
    crate::opencode_runtime::validate_session_id(s).is_ok()
        || crate::kimi::validate_session_id(s).is_ok()
        || (!s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
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

        // Codex has no hook configuration (its hooks cannot be trusted to run
        // in the pane; see `AgentHookConfig`), so its launch carries no
        // `AOE_INSTANCE_ID` and the resume flag sits directly after the binary.
        let shell = crate::session::environment::user_posix_shell();
        assert!(
            cmd.ends_with(&format!(
                "{shell} -lc 'stty susp undef; exec env codex resume \
                 019d1af9-a899-7df1-8f7d-a244126e5ded --model gpt-5 \
                 --dangerously-bypass-approvals-and-sandbox'"
            )),
            "unexpected codex resume command: {cmd}"
        );
        assert!(
            !cmd.contains("AOE_INSTANCE_ID"),
            "a hookless agent's launch must not carry AOE_INSTANCE_ID: {cmd}"
        );
    }

    /// A launch command stays free of `shell_environment_policy` overrides.
    /// An earlier fix rode the pane and instance id in through them before a
    /// live session showed Codex applies that table to its shell tool only,
    /// never to its hooks -- dead weight on every launch, so none is emitted.
    #[test]
    fn test_no_launch_carries_shell_environment_policy_overrides() {
        for tool in ["codex", "claude"] {
            let mut inst = Instance::new("test", "/tmp/test");
            inst.tool = tool.to_string();

            let cmd = inst.build_agent_command(None).unwrap();

            assert!(
                !cmd.contains("shell_environment_policy"),
                "{tool} launch must not carry policy overrides: {cmd}"
            );
        }
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
    fn extra_codex_pane_carries_the_xats_bootstrap() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.cross_agent_team = true;

        let cmd = inst
            .build_extra_pane_command("codex", &inst.project_path)
            .unwrap()
            .command;
        assert!(
            cmd.contains("pre-register-codex-pane"),
            "expected pane pre-registration, got {cmd}"
        );
        assert!(
            cmd.contains("--remote"),
            "expected the app-server connection, got {cmd}"
        );
    }

    #[test]
    fn extra_claude_pane_carries_the_channel_flag() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;

        let cmd = inst
            .build_extra_pane_command("claude", &inst.project_path)
            .unwrap()
            .command;
        assert!(
            cmd.contains("--dangerously-load-development-channels"),
            "expected dev-channels flag, got {cmd}"
        );
    }

    #[test]
    fn extra_pane_leaves_the_instance_launch_context_behind() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;
        inst.command = "claude --instance-override".to_string();
        inst.agent_session_id = Some("preallocated-id".to_string());
        inst.xats_identity_key = Some("instance-key".to_string());

        let cmd = inst
            .build_extra_pane_command("claude", &inst.project_path)
            .unwrap()
            .command;
        assert!(
            !cmd.contains("--instance-override"),
            "override belongs to the instance's own pane, got {cmd}"
        );
        assert!(
            !cmd.contains("preallocated-id"),
            "session id belongs to the instance's own conversation, got {cmd}"
        );
        assert!(
            !cmd.contains("instance-key"),
            "two panes must not present one identity, got {cmd}"
        );
    }

    #[test]
    fn extra_codex_pane_is_bootstrapped_from_its_own_binary_not_the_override() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.cross_agent_team = true;
        inst.command = "codex --instance-override".to_string();
        inst.xats_identity_key = Some("instance-key".to_string());

        let cmd = inst
            .build_extra_pane_command("codex", &inst.project_path)
            .unwrap()
            .command;
        assert!(
            !cmd.contains("--instance-override"),
            "the bootstrap must not relaunch the instance's own program, got {cmd}"
        );
        assert!(
            !cmd.contains("instance-key"),
            "two panes must not present one identity, got {cmd}"
        );
    }

    #[test]
    #[serial_test::serial(shell_env)]
    fn extra_shell_pane_adds_cd_defense_in_depth() {
        let original_shell = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "/bin/zsh");

        let inst = Instance::new("test", "/tmp/project path/it's here");
        let command = inst
            .build_extra_pane_command("shell", &inst.project_path)
            .unwrap()
            .command;
        let escaped_dir = shell_escape(&inst.project_path).replace('\'', "'\\''");

        assert!(command.starts_with("'/bin/zsh' -lc '"));
        assert!(command.contains(&format!(
            "cd {} && stty susp undef; exec '\\''/bin/zsh'\\''",
            escaped_dir
        )));

        match original_shell {
            Some(shell) => std::env::set_var("SHELL", shell),
            None => std::env::remove_var("SHELL"),
        }
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial(shell_env)]
    fn extra_shell_pane_does_not_source_a_bash_login_profile() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let shell_dir = tmp.path().join("shell dir");
        std::fs::create_dir(&shell_dir).unwrap();
        let fake_zsh = shell_dir.join("zsh");
        let reached = tmp.path().join("zsh-reached");
        let bash_sourced = tmp.path().join("bash-sourced");
        let fake_body = format!(
            "#!/bin/sh\nif [ \"$1\" = \"-lc\" ]; then exec /bin/sh -c \"$2\"; fi\nprintf reached > '{}'\n",
            reached.display()
        );
        std::fs::write(&fake_zsh, fake_body).unwrap();
        std::fs::set_permissions(&fake_zsh, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            tmp.path().join(".bash_profile"),
            format!("printf sourced > '{}'\n", bash_sourced.display()),
        )
        .unwrap();

        let original_shell = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", &fake_zsh);
        let command = Instance::new("test", tmp.path().to_str().unwrap())
            .build_extra_pane_command("shell", tmp.path().to_str().unwrap())
            .unwrap()
            .command;
        match original_shell {
            Some(shell) => std::env::set_var("SHELL", shell),
            None => std::env::remove_var("SHELL"),
        }

        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .env("HOME", tmp.path())
            .output()
            .unwrap();

        assert!(output.status.success(), "command failed: {output:?}");
        assert!(reached.exists(), "the configured zsh path was not reached");
        assert!(
            !bash_sourced.exists(),
            "an unrelated Bash login profile was sourced"
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial(shell_env)]
    fn extra_shell_pane_preserves_a_non_posix_interactive_shell() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let fake_fish = tmp.path().join("user shell").join("fish");
        std::fs::create_dir(fake_fish.parent().unwrap()).unwrap();
        let reached = tmp.path().join("fish-reached");
        std::fs::write(
            &fake_fish,
            format!("#!/bin/sh\nprintf reached > '{}'\n", reached.display()),
        )
        .unwrap();
        std::fs::set_permissions(&fake_fish, std::fs::Permissions::from_mode(0o755)).unwrap();

        let original_shell = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", &fake_fish);
        let command = Instance::new("test", tmp.path().to_str().unwrap())
            .build_extra_pane_command("shell", tmp.path().to_str().unwrap())
            .unwrap()
            .command;
        match original_shell {
            Some(shell) => std::env::set_var("SHELL", shell),
            None => std::env::remove_var("SHELL"),
        }

        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .env("HOME", tmp.path())
            .output()
            .unwrap();

        assert!(command.starts_with("'bash' -lc '"));
        assert!(output.status.success(), "command failed: {output:?}");
        assert!(reached.exists(), "the configured fish path was not reached");
    }

    #[test]
    #[serial_test::serial(shell_env)]
    fn extra_agent_pane_keeps_the_tmux_cwd() {
        let original_shell = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "/bin/zsh");

        let mut inst = Instance::new("test", "/tmp/project");
        inst.tool = "claude".to_string();
        let command = inst
            .build_extra_pane_command("claude", &inst.project_path)
            .unwrap()
            .command;

        assert!(command.contains("stty susp undef; exec env claude"));
        assert!(!command.contains("cd "));

        match original_shell {
            Some(shell) => std::env::set_var("SHELL", shell),
            None => std::env::remove_var("SHELL"),
        }
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

    #[test]
    fn codex_app_server_url_parses_into_a_gate_and_a_remote_argument() {
        let default = parse_codex_app_server_url(CODEX_XATS_APP_SERVER_DEFAULT_URL).unwrap();
        assert_eq!(default.host, "127.0.0.1");
        assert_eq!(default.port, 8799);
        assert_eq!(
            default.url, CODEX_XATS_APP_SERVER_DEFAULT_URL,
            "the URL is kept as written; a Url round trip would append a slash"
        );

        let alternate = parse_codex_app_server_url("ws://localhost:8899").unwrap();
        assert_eq!(alternate.host, "localhost");
        assert_eq!(alternate.port, 8899);

        let v6 = parse_codex_app_server_url("ws://[::1]:8899").unwrap();
        assert_eq!(v6.host, "::1", "nc wants the literal without brackets");
        assert_eq!(v6.url, "ws://[::1]:8899", "the URL keeps them");
    }

    /// xats takes `wss:` and a path, so AoE has to as well. A value xats accepts
    /// and AoE refuses is the same silent split this resolution exists to
    /// remove, entering through a different door.
    #[test]
    fn codex_app_server_url_accepts_what_xats_accepts() {
        let secure = parse_codex_app_server_url("wss://example.test:8899").unwrap();
        assert_eq!(secure.host, "example.test");
        assert_eq!(secure.port, 8899);

        let pathed = parse_codex_app_server_url("ws://127.0.0.1:8899/codex").unwrap();
        assert_eq!(pathed.port, 8899, "the gate ignores the path");
        assert_eq!(pathed.url, "ws://127.0.0.1:8899/codex", "the path survives");

        assert_eq!(
            parse_codex_app_server_url("wss://example.test")
                .unwrap()
                .port,
            443,
            "a scheme's default port is known"
        );
        assert_eq!(
            parse_codex_app_server_url("  ws://127.0.0.1:8899  ")
                .unwrap()
                .url,
            "ws://127.0.0.1:8899",
            "surrounding whitespace is trimmed, as xats trims it"
        );
    }

    /// The host reaches a generated `sh -c` script, so this is the guard against
    /// injecting into a command AoE runs, not a formatting preference.
    #[test]
    fn codex_app_server_url_rejects_non_websocket_and_unsafe_hosts() {
        for rejected in [
            "http://127.0.0.1:8799",
            "file:///etc/passwd",
            "127.0.0.1:8799",
            "ws://",
            "ws://:8799",
            "ws://127.0.0.1:port",
            "ws://127.0.0.1:70000",
            "ws://127.0.0.1;touch /tmp/pwned:8799",
            "ws://$(id)",
            "ws://127.0.0.1 -e sh",
            "",
        ] {
            assert!(
                parse_codex_app_server_url(rejected).is_none(),
                "must reject {rejected:?}"
            );
        }
    }

    #[test]
    fn codex_app_server_endpoint_follows_the_xats_variable() {
        let resolve = |single: Option<&str>, list: Option<&str>| {
            resolve_codex_app_server_endpoint(single.map(str::to_string), list.map(str::to_string))
        };

        assert_eq!(
            resolve(None, None).unwrap().url,
            CODEX_XATS_APP_SERVER_DEFAULT_URL,
            "unset means the default"
        );

        let configured = resolve(Some("ws://127.0.0.1:8899"), None).unwrap();
        assert_eq!(configured.url, "ws://127.0.0.1:8899");
        assert_eq!(configured.port, 8899, "the gate follows the same endpoint");

        let rejected = resolve(Some("ws://127.0.0.1:nope"), None).unwrap_err();
        assert!(
            rejected.contains(CODEX_XATS_APP_SERVER_URL_ENV) && rejected.contains("nope"),
            "the diagnostic must name the variable and the value: {rejected}"
        );

        assert_eq!(
            resolve(None, Some("[\"ws://127.0.0.1:8899\"]"))
                .unwrap()
                .url,
            "ws://127.0.0.1:8899",
            "a list with nothing to pick between leaves nothing to guess at"
        );

        let ambiguous = resolve(
            None,
            Some("[\"ws://127.0.0.1:8899\",\"ws://127.0.0.1:8898\"]"),
        )
        .unwrap_err();
        assert!(
            ambiguous.contains(CODEX_XATS_APP_SERVER_URL_ENV),
            "an ambiguous list must point at the single-endpoint variable: {ambiguous}"
        );

        assert_eq!(
            resolve(
                Some("ws://127.0.0.1:8899"),
                Some("[\"ws://127.0.0.1:8898\"]")
            )
            .unwrap()
            .url,
            "ws://127.0.0.1:8899",
            "the single-endpoint variable wins, as it does in xats"
        );
    }

    /// The bootstrap must `exec` into Codex rather than wrap it. See
    /// `codex_xats_bootstrap_command_for`: xats folds a pane's carrier matches
    /// by taking the process group leader, and only `exec` puts Codex at the
    /// leader's pid. Wrapping it leaves `sh` as the leader, xats finds no leader
    /// among the matches, and the pane silently never binds. A comment cannot
    /// stop that edit; this can.
    #[test]
    fn codex_bootstrap_execs_into_codex_rather_than_wrapping_it() {
        let cmd = codex_xats_instance().build_agent_command(None).unwrap();

        assert!(
            cmd.contains("exec codex --remote"),
            "Codex must replace the bootstrap shell, not run under it: {cmd}"
        );
    }

    /// The value AoE will not take must stop the pane, not quietly redirect it
    /// to a different server than the one xats is using.
    #[test]
    fn a_rejected_endpoint_aborts_the_pane_instead_of_falling_back() {
        let aborted = codex_xats_aborted_command("[xats] bad endpoint: ws://nope:nope");

        assert!(aborted.contains("exit 1"));
        assert!(aborted.contains("bad endpoint"));
        assert!(
            !aborted.contains("--remote") && !aborted.contains("8799"),
            "an aborted pane must not launch Codex against the default: {aborted}"
        );
    }

    /// Uses a host and port that both differ from the default, so the default
    /// leaking through anywhere is a visible absence rather than a coincidence.
    #[test]
    fn codex_bootstrap_names_the_configured_endpoint_everywhere() {
        let inst = codex_xats_instance();
        let endpoint = parse_codex_app_server_url("ws://localhost:8899").unwrap();
        let cmd =
            inst.codex_xats_bootstrap_command_for("codex", "codex", &inst.project_path, &endpoint);

        assert!(
            cmd.contains("ws://localhost:8899"),
            "remote argument: {cmd}"
        );
        assert!(cmd.contains("localhost"), "gate host: {cmd}");
        assert!(cmd.contains(" 8899 >/dev/null"), "gate port: {cmd}");
        assert!(
            cmd.contains(&codex_xats_app_server_unavailable("ws://localhost:8899")),
            "the diagnostic must name the endpoint that was probed: {cmd}"
        );
        assert!(
            !cmd.contains("8799") && !cmd.contains("127.0.0.1"),
            "no part of the default may survive a configured endpoint: {cmd}"
        );
    }

    #[test]
    fn codex_bootstrap_uses_the_target_pane_working_directory() {
        let mut inst = codex_xats_instance();
        inst.project_path = "/tmp/primary".to_string();
        let endpoint = parse_codex_app_server_url("ws://localhost:8899").unwrap();
        let cmd = inst.codex_xats_bootstrap_command_for(
            "codex",
            "codex",
            "/tmp/secondary path",
            &endpoint,
        );
        let secondary = shell_escape("/tmp/secondary path").replace('\'', "'\\''");
        let primary = shell_escape("/tmp/primary").replace('\'', "'\\''");

        assert!(cmd.contains(&format!("-C {secondary}")), "command: {cmd}");
        assert!(!cmd.contains(&format!("-C {primary}")), "command: {cmd}");
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

    #[test]
    fn identity_recovery_waits_for_ready_after_all_prompts_are_answered() {
        let answered = [
            AutoConfirmPrompt::DevelopmentChannels,
            AutoConfirmPrompt::WorkspaceTrust,
        ];

        assert!(!settles_after_answer(true, &answered));
        assert!(settles_after_answer(false, &answered));
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
        inst.run_auto_confirm("claude", IdentityKeyOrigin::Existing);

        // Also a no-op for non-claude even if the flag is set.
        inst.tool = "codex".to_string();
        inst.cross_agent_team = true;
        inst.run_auto_confirm("codex", IdentityKeyOrigin::Existing);
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
        inst.primary_pane.tool = "codex".to_string();
        inst.primary_pane.cross_agent_team = true;
        inst
    }

    fn claude_xats_instance() -> Instance {
        let mut inst = Instance::new("test", "/tmp/project path");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;
        inst.primary_pane.cross_agent_team = true;
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
            xats_runtime_generation: 0,
            yolo_mode: false,
            cross_agent_team: true,
            worktree_info: None,
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
    fn test_primary_slot_without_key_is_filled() {
        let inst = claude_xats_instance();
        assert!(inst.slot_needs_identity_key(&slot(0, "")));
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
    fn extra_pane_is_launched_with_a_freshly_minted_key() {
        let mut inst = claude_xats_instance();
        inst.ensure_xats_identity_key();
        let instance_key = inst.xats_identity_key.clone().unwrap();

        let launch = inst
            .build_extra_pane_command("claude", &inst.project_path)
            .unwrap();

        assert!(
            !launch.identity_key.is_empty(),
            "a pane AoE launches is never keyless"
        );
        assert!(
            launch
                .command
                .contains(&format!("XATS_IDENTITY_KEY='{}'", launch.identity_key)),
            "the minted key must reach the process, got: {}",
            launch.command
        );
        assert_ne!(
            launch.identity_key, instance_key,
            "two live panes behind one identity is the state recovery cannot resolve"
        );
        assert!(
            !launch.command.contains(&instance_key),
            "got: {}",
            launch.command
        );
    }

    #[test]
    #[serial_test::serial(shell_env)]
    fn extra_shell_pane_cds_to_its_own_directory_not_the_session_s() {
        let original_shell = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "/bin/zsh");

        let inst = Instance::new("test", "/tmp/session dir");
        let command = inst
            .build_extra_pane_command("shell", "/tmp/other dir")
            .unwrap()
            .command;

        assert!(
            command.contains(&shell_escape("/tmp/other dir").replace('\'', "'\\''")),
            "got: {command}"
        );
        assert!(
            !command.contains("session dir"),
            "a login shell that resets the directory must not land in the session's, got: {command}"
        );

        match original_shell {
            Some(shell) => std::env::set_var("SHELL", shell),
            None => std::env::remove_var("SHELL"),
        }
    }

    /// The CLI can now name a tool other than the session's. Building it as a
    /// non-primary pane is what keeps the instance's own launch context out of
    /// it; the tool name alone must not re-open that door.
    #[test]
    fn a_named_tool_does_not_pick_up_the_instance_s_launch_context() {
        let mut inst = claude_xats_instance();
        inst.command = "claude --instance-override".to_string();
        inst.agent_session_id = Some("pre-allocated-id".to_string());
        inst.xats_identity_key = Some("instance-key".to_string());

        let launch = inst
            .build_extra_pane_command("codex", &inst.project_path)
            .unwrap();

        assert!(
            !launch.command.contains("--instance-override"),
            "got: {}",
            launch.command
        );
        assert!(
            !launch.command.contains("pre-allocated-id"),
            "got: {}",
            launch.command
        );
        assert!(
            !launch.command.contains("instance-key"),
            "got: {}",
            launch.command
        );
        assert_ne!(launch.identity_key, "instance-key");
    }

    #[test]
    fn each_extra_pane_gets_its_own_key() {
        // Launch is the only moment at which a second pane sharing an identity is
        // preventable, so the freshness is asserted rather than assumed.
        let inst = claude_xats_instance();

        let first = inst
            .build_extra_pane_command("claude", &inst.project_path)
            .unwrap();
        let second = inst
            .build_extra_pane_command("claude", &inst.project_path)
            .unwrap();

        assert_ne!(first.identity_key, second.identity_key);
    }

    #[test]
    fn extra_pane_key_is_injected_as_env_not_argv() {
        // The key is an environment assignment the pane's shell consumes, so the
        // agent process never carries it in argv. It is still readable from the
        // pane's recorded start command, which is a property of this injection
        // route for every pane, primary included, and is out of scope here.
        let inst = codex_xats_instance();
        let launch = inst
            .build_extra_pane_command("codex", &inst.project_path)
            .unwrap();
        let marker = format!("XATS_IDENTITY_KEY='{}'", launch.identity_key);

        let argv = launch
            .command
            .split_once(&marker)
            .map(|(_, rest)| rest)
            .unwrap_or_else(|| panic!("key was not injected: {}", launch.command));
        assert!(
            !argv.contains(&launch.identity_key),
            "identity key must not also appear in the command arguments, got: {}",
            launch.command
        );
    }

    #[test]
    fn extra_pane_gets_no_key_without_cross_agent_team() {
        let mut inst = claude_xats_instance();
        inst.cross_agent_team = false;

        let launch = inst
            .build_extra_pane_command("claude", &inst.project_path)
            .unwrap();

        assert!(launch.identity_key.is_empty());
        assert!(
            !launch.command.contains("XATS_IDENTITY_KEY"),
            "got: {}",
            launch.command
        );
    }

    #[test]
    #[serial_test::serial(shell_env)]
    fn extra_shell_pane_gets_no_key() {
        // A shell runs no agent and registers no identity, so there is nothing to
        // mint a key for.
        let inst = claude_xats_instance();

        let launch = inst
            .build_extra_pane_command("shell", &inst.project_path)
            .unwrap();

        assert!(launch.identity_key.is_empty());
        assert!(
            !launch.command.contains("XATS_IDENTITY_KEY"),
            "got: {}",
            launch.command
        );
    }

    fn launched_slot(slot: i64, agent: &str, native_session_id: &str) -> crate::db::AgentSlot {
        crate::db::AgentSlot {
            instance_id: "test".to_string(),
            slot,
            agent: agent.to_string(),
            native_session_id: native_session_id.to_string(),
            cwd: "/tmp/project".to_string(),
            tmux_pane: "%1".to_string(),
            xats_identity_key: String::new(),
            xats_runtime_generation: 0,
            yolo_mode: false,
            cross_agent_team: true,
            worktree_info: None,
            last_seen_at: 1,
        }
    }

    #[test]
    fn slot_zero_without_a_native_session_id_resumes_from_the_stored_token() {
        // The launch-time record has no conversation until the first capture, and
        // the instance's scraped token is the only resume source in that window.
        let mut inst = claude_xats_instance();
        inst.resume_token = Some("stored-token".to_string());

        assert_eq!(
            inst.slot_resume_source(&launched_slot(0, "claude", ""), RestartMode::Resume),
            "stored-token"
        );
    }

    #[test]
    fn a_recorded_native_session_id_beats_the_stored_token() {
        let mut inst = claude_xats_instance();
        inst.resume_token = Some("stored-token".to_string());

        assert_eq!(
            inst.slot_resume_source(&launched_slot(0, "claude", "captured"), RestartMode::Resume),
            "captured"
        );
    }

    #[test]
    fn a_fresh_restart_ignores_the_stored_token() {
        let mut inst = claude_xats_instance();
        inst.resume_token = Some("stored-token".to_string());

        assert_eq!(
            inst.slot_resume_source(&launched_slot(0, "claude", ""), RestartMode::Fresh),
            ""
        );
    }

    #[test]
    fn no_other_slot_consults_the_instance_resume_token() {
        // The instance's token describes the instance's own pane. A second pane
        // resumed from it would land in the primary pane's conversation.
        let mut inst = claude_xats_instance();
        inst.resume_token = Some("stored-token".to_string());

        assert_eq!(
            inst.slot_resume_source(&launched_slot(1, "claude", ""), RestartMode::Resume),
            ""
        );
        assert_eq!(
            inst.slot_resume_source(&launched_slot(0, "codex", ""), RestartMode::Resume),
            "",
            "slot 0 running another agent is not the pane the token describes"
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

    /// A session whose Cross Agent Team state is set through the pane config the
    /// launch paths read, rather than the legacy mirror fields.
    fn xats_pane_instance(tool: &str) -> Instance {
        let mut inst = Instance::new("test", "/tmp/project path");
        inst.set_primary_pane_config(PaneConfig::new(tool, "/tmp/project path", false, true));
        inst
    }

    fn temp_store() -> (tempfile::TempDir, crate::db::Store) {
        let tmp = tempfile::tempdir().unwrap();
        let (store, _) = crate::db::Store::open_with_schema_at(&tmp.path().join("aoe.db")).unwrap();
        (tmp, store)
    }

    /// The distinction the reconnect decision rests on. It exists only at launch
    /// time: once the key is written, nothing on it says which launch wrote it.
    #[test]
    fn identity_key_origin_separates_the_launch_that_minted_it() {
        let mut inst = xats_pane_instance("claude");
        assert_eq!(inst.ensure_xats_identity_key(), IdentityKeyOrigin::Minted);
        assert_eq!(
            inst.ensure_xats_identity_key(),
            IdentityKeyOrigin::Existing,
            "a later launch reuses the key, which is what makes an identity \
             reclaimable"
        );
    }

    #[test]
    fn a_session_without_cross_agent_team_reports_no_identity_key() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert_eq!(inst.ensure_xats_identity_key(), IdentityKeyOrigin::Absent);
    }

    #[test]
    fn slot_identity_keys_are_minted_once_then_reported_as_existing() {
        let (_tmp, store) = temp_store();
        let inst = xats_pane_instance("claude");
        let mut slots = vec![slot(0, "")];

        let first = inst.ensure_slot_identity_keys(&store, &mut slots);
        assert_eq!(first.get(&0), Some(&IdentityKeyOrigin::Minted));
        assert!(!slots[0].xats_identity_key.is_empty());

        let second = inst.ensure_slot_identity_keys(&store, &mut slots);
        assert_eq!(second.get(&0), Some(&IdentityKeyOrigin::Existing));
    }

    /// One relaunch can mint for one pane and reuse another's, so the decision
    /// cannot be taken once for the batch. The slot that reused its key is asked
    /// to reclaim; its sibling, launching for the first time, is not.
    #[test]
    fn one_relaunch_separates_the_slot_that_reused_its_key_from_its_sibling() {
        let (_tmp, store) = temp_store();
        let inst = xats_pane_instance("claude");
        let mut slots = vec![slot(0, "key-from-an-earlier-launch"), slot(1, "")];

        let origins = inst.ensure_slot_identity_keys(&store, &mut slots);

        assert_eq!(origins.get(&0), Some(&IdentityKeyOrigin::Existing));
        assert_eq!(origins.get(&1), Some(&IdentityKeyOrigin::Minted));

        let pane = PaneConfig::new("claude", "/tmp", false, true);
        assert!(inst.reclaims_xats_identity(&pane, slot_identity_origin(&origins, 0)));
        assert!(!inst.reclaims_xats_identity(&pane, slot_identity_origin(&origins, 1)));
    }

    /// A slot the caller never reported on is not evidence that its pane owns an
    /// identity, so it is left alone rather than asked to reclaim one.
    #[test]
    fn an_unreported_slot_is_never_asked_to_reclaim_an_identity() {
        let origins = HashMap::new();
        assert_eq!(slot_identity_origin(&origins, 0), IdentityKeyOrigin::Absent);
    }

    /// A key that did not reach the store will not be there next launch, so
    /// nothing may be built on it having been this pane's. An out-of-range slot
    /// is the store's own rejection, not a stubbed one.
    #[test]
    fn a_key_that_could_not_be_persisted_is_not_reported_as_reusable() {
        let (_tmp, store) = temp_store();
        let inst = xats_pane_instance("claude");
        let rejected = crate::db::MAX_SLOT + 1;
        let mut slots = vec![slot(rejected, "")];

        let origins = inst.ensure_slot_identity_keys(&store, &mut slots);

        assert_eq!(origins.get(&rejected), Some(&IdentityKeyOrigin::Absent));
        assert!(
            slots[0].xats_identity_key.is_empty(),
            "an unpersisted key must not be handed to the launch either"
        );
        assert!(!origins[&rejected].reclaims_identity());
    }

    #[test]
    fn a_relaunched_claude_pane_reusing_its_key_reclaims_its_identity() {
        let inst = xats_pane_instance("claude");
        let pane = PaneConfig::new("claude", "/tmp", false, true);
        assert!(inst.reclaims_xats_identity(&pane, IdentityKeyOrigin::Existing));
    }

    /// Codex binds through pane pre-registration before its process starts, so it
    /// never needs asking, and text typed at it lands in its conversation.
    #[test]
    fn a_codex_pane_is_never_asked_to_reclaim_an_identity() {
        let inst = xats_pane_instance("codex");
        let pane = PaneConfig::new("codex", "/tmp", false, true);
        assert!(!inst.reclaims_xats_identity(&pane, IdentityKeyOrigin::Existing));
    }

    #[test]
    fn a_pane_without_cross_agent_team_is_never_asked_to_reclaim_an_identity() {
        let inst = xats_pane_instance("claude");
        let pane = PaneConfig::new("claude", "/tmp", false, false);
        assert!(!inst.reclaims_xats_identity(&pane, IdentityKeyOrigin::Existing));
    }

    #[test]
    fn a_sandboxed_session_is_never_asked_to_reclaim_an_identity() {
        let mut inst = xats_pane_instance("claude");
        inst.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        let pane = PaneConfig::new("claude", "/tmp", false, true);
        assert!(!inst.reclaims_xats_identity(&pane, IdentityKeyOrigin::Existing));
    }

    /// Fork and new-from-selection both start keyless, so their first launch
    /// mints. Asking either to reclaim would put two live panes behind one xats
    /// name, which is the state the daemon cannot resolve.
    #[test]
    fn a_freshly_minted_key_is_never_asked_to_reclaim_an_identity() {
        let mut inst = xats_pane_instance("claude");
        inst.ensure_xats_identity_key();
        inst.resume_token = Some("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11".to_string());

        let mut fork = inst.create_fork("forked".to_string(), None).unwrap();
        let forked = fork.ensure_xats_identity_key();
        assert_eq!(forked, IdentityKeyOrigin::Minted);
        assert!(!fork.reclaims_xats_identity(fork.primary_pane_config(), forked));

        let mut built = xats_pane_instance("claude");
        let first = built.ensure_xats_identity_key();
        assert_eq!(first, IdentityKeyOrigin::Minted);
        assert!(!built.reclaims_xats_identity(built.primary_pane_config(), first));
    }

    #[test]
    fn test_cross_agent_team_supported_tool_helpers() {
        assert!(Instance::supports_cross_agent_team_tool("claude"));
        assert!(Instance::supports_cross_agent_team_tool("codex"));
        assert!(Instance::supports_cross_agent_team_tool("opencode"));

        let mut inst = codex_xats_instance();
        assert!(inst.is_cross_agent_team());
        // A Codex instance takes Codex's integration, and takes Claude's for a
        // Claude pane adopted into it -- the instance's tool decides neither.
        assert!(inst.cross_agent_team_pane("codex"));
        assert!(inst.cross_agent_team_pane("claude"));
        assert!(inst.cross_agent_team_pane("opencode"));
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
        assert!(cmd.contains(CODEX_XATS_APP_SERVER_DEFAULT_URL));
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
        assert_codex_xats_preregister_shape(&cmd);
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
        assert_codex_xats_preregister_shape(&resume_cmd);

        let (fresh_cmd, resumed) = inst
            .build_pane_resume_plan("codex", token, true, RestartMode::Fresh, None)
            .expect("Codex fresh plan");
        assert!(!resumed);
        assert!(fresh_cmd.contains("pre-register-codex-pane"));
        assert!(!fresh_cmd.contains(&format!("resume {token}")));
        assert_codex_xats_preregister_shape(&fresh_cmd);
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
        assert_codex_xats_preregister_shape(&cmd);
    }

    /// The pre-registration call shape every launch plan must carry: the
    /// identity key delivered by naming its environment variable (the CLI
    /// reads the value itself, so it never rides any argv) plus the
    /// lengthened row TTL, under the flag names the xats CLI actually parses
    /// (`--ttl`, not a guessed spelling -- its parser ignores unknown flags
    /// silently, so a wrong name would "succeed" while changing nothing).
    fn assert_codex_xats_preregister_shape(cmd: &str) {
        assert!(
            cmd.contains("--identity-key-env XATS_IDENTITY_KEY"),
            "missing identity-key env-name flag: {cmd}"
        );
        assert!(
            !cmd.contains("--identity-key \""),
            "the key value must not ride any argv, even pre-register's: {cmd}"
        );
        assert!(
            cmd.contains(&format!("--ttl {CODEX_XATS_PREREGISTER_TTL_SECONDS}")),
            "missing pre-registration TTL: {cmd}"
        );
        assert!(
            !cmd.contains("--ttl-seconds"),
            "--ttl-seconds is not a flag the xats CLI parses: {cmd}"
        );
        assert!(
            !cmd.contains("set --"),
            "positional parameters are shared script state; the exec'ed \
             command must not be able to see pre-registration args: {cmd}"
        );
    }

    #[test]
    fn test_codex_xats_preregister_first_attempt_carries_key_and_ttl() {
        let mut inst = codex_xats_instance();
        inst.xats_identity_key = Some("secret-identity-key-value".to_string());

        let cmd = inst.build_agent_command(None).unwrap();

        assert_codex_xats_preregister_shape(&cmd);
        // The key's value appears nowhere in the bootstrap script: the script
        // names the variable and the CLI reads it from its own environment.
        // The value's one legitimate home is the env-injection prefix before
        // the outer shell.
        let script = &cmd[cmd.find("sh -c").unwrap()..];
        assert!(!script.contains("secret-identity-key-value"));
        assert!(cmd.contains("XATS_IDENTITY_KEY='secret-identity-key-value' "));

        // Two pre-registration call sites, and only two: the with-key and
        // without-key branches of the single attempt. A third would be the
        // retry that discarded the key -- see `CODEX_XATS_PREREGISTER_FAILED`.
        let calls: Vec<usize> = cmd
            .match_indices("pre-register-codex-pane")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            calls.len(),
            2,
            "expected only the key-branch and keyless-branch calls: {cmd}"
        );
        // Every call site carries the TTL, so none of them is the keyless
        // retry shape.
        let tail = &cmd[calls[0]..cmd.rfind(" exec ").unwrap()];
        assert_eq!(
            tail.matches("--ttl").count(),
            2,
            "a pre-registration without a TTL is the retry shape: {cmd}"
        );
    }

    /// How the fake npx behaves across the bootstrap's calls.
    #[derive(Clone, Copy)]
    enum FakeNpx {
        Succeed,
        FailFirst,
        FailAll,
    }

    /// Recorded invocations of a fake binary: one inner vec per call, one
    /// element per argument, boundaries preserved (an argument containing
    /// spaces or newlines stays a single element).
    type FakeCalls = Vec<Vec<String>>;

    /// Write the fake `uuidgen`/`nc`/`npx`/`codex` binaries the bootstrap
    /// script will find on PATH, recording argv into the returned log paths.
    ///
    /// Recording is NUL-framed for losslessness: each argument ends with a
    /// NUL byte and each call with a newline, so an argument containing any
    /// printable text -- `--`, spaces, newlines -- survives exactly.
    fn write_codex_bootstrap_fakes(
        bin: &std::path::Path,
        tmp: &std::path::Path,
        npx_mode: FakeNpx,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let npx_log = tmp.join("npx.log");
        let codex_log = tmp.join("codex.log");
        let write_fake = |name: &str, body: String| {
            let path = bin.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        };
        // Netstring framing: `<byte-len>:<bytes>,` per argument, `;` closing
        // each call. Unambiguous for every possible argument -- leading or
        // bare newlines, empty strings, a literal `--`.
        let record = |log: &std::path::Path| {
            format!(
                "{{ for a in \"$@\"; do                      printf '%s:' \"$(printf %s \"$a\" | wc -c | tr -d ' ')\";                      printf '%s,' \"$a\";                  done; printf ';'; }} >> {}",
                shell_escape(&log.display().to_string())
            )
        };
        write_fake(
            "uuidgen",
            "echo 12345678-1234-1234-1234-123456789abc".into(),
        );
        write_fake("nc", "exit 0".into());
        let npx_fail = match npx_mode {
            FakeNpx::Succeed => String::new(),
            // Fail exactly the first invocation, by call-counter file.
            FakeNpx::FailFirst => format!(
                "if [ ! -f {c} ]; then touch {c}; exit 1; fi",
                c = shell_escape(&tmp.join("npx.first").display().to_string())
            ),
            FakeNpx::FailAll => "exit 1".to_string(),
        };
        write_fake("npx", format!("{}\n{npx_fail}", record(&npx_log)));
        write_fake("codex", record(&codex_log));
        (npx_log, codex_log)
    }

    /// Decode a netstring-framed fake log back into per-call argv vectors.
    fn parse_fake_calls(path: &std::path::Path) -> FakeCalls {
        let bytes = std::fs::read(path).unwrap_or_default();
        let mut calls = Vec::new();
        let mut call: Vec<String> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b';' {
                calls.push(std::mem::take(&mut call));
                i += 1;
                continue;
            }
            let colon = bytes[i..].iter().position(|b| *b == b':').unwrap() + i;
            let len: usize = std::str::from_utf8(&bytes[i..colon])
                .unwrap()
                .parse()
                .unwrap();
            let start = colon + 1;
            call.push(String::from_utf8(bytes[start..start + len].to_vec()).unwrap());
            assert_eq!(bytes[start + len], b',', "malformed fake log");
            i = start + len + 1;
        }
        calls
    }

    /// Run a bootstrap script under `sh` with the fake-binary PATH and a
    /// controlled environment. `env` entries with `None` remove the variable.
    fn execute_bootstrap_script(
        script: &str,
        bin: &std::path::Path,
        identity_key: Option<&str>,
        env: &[(&str, Option<&str>)],
    ) -> std::process::Output {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg(script)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("TMUX_PANE", "%fake")
            .env_remove("SHELLOPTS");
        match identity_key {
            Some(key) => {
                cmd.env("XATS_IDENTITY_KEY", key);
            }
            None => {
                cmd.env_remove("XATS_IDENTITY_KEY");
            }
        }
        for (name, value) in env {
            match value {
                Some(v) => {
                    cmd.env(name, v);
                }
                None => {
                    cmd.env_remove(name);
                }
            }
        }
        cmd.output().unwrap()
    }

    /// Execute the real generated bootstrap script with fake binaries on PATH
    /// and assert on the argv each one actually receives. String-shape
    /// assertions cannot catch a flag the CLI does not parse or script state
    /// leaking into the exec'ed command; running the script can.
    ///
    /// `env`: extra environment for the script, on top of PATH and TMUX_PANE.
    /// A `None` value removes the variable (`SHELLOPTS`, the sentinel, and
    /// `XATS_IDENTITY_KEY` are all inheritable state a harness must control).
    /// `extra_args` lands on the Codex command line the way a user's would.
    fn run_codex_bootstrap_with_fakes(
        identity_key: Option<&str>,
        npx_mode: FakeNpx,
        env: &[(&str, Option<&str>)],
        extra_args: &str,
    ) -> (bool, FakeCalls, FakeCalls) {
        let (ok, npx, codex, _) =
            run_codex_bootstrap_capturing_stderr(identity_key, npx_mode, env, extra_args);
        (ok, npx, codex)
    }

    /// As above, plus the script's stderr, for the cases that assert on which
    /// diagnostic the pane printed rather than only on whether it failed.
    fn run_codex_bootstrap_capturing_stderr(
        identity_key: Option<&str>,
        npx_mode: FakeNpx,
        env: &[(&str, Option<&str>)],
        extra_args: &str,
    ) -> (bool, FakeCalls, FakeCalls, String) {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let (npx_log, codex_log) = write_codex_bootstrap_fakes(&bin, tmp.path(), npx_mode);

        let mut inst = codex_xats_instance();
        inst.xats_identity_key = identity_key.map(str::to_string);
        // The bootstrap `sh -c` core straight from the builder: the full pane
        // command re-escapes it inside a login-shell wrapper (which would also
        // load the developer's rc files), so it cannot be sliced back out and
        // executed. The key travels via this runner's env instead of the
        // wrapper's env prefix, which is equivalent for the script.
        let base = "codex";
        let cmd_with_args = if extra_args.is_empty() {
            base.to_string()
        } else {
            format!("{base} {extra_args}")
        };
        let script = inst.codex_xats_bootstrap_command(&cmd_with_args, base, &inst.project_path);

        let out = execute_bootstrap_script(&script, &bin, identity_key, env);
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() {
            eprintln!("bootstrap stderr: {stderr}");
        }
        (
            out.status.success(),
            parse_fake_calls(&npx_log),
            parse_fake_calls(&codex_log),
            stderr,
        )
    }

    #[test]
    fn test_codex_xats_bootstrap_executes_preregister_with_key_and_ttl() {
        let (ok, npx, codex) =
            run_codex_bootstrap_with_fakes(Some("live-key-123"), FakeNpx::Succeed, &[], "");
        assert!(ok, "bootstrap should succeed");
        assert_eq!(npx.len(), 1, "one successful pre-register call: {npx:?}");
        let flags: Vec<&str> = npx[0].iter().map(String::as_str).collect();
        let env_flag = flags
            .iter()
            .position(|a| *a == "--identity-key-env")
            .expect("daemon must be told the key's variable name");
        assert_eq!(
            flags[env_flag + 1],
            "XATS_IDENTITY_KEY",
            "the flag names the variable: {npx:?}"
        );
        let ttl_flag = flags
            .iter()
            .position(|a| *a == "--ttl")
            .expect("daemon must receive the TTL under the flag it parses");
        assert_eq!(
            flags[ttl_flag + 1],
            CODEX_XATS_PREREGISTER_TTL_SECONDS.to_string(),
            "TTL value: {npx:?}"
        );
        assert!(
            !npx[0].iter().any(|a| a.contains("live-key-123")),
            "the key value must not ride even pre-register's argv: {npx:?}"
        );
        assert!(
            !codex_argv(&codex)
                .iter()
                .any(|a| a.contains("live-key-123") || a.contains("identity-key")),
            "the key must never reach the codex argv: {codex:?}"
        );
        assert!(
            codex_argv(&codex).iter().any(|a| a == "--remote"),
            "codex must have exec'ed: {codex:?}"
        );
    }

    /// The exec'ed codex call: exactly one, argv element-precise.
    fn codex_argv(codex: &FakeCalls) -> &[String] {
        assert_eq!(codex.len(), 1, "codex must exec exactly once: {codex:?}");
        &codex[0]
    }

    #[test]
    fn test_codex_xats_bootstrap_executes_keyless_preregister() {
        let (ok, npx, codex) = run_codex_bootstrap_with_fakes(None, FakeNpx::Succeed, &[], "");
        assert!(ok);
        assert_eq!(npx.len(), 1);
        assert!(
            !npx[0].iter().any(|a| a.contains("identity-key")),
            "no key in the environment means no flag: {npx:?}"
        );
        assert!(
            npx[0].iter().any(|a| a == "--ttl"),
            "the TTL still rides: {npx:?}"
        );
        assert!(codex_argv(&codex).iter().any(|a| a == "--remote"));
    }

    /// A keyed pre-registration that fails is not retried without the key. The
    /// keyless registration such a retry produced looked healthy and left the
    /// pane permanently unrecognizable to the daemon.
    #[test]
    fn test_codex_xats_keyed_preregister_failure_is_not_retried_without_the_key() {
        let (ok, npx, codex) =
            run_codex_bootstrap_with_fakes(Some("live-key-123"), FakeNpx::FailFirst, &[], "");

        assert!(!ok, "a failed pre-registration must fail the launch");
        assert_eq!(npx.len(), 1, "exactly one attempt, no retry: {npx:?}");
        assert!(npx[0].iter().any(|a| a == "--identity-key-env"));
        assert!(codex.is_empty(), "codex must not launch: {codex:?}");
    }

    /// The reviewer's reproduction, kept because the hazard outlived the retry
    /// it was written for: `sh` imports `SHELLOPTS=errexit` from the
    /// environment, under which a plain failing command exits the script before
    /// any `$?` check. The failure must still reach its diagnostic rather than
    /// dying silently at the failing `npx`.
    #[test]
    fn test_codex_xats_preregister_failure_survives_inherited_errexit() {
        let (ok, npx, codex, stderr) = run_codex_bootstrap_capturing_stderr(
            Some("live-key-123"),
            FakeNpx::FailFirst,
            &[("SHELLOPTS", Some("errexit"))],
            "",
        );

        assert!(!ok);
        assert_eq!(npx.len(), 1, "exactly one attempt, no retry: {npx:?}");
        assert!(codex.is_empty(), "codex must not launch: {codex:?}");
        assert!(
            stderr.contains(CODEX_XATS_PREREGISTER_FAILED),
            "errexit must not skip the diagnostic: {stderr}"
        );
    }

    /// A user's extra args referencing the positional parameters must expand
    /// to nothing, the way they did before this change -- not to leftover
    /// pre-registration arguments. This is the leak path that `set --` opened.
    #[test]
    fn test_codex_xats_positional_references_in_extra_args_stay_empty() {
        let (ok, npx, codex) = run_codex_bootstrap_with_fakes(
            Some("live-key-123"),
            FakeNpx::Succeed,
            &[],
            r#""$@" "$2""#,
        );
        assert!(ok);
        assert_eq!(npx.len(), 1);
        let argv = codex_argv(&codex);
        assert!(
            !argv.iter().any(|a| a.contains("live-key-123")
                || a.contains("identity-key")
                || a.contains("--ttl")),
            "positional references must not resurrect pre-register args: {argv:?}"
        );
        // "$@" expands to nothing and "$2" to one empty argument.
        assert_eq!(
            argv.iter().filter(|a| a.is_empty()).count(),
            1,
            "expected exactly the empty expansion of \"$2\": {argv:?}"
        );
    }

    /// The failure sentinel must be script-local: an inherited environment
    /// variable of the same name must not turn a successful first attempt
    /// into a spurious bare fallback (which would overwrite the daemon's
    /// key- and TTL-carrying row with a bare one).
    #[test]
    fn test_codex_xats_success_ignores_inherited_failure_sentinel() {
        let (ok, npx, codex) = run_codex_bootstrap_with_fakes(
            Some("live-key-123"),
            FakeNpx::Succeed,
            &[("pre_register_failed", Some("1"))],
            "",
        );
        assert!(ok);
        assert_eq!(
            npx.len(),
            1,
            "a successful first attempt must not be followed by a fallback: {npx:?}"
        );
        assert!(codex_argv(&codex).iter().any(|a| a == "--remote"));
    }

    /// The fake-argv framing must be lossless for awkward arguments: a bare
    /// `--` and an empty string round-trip exactly (netstrings; a
    /// delimiter-based format broke on these). A real newline cannot reach an
    /// argument through this path at all: `environment::shell_escape`
    /// flattens newlines to literal `\n` before the script is built
    /// (pre-existing), and the recording faithfully shows that flattened
    /// form.
    #[test]
    fn test_codex_xats_fake_recording_roundtrips_awkward_args() {
        let (ok, _npx, codex) = run_codex_bootstrap_with_fakes(
            Some("live-key-123"),
            FakeNpx::Succeed,
            &[],
            "--note 'lead\ntail' -- ''",
        );
        assert!(ok);
        let argv = codex_argv(&codex);
        let tail: Vec<&str> = argv
            .iter()
            .skip_while(|a| *a != "--note")
            .map(String::as_str)
            .collect();
        assert_eq!(
            tail,
            ["--note", "lead\\ntail", "--", ""],
            "awkward args must survive recording exactly: {argv:?}"
        );
    }

    #[test]
    fn test_codex_xats_bootstrap_preregister_failure_is_fatal_without_codex() {
        let (ok, npx, codex) =
            run_codex_bootstrap_with_fakes(Some("live-key-123"), FakeNpx::FailAll, &[], "");
        assert!(!ok, "a failed pre-registration must fail the launch");
        assert_eq!(npx.len(), 1, "no second attempt: {npx:?}");
        assert!(codex.is_empty(), "codex must not launch: {codex:?}");
    }

    #[test]
    fn test_codex_xats_exec_command_carries_no_identity_material() {
        let cmd = codex_xats_instance().build_agent_command(None).unwrap();

        let execed = &cmd[cmd.rfind(" exec ").unwrap()..];
        assert!(
            !execed.contains("identity-key"),
            "argv leaks flag: {execed}"
        );
        assert!(
            !execed.contains("XATS_IDENTITY_KEY"),
            "argv leaks key reference: {execed}"
        );
        assert!(!execed.contains("ttl"), "argv leaks TTL: {execed}");
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
            &codex_xats_app_server_unavailable(CODEX_XATS_APP_SERVER_DEFAULT_URL),
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

    /// Every adopted slot is assigned its own durable identity key before AoE
    /// launches it, including slot 0.
    #[test]
    fn test_adopted_slot_zero_needs_its_own_identity_key() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;

        let own = recovered_slot(0, "claude", "/tmp/test", "%0");
        assert!(
            inst.slot_needs_identity_key(&own),
            "slot 0 stores its key in its own durable slot"
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
            xats_runtime_generation: 0,
            yolo_mode: false,
            cross_agent_team: false,
            worktree_info: None,
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
        // A legacy command equal to the registered binary still uses the managed
        // host runtime and exercises the EnvVar decoration.
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
    fn legacy_primary_pane_paths_normalize_capabilities() {
        let mut from_legacy = Instance::new("Test", "/tmp");
        from_legacy.tool = "shell".to_string();
        from_legacy.yolo_mode = true;
        from_legacy.cross_agent_team = true;
        from_legacy.sync_primary_pane_from_legacy();
        assert!(!from_legacy.primary_pane.yolo_mode);
        assert!(!from_legacy.primary_pane.cross_agent_team);
        assert!(!from_legacy.yolo_mode);
        assert!(!from_legacy.cross_agent_team);

        let mut hydrated = Instance::new("Test", "/tmp");
        hydrated.primary_pane = PaneConfig {
            tool: "shell".to_string(),
            working_dir: "/tmp".to_string(),
            yolo_mode: true,
            cross_agent_team: true,
            worktree: None,
        };
        hydrated.hydrate_legacy_primary_pane();
        assert!(!hydrated.primary_pane.yolo_mode);
        assert!(!hydrated.primary_pane.cross_agent_team);
        assert!(!hydrated.yolo_mode);
        assert!(!hydrated.cross_agent_team);
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
    fn test_create_fork_rejects_managed_host_opencode() {
        let parent = parent_instance("opencode", Some("ses_parent"));
        let error = parent
            .create_fork("unsupported".to_string(), None)
            .expect_err("managed host OpenCode fork should fail closed");
        assert!(error.to_string().contains("exact-session runtime fork"));
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
