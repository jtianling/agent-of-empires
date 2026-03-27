//! Status detection for agent sessions

use crate::session::Status;

use super::utils::strip_ansi;

const SPINNER_CHARS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn detect_status_from_title(title: &str) -> Option<Status> {
    title
        .chars()
        .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch))
        .then_some(Status::Running)
}

pub fn detect_status_from_content(content: &str, tool: &str, _fg_pid: Option<u32>) -> Status {
    // Strip ANSI escape codes before passing to detectors. capture-pane is
    // called with -e (to preserve colors for the TUI preview), but color codes
    // interspersed in text like "esc interrupt" break plain substring matches.
    let clean = strip_ansi(content);
    let status = crate::agents::get_agent(tool)
        .map(|a| (a.detect_status)(&clean))
        .unwrap_or(Status::Idle);

    if status == Status::Idle {
        let last_lines: Vec<&str> = clean.lines().rev().take(5).collect();
        tracing::debug!(
            "status detection returned Idle for tool '{}', last 5 lines: {:?}",
            tool,
            last_lines
        );
    }

    status
}

pub fn detect_claude_status(raw_content: &str) -> Status {
    let lines: Vec<&str> = raw_content.lines().collect();
    let non_empty_lines: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .copied()
        .collect();

    let last_lines: String = non_empty_lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");
    let last_lines_lower = last_lines.to_lowercase();

    // IDLE early-exit: Claude Code shows ❯ (U+276F) as its input prompt.
    // If the prompt is visible in the last few non-empty lines (ignoring
    // status bar lines), the agent is idle. Check this BEFORE tool-use
    // patterns because completed tool calls like "Bash(sleep 10)" remain
    // in scrollback and would otherwise false-positive as Running.
    let has_idle_prompt = non_empty_lines.iter().rev().take(6).any(|line| {
        let trimmed = strip_ansi(line);
        let trimmed = trimmed.trim();
        trimmed == "\u{276f}" || trimmed.starts_with("\u{276f} ")
    });

    // RUNNING: Braille spinner characters (U+2800..U+28FF) in last 5 lines.
    // Spinners override the idle prompt since they indicate active work.
    for line in non_empty_lines.iter().rev().take(5) {
        if line
            .chars()
            .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch))
        {
            return Status::Running;
        }
    }

    // If idle prompt is visible and no spinner, agent is idle -- skip
    // tool-use pattern matching which would hit completed calls in scrollback.
    if has_idle_prompt {
        return Status::Idle;
    }

    // RUNNING: Tool use output patterns in last 10 lines
    let tool_patterns = [
        "Read(",
        "Edit(",
        "Write(",
        "Bash(",
        "Grep(",
        "Glob(",
        "Agent(",
        "TodoWrite(",
        "TodoRead(",
        "Skill(",
        "ToolSearch(",
        "WebFetch(",
        "WebSearch(",
        "NotebookEdit(",
    ];
    for line in non_empty_lines.iter().rev().take(10) {
        let trimmed = strip_ansi(line);
        for pattern in &tool_patterns {
            if trimmed.contains(pattern) {
                return Status::Running;
            }
        }
    }

    // RUNNING: Progress-like patterns ("..." at end of line with content before it)
    for line in non_empty_lines.iter().rev().take(5) {
        let trimmed = strip_ansi(line);
        let trimmed = trimmed.trim();
        if trimmed.len() > 3 && (trimmed.ends_with("...") || trimmed.ends_with("…")) {
            return Status::Running;
        }
    }

    // WAITING: Permission/approval prompts with "Allow" and "Deny" choices
    if last_lines.contains("Allow") && last_lines.contains("Deny") {
        return Status::Waiting;
    }

    // WAITING: Lowercase allow/deny action buttons
    if last_lines_lower.contains("allow") && last_lines_lower.contains("deny") {
        return Status::Waiting;
    }

    // WAITING: Yes/no confirmation prompts
    let yn_prompts = ["(y/n)", "(Y/n)", "(yes/no)", "[y/n]", "[Y/n]"];
    for prompt in &yn_prompts {
        if last_lines.contains(prompt) {
            return Status::Waiting;
        }
    }

    // WAITING: Interactive numbered selection in last 10 lines
    let recent_10: Vec<&str> = non_empty_lines.iter().rev().take(10).copied().collect();
    let has_numbered_options = recent_10
        .iter()
        .filter(|line| {
            let trimmed = strip_ansi(line);
            let trimmed = trimmed.trim();
            trimmed.starts_with("1.") || trimmed.starts_with("2.") || trimmed.starts_with("3.")
        })
        .count()
        >= 2;
    if has_numbered_options {
        return Status::Waiting;
    }

    Status::Idle
}

