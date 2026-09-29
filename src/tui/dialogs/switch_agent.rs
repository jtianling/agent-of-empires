//! Switch-agent dialog.
//!
//! Opens on `a`/`A` from the home view for the selected session. Picks the
//! agent the session is handed to; the switch discards the conversation and
//! keeps the session's xats identity.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::DialogResult;
use crate::tui::styles::Theme;

pub struct SwitchAgentDialog {
    session_id: String,
    session_title: String,
    current: String,
    tools: Vec<&'static str>,
    tool_index: usize,
}

impl SwitchAgentDialog {
    pub fn new(session_id: &str, session_title: &str, current: &str) -> Self {
        let tools = crate::agents::SWITCHABLE_AGENTS.to_vec();
        let tool_index = tools.iter().position(|&t| t != current).unwrap_or(0);
        Self {
            session_id: session_id.to_string(),
            session_title: session_title.to_string(),
            current: current.to_string(),
            tools,
            tool_index,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Submits the chosen agent; choosing the current one cancels.
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<String> {
        let len = self.tools.len().max(1);
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => {
                let target = self.tools[self.tool_index];
                if target == self.current {
                    DialogResult::Cancel
                } else {
                    DialogResult::Submit(target.to_string())
                }
            }
            KeyCode::Left | KeyCode::BackTab => {
                self.tool_index = (self.tool_index + len - 1) % len;
                DialogResult::Continue
            }
            KeyCode::Right | KeyCode::Tab | KeyCode::Char(' ') => {
                self.tool_index = (self.tool_index + 1) % len;
                DialogResult::Continue
            }
            _ => DialogResult::Continue,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let dialog_width = super::responsive_width(area, 70);
        let dialog_area = super::centered_rect(area, dialog_width, 11);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(" Switch Agent ")
            .title_style(Style::default().fg(theme.title).bold());

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        let session_line = Line::from(vec![
            Span::styled("Session: ", Style::default().fg(theme.dimmed)),
            Span::styled(&self.session_title, Style::default().fg(theme.text)),
            Span::styled("  (now ", Style::default().fg(theme.dimmed)),
            Span::styled(&self.current, Style::default().fg(theme.text)),
            Span::styled(")", Style::default().fg(theme.dimmed)),
        ]);
        frame.render_widget(Paragraph::new(session_line), chunks[0]);

        let mut tool_spans = vec![
            Span::styled("Agent:", Style::default().fg(theme.accent).underlined()),
            Span::raw(" "),
        ];
        for (idx, tool_name) in self.tools.iter().enumerate() {
            let is_selected = idx == self.tool_index;
            let style = if is_selected {
                Style::default().fg(theme.accent).bold()
            } else {
                Style::default().fg(theme.dimmed)
            };
            if idx > 0 {
                tool_spans.push(Span::raw("  "));
            }
            tool_spans.push(Span::styled(if is_selected { "● " } else { "○ " }, style));
            tool_spans.push(Span::styled(*tool_name, style));
        }
        frame.render_widget(Paragraph::new(Line::from(tool_spans)), chunks[2]);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Restarts fresh; the xats name and team are kept.",
                Style::default().fg(theme.dimmed),
            ))),
            chunks[3],
        );

        let hint = Line::from(vec![
            Span::styled("←/→", Style::default().fg(theme.accent)),
            Span::styled(" agent  ", Style::default().fg(theme.dimmed)),
            Span::styled("Enter", Style::default().fg(theme.accent)),
            Span::styled(" switch  ", Style::default().fg(theme.dimmed)),
            Span::styled("Esc", Style::default().fg(theme.accent)),
            Span::styled(" cancel", Style::default().fg(theme.dimmed)),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[5]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn submitted(result: DialogResult<String>) -> Option<String> {
        match result {
            DialogResult::Submit(tool) => Some(tool),
            _ => None,
        }
    }

    #[test]
    fn preselects_the_other_agent() {
        let mut dialog = SwitchAgentDialog::new("id-1", "Carthage", "claude");
        assert_eq!(
            submitted(dialog.handle_key(key(KeyCode::Enter))).as_deref(),
            Some("codex")
        );
        let mut dialog = SwitchAgentDialog::new("id-1", "Carthage", "codex");
        assert_eq!(
            submitted(dialog.handle_key(key(KeyCode::Enter))).as_deref(),
            Some("claude")
        );
    }

    #[test]
    fn choosing_the_current_agent_cancels() {
        let mut dialog = SwitchAgentDialog::new("id-1", "Carthage", "claude");
        dialog.handle_key(key(KeyCode::Right));
        assert!(matches!(
            dialog.handle_key(key(KeyCode::Enter)),
            DialogResult::Cancel
        ));
    }

    #[test]
    fn esc_cancels() {
        let mut dialog = SwitchAgentDialog::new("id-1", "Carthage", "claude");
        assert!(matches!(
            dialog.handle_key(key(KeyCode::Esc)),
            DialogResult::Cancel
        ));
    }
}