pub fn detect_opencode_status(raw_content: &str) -> Status {
    let content = raw_content.to_lowercase();
    let lines: Vec<&str> = content.lines().collect();
    let non_empty_lines: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .copied()
        .collect();

    let last_lines: String = non_empty_lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");
    let last_lines_lower = last_lines.to_lowercase();

    // RUNNING: OpenCode shows "esc to interrupt" when busy (same as Claude Code)
    // Only check in last lines to avoid matching comments/code in terminal output
    if last_lines_lower.contains("esc to interrupt") || last_lines_lower.contains("esc interrupt") {
        return Status::Running;
    }

    for line in &lines {
        for spinner in SPINNER_CHARS {
            if line.contains(spinner) {
                return Status::Running;
            }
        }
    }

    // WAITING: Selection menus (shows "Enter to select" or "Esc to cancel")
    // Only check in last lines to avoid matching comments/code
    if last_lines_lower.contains("enter to select") || last_lines_lower.contains("esc to cancel") {
        return Status::Waiting;
    }

    // WAITING: Permission/confirmation prompts
    // Only check in last lines
    let permission_prompts = [
        "(y/n)",
        "[y/n]",
        "continue?",
        "proceed?",
        "approve",
        "allow",
    ];
    for prompt in &permission_prompts {
        if last_lines_lower.contains(prompt) {
            return Status::Waiting;
        }
    }

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("❯") && trimmed.len() > 2 {
            let after_cursor = trimmed.get(3..).unwrap_or("").trim_start();
            if after_cursor.starts_with("1.")
                || after_cursor.starts_with("2.")
                || after_cursor.starts_with("3.")
            {
                return Status::Waiting;
            }
        }
    }
    if lines.iter().any(|line| {
        line.contains("❯") && (line.contains(" 1.") || line.contains(" 2.") || line.contains(" 3."))
    }) {
        return Status::Waiting;
    }

    for line in non_empty_lines.iter().rev().take(10) {
        let clean_line = strip_ansi(line).trim().to_string();

        if clean_line == ">" || clean_line == "> " || clean_line == ">>" {
            return Status::Waiting;
        }
        if clean_line.starts_with("> ")
            && !clean_line.to_lowercase().contains("esc")
            && clean_line.len() < 100
        {
            return Status::Waiting;
        }
    }

    // WAITING - Completion indicators + input prompt nearby
    // Only check in last lines
    let completion_indicators = [
        "complete",
        "done",
        "finished",
        "ready",
        "what would you like",
        "what else",
        "anything else",
        "how can i help",
        "let me know",
    ];
    let has_completion = completion_indicators
        .iter()
        .any(|ind| last_lines_lower.contains(ind));
    if has_completion {
        for line in non_empty_lines.iter().rev().take(10) {
            let clean = strip_ansi(line).trim().to_string();
            if clean == ">" || clean == "> " || clean == ">>" {
                return Status::Waiting;
            }
        }
    }

    Status::Idle
}

pub fn detect_vibe_status(raw_content: &str) -> Status {
    let content = raw_content.to_lowercase();
    let lines: Vec<&str> = content.lines().collect();
    let non_empty_lines: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .copied()
        .collect();

    let last_lines: String = non_empty_lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");
    let last_lines_lower = last_lines.to_lowercase();

    // Vibe uses Textual TUI which can render text vertically (one char per line).
    // Join recent single-char lines to reconstruct words for detection.
    let recent_text: String = non_empty_lines
        .iter()
        .rev()
        .take(50)
        .rev()
        .map(|l| l.trim())
        .collect::<Vec<&str>>()
        .join("");
    let recent_text_lower = recent_text.to_lowercase();

    // WAITING checks come first - they're more specific than Running indicators

    // WAITING: Vibe's approval prompts show navigation hints
    // Pattern: "↑↓ navigate  Enter select  ESC reject"
    if last_lines_lower.contains("↑↓ navigate")
        || last_lines_lower.contains("enter select")
        || last_lines_lower.contains("esc reject")
    {
        return Status::Waiting;
    }

    // WAITING: Tool approval warning (shows "⚠ {tool_name} command")
    if last_lines.contains("⚠") && last_lines_lower.contains("command") {
        return Status::Waiting;
    }

    // WAITING: Approval options shown by Vibe
    let approval_options = [
        "yes and always allow",
        "no and tell the agent",
        "› 1.", // Selected numbered option
        "› 2.",
        "› 3.",
    ];
    for option in &approval_options {
        if last_lines_lower.contains(option) {
            return Status::Waiting;
        }
    }

    // WAITING: Generic selection cursor (› followed by text)
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("›") && trimmed.len() > 2 {
            return Status::Waiting;
        }
    }

    // RUNNING: Check for braille spinners anywhere in recent content
    // Vibe renders vertically so spinner may be on its own line
    for spinner in SPINNER_CHARS {
        if recent_text.contains(spinner) {
            return Status::Running;
        }
    }

    // RUNNING: Activity indicators (may be rendered vertically)
    let activity_indicators = [
        "running",
        "reading",
        "writing",
        "executing",
        "processing",
        "generating",
        "thinking",
    ];
    for indicator in &activity_indicators {
        if recent_text_lower.contains(indicator) {
            return Status::Running;
        }
    }

    // RUNNING: Ellipsis at end often indicates ongoing activity
    if recent_text.ends_with("…") || recent_text.ends_with("...") {
        return Status::Running;
    }

    Status::Idle
}

pub fn detect_codex_status(raw_content: &str) -> Status {
    let lines: Vec<&str> = raw_content.lines().collect();
    let non_empty_lines: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .copied()
        .collect();

    let last_lines: String = non_empty_lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");
    let last_lines_lower = last_lines.to_lowercase();

    // IDLE early-exit: Codex uses › (U+203A) as its input prompt.
    // If the prompt is visible in the last few lines, Codex is idle.
    // Check this BEFORE bullet spinner patterns because completed
    // Codex responses use • as a bullet prefix in scrollback.
    let has_idle_prompt = non_empty_lines.iter().rev().take(6).any(|line| {
        let trimmed = strip_ansi(line);
        let trimmed = trimmed.trim();
        trimmed == "\u{203a}" || trimmed.starts_with("\u{203a} ")
    });

    // RUNNING: Codex shows "• Working (Xs • esc to interrupt)" while processing.
    // "esc to interrupt" is a definitive Running signal even with idle prompt.
    if last_lines_lower.contains("esc to interrupt") {
        return Status::Running;
    }

    // If idle prompt is visible and no "esc to interrupt", skip bullet spinner
    // check which would false-positive on completed response bullets in scrollback.
    if !has_idle_prompt {
        // Codex uses • (U+2022) / ◦ (U+25E6) as its spinner, not braille chars.
        for line in non_empty_lines.iter().rev().take(10) {
            let trimmed = strip_ansi(line);
            let trimmed = trimmed.trim();
            if trimmed.starts_with('\u{2022}') || trimmed.starts_with('\u{25E6}') {
                return Status::Running;
            }
        }
    }

    // WAITING: Approval dialogs -- "Press enter to confirm or esc to cancel"
    if last_lines_lower.contains("press enter to confirm") {
        return Status::Waiting;
    }

    // WAITING: Approval selection list -- lines like "› 1. Yes, proceed (y)"
    for line in non_empty_lines.iter().rev().take(15) {
        let trimmed = strip_ansi(line);
        let trimmed = trimmed.trim();
        if trimmed.starts_with('\u{203a}') {
            let after = trimmed.get(3..).unwrap_or("").trim_start();
            if after.starts_with("1.") || after.starts_with("2.") || after.starts_with("3.") {
                return Status::Waiting;
            }
        }
    }

    Status::Idle
}

/// Cursor agent status is detected via hooks (file-based), same as Claude Code.
pub fn detect_cursor_status(_content: &str) -> Status {
    Status::Idle
}

/// Terminal sessions are plain shells -- no meaningful status to detect.
pub fn detect_terminal_status(_content: &str) -> Status {
    Status::Idle
}

/// Aggregate statuses from multiple panes. Priority: Waiting > Running > Idle.
pub fn aggregate_pane_statuses(statuses: &[Status]) -> Status {
    if statuses.contains(&Status::Waiting) {
        return Status::Waiting;
    }
    if statuses.contains(&Status::Running) {
        return Status::Running;
    }
    Status::Idle
}

const SHELL_COMMANDS: &[&str] = &[
    "bash", "zsh", "fish", "sh", "dash", "ksh", "tcsh", "csh", "nu", "elvish",
];

pub fn is_shell_command(cmd: &str) -> bool {
    let cmd_lower = cmd.to_lowercase();
    let basename = cmd_lower.rsplit('/').next().unwrap_or(&cmd_lower);
    SHELL_COMMANDS.contains(&basename)
}

pub fn detect_agent_type_from_command(pane_current_command: &str) -> Option<&'static str> {
    if pane_current_command.is_empty() {
        return None;
    }

    let cmd_lower = pane_current_command.to_lowercase();

    if is_shell_command(&cmd_lower) {
        return Some("shell");
    }

    if cmd_lower.contains("claude") {
        return Some("claude");
    }
    if cmd_lower.contains("codex") {
        return Some("codex");
    }
    if cmd_lower.contains("gemini") {
        return Some("gemini");
    }
    if cmd_lower.contains("opencode") || cmd_lower.contains("open-code") {
        return Some("opencode");
    }
    if cmd_lower.contains("vibe") {
        return Some("vibe");
    }
    if cmd_lower == "agent" {
        return Some("cursor");
    }
    if cmd_lower.contains("copilot") {
        return Some("copilot");
    }

    None
}

/// Detect which agent type is running in a pane.
/// Returns the agent name (e.g., "claude", "codex") or "shell" for shell panes.
/// Returns None if the pane type cannot be determined.
pub fn detect_agent_type_from_pane(pane_info: &crate::tmux::PaneInfo) -> Option<&'static str> {
    if let Some(agent) = detect_agent_type_from_command(&pane_info.current_command) {
        return Some(agent);
    }

    if let Some(pid) = pane_info.pane_pid {
        if let Some(comm) = crate::process::get_process_comm(pid) {
            if let Some(agent) = detect_agent_type_from_command(&comm) {
                if agent != "shell" {
                    return Some(agent);
                }
            }

            if is_shell_command(&comm) {
                if let Some(fg_pid) = crate::process::get_foreground_pid(pid) {
                    if fg_pid != pid {
                        if let Some(fg_comm) = crate::process::get_process_comm(fg_pid) {
                            if let Some(agent) = detect_agent_type_from_command(&fg_comm) {
                                return Some(agent);
                            }
                        }
                    }
                }
                return Some("shell");
            }
        }
    }

    None
}

/// Copilot CLI status detection via tmux pane parsing.
/// Copilot CLI is a full-screen TUI. It shows "Thinking" while the model is
/// processing and displays tool approval prompts when actions need confirmation.
pub fn detect_copilot_status(raw_content: &str) -> Status {
    let content = raw_content.to_lowercase();
    let lines: Vec<&str> = content.lines().collect();
    let non_empty_lines: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .copied()
        .collect();

    let last_lines: String = non_empty_lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");
    let last_lines_lower = last_lines.to_lowercase();

    // RUNNING: Copilot shows spinners and "Thinking" while the model is processing
    for line in &lines {
        for spinner in SPINNER_CHARS {
            if line.contains(spinner) {
                return Status::Running;
            }
        }
    }

    if last_lines_lower.contains("thinking")
        || last_lines_lower.contains("working")
        || last_lines_lower.contains("esc to interrupt")
        || last_lines_lower.contains("ctrl+c to interrupt")
    {
        return Status::Running;
    }

    // WAITING: Tool approval prompts
    let approval_prompts = [
        "approve",
        "allow",
        "(y/n)",
        "[y/n]",
        "continue?",
        "run command?",
        "allow this tool",
        "approve for the rest",
    ];
    for prompt in &approval_prompts {
        if last_lines_lower.contains(prompt) {
            return Status::Waiting;
        }
    }

    // WAITING: Selection menus
    if last_lines_lower.contains("enter to select") || last_lines_lower.contains("esc to cancel") {
        return Status::Waiting;
    }

    // WAITING: Input prompt ready
    for line in non_empty_lines.iter().rev().take(10) {
        let clean_line = strip_ansi(line).trim().to_string();
        if clean_line == ">" || clean_line == "> " || clean_line == "copilot>" {
            return Status::Waiting;
        }
        if clean_line.starts_with("> ")
            && !clean_line.to_lowercase().contains("esc")
            && clean_line.len() < 100
        {
            return Status::Waiting;
        }
    }

    Status::Idle
}

/// Pi coding agent status detection via tmux pane parsing.
/// Pi always auto-approves tool use (no approval gates), so we only detect
/// Running vs Idle/Waiting-for-input states.
pub fn detect_pi_status(raw_content: &str) -> Status {
    let content = raw_content.to_lowercase();
    let lines: Vec<&str> = content.lines().collect();
    let non_empty_lines: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .copied()
        .collect();

    let last_lines: String = non_empty_lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");
    let last_lines_lower = last_lines.to_lowercase();

    // RUNNING: Pi shows spinners and activity indicators
    for line in &lines {
        for spinner in SPINNER_CHARS {
            if line.contains(spinner) {
                return Status::Running;
            }
        }
    }

    if last_lines_lower.contains("esc to interrupt")
        || last_lines_lower.contains("ctrl+c to interrupt")
    {
        return Status::Running;
    }

    // WAITING: Check for input prompt before activity indicators, since words
    // like "reading" or "writing" can linger in scrollback after the agent
    // finishes and shows a prompt.
    for line in non_empty_lines.iter().rev().take(5) {
        let clean_line = strip_ansi(line).trim().to_string();
        if clean_line == ">" || clean_line == "> " || clean_line == "pi>" {
            return Status::Waiting;
        }
        if clean_line.starts_with("> ")
            && !clean_line.to_lowercase().contains("esc")
            && clean_line.len() < 100
        {
            return Status::Waiting;
        }
    }

    // RUNNING: Activity indicators in the last few lines
    let activity_indicators = ["thinking", "working", "reading", "writing", "executing"];
    for indicator in &activity_indicators {
        if last_lines_lower.contains(indicator) {
            return Status::Running;
        }
    }

    Status::Idle
}

#[cfg(test)]
mod title_tests {
    use super::*;

    #[test]
    fn test_detect_status_from_title_with_spinner() {
        assert_eq!(
            detect_status_from_title("⠙ codex working"),
            Some(Status::Running)
        );
        assert_eq!(
            detect_status_from_title("agent ⠋ compiling"),
            Some(Status::Running)
        );
    }

    #[test]
    fn test_detect_status_from_title_without_spinner() {
        assert_eq!(detect_status_from_title("codex idle"), None);
        assert_eq!(detect_status_from_title("done ✓"), None);
    }

    #[test]
    fn test_detect_status_from_title_golden_fixtures() {
        let running = include_str!("../../tests/fixtures/title_fast_path/running_title.txt");
        let non_running =
            include_str!("../../tests/fixtures/title_fast_path/non_running_title.txt");

        assert_eq!(
            detect_status_from_title(running.trim()),
            Some(Status::Running)
        );
        assert_eq!(detect_status_from_title(non_running.trim()), None);
    }
}

pub fn detect_gemini_status(raw_content: &str) -> Status {
    let content = raw_content.to_lowercase();
    let lines: Vec<&str> = content.lines().collect();
    let non_empty_lines: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .copied()
        .collect();

    let last_lines: String = non_empty_lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");
    let last_lines_lower = last_lines.to_lowercase();

    // RUNNING: Gemini shows activity indicators
    if last_lines_lower.contains("esc to interrupt")
        || last_lines_lower.contains("ctrl+c to interrupt")
    {
        return Status::Running;
    }

    for line in &lines {
        for spinner in SPINNER_CHARS {
            if line.contains(spinner) {
                return Status::Running;
            }
        }
    }

    // WAITING: Approval prompts
    let approval_prompts = [
        "(y/n)",
        "[y/n]",
        "allow",
        "approve",
        "execute?",
        "enter to select",
        "esc to cancel",
    ];
    for prompt in &approval_prompts {
        if last_lines_lower.contains(prompt) {
            return Status::Waiting;
        }
    }

    // WAITING: Input prompt
    for line in non_empty_lines.iter().rev().take(10) {
        let clean_line = strip_ansi(line).trim().to_string();
        if clean_line == ">" || clean_line == "> " {
            return Status::Waiting;
        }
    }

    Status::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_cursor_terminal_status_is_stub() {
        assert_eq!(detect_cursor_status("anything"), Status::Idle);
        assert_eq!(detect_terminal_status("anything"), Status::Idle);
    }

    #[test]
    fn test_detect_claude_status_running_spinner() {
        let content = "Some output\n\u{280b} Working on task...\n";
        assert_eq!(detect_claude_status(content), Status::Running);
    }

    #[test]
    fn test_detect_claude_status_running_tool_use() {
        let content = "Let me read that file.\n\n  Read(/Users/foo/bar.rs)\n\n";
        assert_eq!(detect_claude_status(content), Status::Running);

        let content = "I'll edit this.\n\n  Edit(/src/main.rs)\n\n";
        assert_eq!(detect_claude_status(content), Status::Running);

        let content = "Running a command.\n\n  Bash(cargo test)\n\n";
        assert_eq!(detect_claude_status(content), Status::Running);

        let content = "Searching files.\n\n  Grep(pattern)\n\n";
        assert_eq!(detect_claude_status(content), Status::Running);

        let content = "Finding files.\n\n  Glob(**/*.rs)\n\n";
        assert_eq!(detect_claude_status(content), Status::Running);
    }

    #[test]
    fn test_detect_claude_status_running_progress() {
        let content = "Analyzing the codebase...\n";
        assert_eq!(detect_claude_status(content), Status::Running);

        let content = "Generating response\u{2026}\n";
        assert_eq!(detect_claude_status(content), Status::Running);
    }

    #[test]
    fn test_detect_claude_status_waiting_permission() {
        let content = "Claude wants to run a command:\n\n  bash: rm -rf /tmp/test\n\nAllow  Deny\n";
        assert_eq!(detect_claude_status(content), Status::Waiting);
    }

    #[test]
    fn test_detect_claude_status_waiting_yn_prompt() {
        let content = "Do you want to proceed? (y/n)\n";
        assert_eq!(detect_claude_status(content), Status::Waiting);

        let content = "Continue? (Y/n)\n";
        assert_eq!(detect_claude_status(content), Status::Waiting);

        let content = "Overwrite file? (yes/no)\n";
        assert_eq!(detect_claude_status(content), Status::Waiting);
    }

    #[test]
    fn test_detect_claude_status_waiting_numbered_selection() {
        let content = "Select an option:\n1. Create new file\n2. Edit existing\n3. Cancel\n";
        assert_eq!(detect_claude_status(content), Status::Waiting);
    }

    #[test]
    fn test_detect_claude_status_idle() {
        let content = "Done! The file has been updated.\n\n> \n";
        assert_eq!(detect_claude_status(content), Status::Idle);
    }

    #[test]
    fn test_detect_claude_status_idle_with_prompt() {
        // Claude Code's ❯ prompt means idle
        let content = "Done.\n\n\u{276f} \n";
        assert_eq!(detect_claude_status(content), Status::Idle);
    }

    #[test]
    fn test_detect_claude_status_idle_prompt_overrides_historical_tool_calls() {
        // Completed tool calls in scrollback should NOT trigger Running
        // when the idle prompt is visible
        let content =
            "\u{23fa} Bash(sleep 10)\n  \u{23bf}  (No output)\n\n\u{23fa} Done.\n\n\u{276f} \n";
        assert_eq!(detect_claude_status(content), Status::Idle);
    }

    #[test]
    fn test_detect_claude_status_idle_real_world_scrollback() {
        // Real-world case: multiple completed Bash() calls in scrollback with idle prompt
        let content = "\u{23fa} Bash(sleep 10)\n  \u{23bf}  (No output)\n\n\u{23fa} Done.\n\n\u{276f} sleep 10\n\n\u{23fa} Bash(sleep 10)\n  \u{23bf}  (No output)\n\n\u{23fa} Done.\n\n\u{276f} \n───\n  Opus 4.6 (1M context)\n  \u{23f5}\u{23f5} bypass permissions\n";
        assert_eq!(detect_claude_status(content), Status::Idle);
    }

    #[test]
    fn test_detect_claude_status_idle_plain_output() {
        let content = "file saved successfully";
        assert_eq!(detect_claude_status(content), Status::Idle);
    }

    #[test]
    fn test_detect_claude_status_running_spinner_overrides_idle_prompt() {
        // Spinner should still win even if prompt is visible (edge case during transition)
        let content = "\u{276f} \n\u{280b} Working on task...\n";
        assert_eq!(detect_claude_status(content), Status::Running);
    }

    #[test]
    fn test_detect_claude_status_running_beats_waiting() {
        // Spinner present alongside allow/deny -- Running wins
        let content = "Processing...\n\u{280b} Working\nAllow  Deny\n";
        assert_eq!(detect_claude_status(content), Status::Running);
    }

    #[test]
    fn test_detect_status_from_content_unknown_tool_returns_idle() {
        let status = detect_status_from_content("Processing ⠋", "unknown_tool", None);
        assert_eq!(status, Status::Idle);
    }

    #[test]
    fn test_detect_status_strips_ansi_before_matching() {
        // capture-pane -e injects ANSI color codes between characters, which
        // can split signal strings like "esc interrupt" so they no longer match
        // as plain substrings. The dispatcher must strip ANSI before calling
        // any agent detector.
        let ansi_running =
            "\x1b[38;2;39;62;94m⬝⬝⬝⬝⬝⬝⬝⬝\x1b[0m  \x1b[38;2;238;238;238mesc \x1b[38;2;128;128;128minterrupt\x1b[0m";
        assert_eq!(
            detect_status_from_content(ansi_running, "opencode", None),
            Status::Running,
            "ANSI codes around 'esc interrupt' should not prevent Running detection"
        );

        let ansi_spinner = "\x1b[38;2;255;255;255m⠋\x1b[0m generating";
        assert_eq!(
            detect_status_from_content(ansi_spinner, "opencode", None),
            Status::Running,
            "ANSI codes around spinner chars should not prevent Running detection"
        );
    }

    #[test]
    fn test_detect_opencode_status_running() {
        assert_eq!(
            detect_opencode_status("Processing your request\nesc to interrupt"),
            Status::Running
        );
        assert_eq!(
            detect_opencode_status("Working... esc interrupt"),
            Status::Running
        );
        assert_eq!(detect_opencode_status("Generating ⠋"), Status::Running);
        assert_eq!(detect_opencode_status("Loading ⠹"), Status::Running);
    }

    #[test]
    fn test_detect_opencode_status_waiting() {
        assert_eq!(
            detect_opencode_status("allow this action? [y/n]"),
            Status::Waiting
        );
        assert_eq!(detect_opencode_status("continue? (y/n)"), Status::Waiting);
        assert_eq!(detect_opencode_status("approve changes"), Status::Waiting);
        assert_eq!(detect_opencode_status("task complete.\n>"), Status::Waiting);
        assert_eq!(
            detect_opencode_status("ready for input\n> "),
            Status::Waiting
        );
        assert_eq!(
            detect_opencode_status("done! what else can i help with?\n>"),
            Status::Waiting
        );
    }

    #[test]
    fn test_detect_opencode_status_idle() {
        assert_eq!(detect_opencode_status("some random output"), Status::Idle);
        assert_eq!(
            detect_opencode_status("file saved successfully"),
            Status::Idle
        );
    }

    #[test]
    fn test_detect_opencode_status_numbered_selection() {
        let content = "Select:\n❯ 1. Option A\n  2. Option B";
        assert_eq!(detect_opencode_status(content), Status::Waiting);
    }

    #[test]
    fn test_detect_opencode_status_completion_with_prompt() {
        let content = "Task complete! What else can I help with?\n>";
        assert_eq!(detect_opencode_status(content), Status::Waiting);
    }

    #[test]
    fn test_detect_opencode_status_double_prompt() {
        assert_eq!(detect_opencode_status("Ready\n>>"), Status::Waiting);
    }

    #[test]
    fn test_detect_vibe_status_running() {
        // Braille spinners
        assert_eq!(detect_vibe_status("processing ⠋"), Status::Running);
        assert_eq!(detect_vibe_status("⠹"), Status::Running);

        // Activity indicators
        assert_eq!(detect_vibe_status("Running bash"), Status::Running);
        assert_eq!(detect_vibe_status("Reading file"), Status::Running);
        assert_eq!(detect_vibe_status("Writing changes"), Status::Running);
        assert_eq!(detect_vibe_status("Generating code"), Status::Running);

        // Vertical text (Vibe's Textual TUI renders one char per line)
        assert_eq!(
            detect_vibe_status("⠋\nR\nu\nn\nn\ni\nn\ng\nb\na\ns\nh\n…"),
            Status::Running
        );

        // Ellipsis indicates ongoing activity
        assert_eq!(detect_vibe_status("Working…"), Status::Running);
        assert_eq!(detect_vibe_status("Loading..."), Status::Running);
    }

    #[test]
    fn test_detect_vibe_status_waiting() {
        // Vibe's approval prompt navigation hints
        assert_eq!(
            detect_vibe_status("↑↓ navigate  Enter select  ESC reject"),
            Status::Waiting
        );
        // Tool approval warning
        assert_eq!(
            detect_vibe_status("⚠ bash command\nExecute this?"),
            Status::Waiting
        );
        // Approval options
        assert_eq!(
            detect_vibe_status(
                "› Yes\n  Yes and always allow bash for this session\n  No and tell the agent"
            ),
            Status::Waiting
        );
    }

    #[test]
    fn test_detect_vibe_status_idle() {
        assert_eq!(detect_vibe_status("some random output"), Status::Idle);
        assert_eq!(detect_vibe_status("file saved successfully"), Status::Idle);
        assert_eq!(detect_vibe_status("Done!"), Status::Idle);
    }

    #[test]
    fn test_detect_codex_status_running() {
        // "esc to interrupt" is the primary running indicator
        assert_eq!(
            detect_codex_status("\u{2022} Working (5s \u{2022} esc to interrupt)"),
            Status::Running
        );
        assert_eq!(
            detect_codex_status("processing request\nesc to interrupt"),
            Status::Running
        );
        // Bullet spinner (U+2022) at line start
        assert_eq!(
            detect_codex_status("\u{2022} Working (12s)"),
            Status::Running
        );
        // Hollow bullet spinner (U+25E6) at line start
        assert_eq!(
            detect_codex_status("\u{25e6} Working (3s)"),
            Status::Running
        );
    }

    #[test]
    fn test_detect_codex_status_waiting() {
        // Approval dialog with numbered selection
        assert_eq!(
            detect_codex_status(
                "Would you like to run the following command?\n\u{203a} 1. Yes, proceed (y)\n  2. No (esc)\n\nPress enter to confirm or esc to cancel"
            ),
            Status::Waiting
        );
        // "Press enter to confirm" alone
        assert_eq!(
            detect_codex_status("some dialog\nPress enter to confirm or esc to cancel"),
            Status::Waiting
        );
    }

    #[test]
    fn test_detect_codex_status_idle_at_prompt() {
        // Bare › prompt is Codex's idle input prompt, not Waiting
        assert_eq!(
            detect_codex_status("some output\n\u{203a} Ask Codex to do anything"),
            Status::Idle
        );
        assert_eq!(detect_codex_status("done!\n\u{203a} "), Status::Idle);
    }

    #[test]
    fn test_detect_codex_status_idle_prompt_overrides_bullet_scrollback() {
        // Completed Codex responses use • as bullet prefix. When the idle
        // prompt › is visible, • in scrollback should NOT trigger Running.
        let content = "\u{2022} Hello. What do you need help with?\n\n\n\u{203a} Explain this codebase\n\n  gpt-5.4 high fast\n";
        assert_eq!(detect_codex_status(content), Status::Idle);
    }

    #[test]
    fn test_detect_codex_status_idle() {
        assert_eq!(detect_codex_status("file saved"), Status::Idle);
        assert_eq!(detect_codex_status("random output text"), Status::Idle);
    }

    #[test]
    fn test_detect_codex_status_running_beats_prompt() {
        // When both spinner and › prompt are visible, Running wins
        assert_eq!(
            detect_codex_status(
                "\u{2022} Working (5s \u{2022} esc to interrupt)\n  \u{2514} Reading file\n\u{203a} "
            ),
            Status::Running
        );
    }

    #[test]
    fn test_detect_gemini_status_running() {
        assert_eq!(
            detect_gemini_status("processing request\nesc to interrupt"),
            Status::Running
        );
        assert_eq!(detect_gemini_status("generating ⠋"), Status::Running);
        assert_eq!(detect_gemini_status("working ⠹"), Status::Running);
    }

    #[test]
    fn test_detect_gemini_status_waiting() {
        assert_eq!(
            detect_gemini_status("run this command? (y/n)"),
            Status::Waiting
        );
        assert_eq!(detect_gemini_status("approve changes?"), Status::Waiting);
        assert_eq!(
            detect_gemini_status("execute this action? [y/n]"),
            Status::Waiting
        );
        assert_eq!(detect_gemini_status("ready\n>"), Status::Waiting);
    }

    #[test]
    fn test_detect_gemini_status_idle() {
        assert_eq!(detect_gemini_status("file saved"), Status::Idle);
        assert_eq!(detect_gemini_status("random output text"), Status::Idle);
    }

    #[test]
    fn test_is_shell_command() {
        assert!(is_shell_command("zsh"));
        assert!(is_shell_command("bash"));
        assert!(is_shell_command("fish"));
        assert!(is_shell_command("/bin/zsh"));
        assert!(is_shell_command("/usr/local/bin/fish"));
        assert!(!is_shell_command("claude"));
        assert!(!is_shell_command("codex"));
        assert!(!is_shell_command("2.1.81"));
    }

    #[test]
    fn test_detect_agent_type_from_command() {
        assert_eq!(detect_agent_type_from_command("claude"), Some("claude"));
        assert_eq!(
            detect_agent_type_from_command("codex-aarch64-apple-darwin"),
            Some("codex")
        );
        assert_eq!(detect_agent_type_from_command("gemini"), Some("gemini"));
        assert_eq!(detect_agent_type_from_command("opencode"), Some("opencode"));
        assert_eq!(detect_agent_type_from_command("vibe"), Some("vibe"));
        assert_eq!(detect_agent_type_from_command("agent"), Some("cursor"));
        assert_eq!(detect_agent_type_from_command("copilot"), Some("copilot"));
        assert_eq!(detect_agent_type_from_command("zsh"), Some("shell"));
        assert_eq!(detect_agent_type_from_command("bash"), Some("shell"));
        assert_eq!(detect_agent_type_from_command("2.1.81"), None);
        assert_eq!(detect_agent_type_from_command(""), None);
    }

    #[test]
    fn test_detect_copilot_status_running() {
        assert_eq!(
            detect_copilot_status("processing request\nesc to interrupt"),
            Status::Running
        );
        assert_eq!(
            detect_copilot_status("Thinking about your request"),
            Status::Running
        );
        assert_eq!(detect_copilot_status("working ⠋"), Status::Running);
        assert_eq!(detect_copilot_status("loading ⠹"), Status::Running);
    }

    #[test]
    fn test_detect_copilot_status_waiting() {
        assert_eq!(detect_copilot_status("run command? (y/n)"), Status::Waiting);
        assert_eq!(
            detect_copilot_status("Allow this tool to run?"),
            Status::Waiting
        );
        assert_eq!(
            detect_copilot_status("pick an option\nenter to select"),
            Status::Waiting
        );
        assert_eq!(detect_copilot_status("done\n>"), Status::Waiting);
        assert_eq!(detect_copilot_status("done\ncopilot>"), Status::Waiting);
    }

    #[test]
    fn test_detect_copilot_status_idle() {
        assert_eq!(detect_copilot_status("file saved"), Status::Idle);
        assert_eq!(detect_copilot_status("random output text"), Status::Idle);
    }

    #[test]
    fn test_detect_pi_status_running() {
        assert_eq!(detect_pi_status("generating ⠋"), Status::Running);
        assert_eq!(detect_pi_status("loading ⠹"), Status::Running);
        assert_eq!(
            detect_pi_status("processing request\nesc to interrupt"),
            Status::Running
        );
        assert_eq!(detect_pi_status("thinking about code"), Status::Running);
        assert_eq!(detect_pi_status("reading file.ts"), Status::Running);
    }

    #[test]
    fn test_detect_pi_status_waiting() {
        assert_eq!(detect_pi_status("done\n>"), Status::Waiting);
        assert_eq!(detect_pi_status("ready\n> "), Status::Waiting);
        assert_eq!(detect_pi_status("complete\npi>"), Status::Waiting);
        // Prompt takes priority over activity words lingering in scrollback
        assert_eq!(
            detect_pi_status("reading config.toml\nDone.\n>"),
            Status::Waiting
        );
    }

    #[test]
    fn test_detect_pi_status_idle() {
        assert_eq!(detect_pi_status("file saved"), Status::Idle);
        assert_eq!(detect_pi_status("random output text"), Status::Idle);
    }
}
