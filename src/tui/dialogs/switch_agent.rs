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
use crate::xats_identity::KeyHolder;

pub struct SwitchAgentDialog {
    session_id: String,
    session_title: String,
    current: String,
    tools: Vec<&'static str>,
    tool_index: usize,
    /// Per switched pane (by slot), who xats says holds its identity key: the
    /// identity the new agent will come back as.
    identities: Vec<(i64, KeyHolder)>,
}

impl SwitchAgentDialog {
    pub fn new(
        session_id: &str,
        session_title: &str,
        current: &str,
        identities: Vec<(i64, KeyHolder)>,
    ) -> Self {
        let tools = crate::agents::SWITCHABLE_AGENTS.to_vec();
        let tool_index = tools.iter().position(|&t| t != current).unwrap_or(0);
        Self {
            session_id: session_id.to_string(),
            session_title: session_title.to_string(),
            current: current.to_string(),
            tools,
            tool_index,
            identities,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[cfg(test)]
    pub fn identities(&self) -> &[(i64, KeyHolder)] {
        &self.identities
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
        let identity_rows = self.identities.len() as u16;
        let dialog_area = super::centered_rect(area, dialog_width, 11 + identity_rows);

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
                Constraint::Length(identity_rows),
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
                "Restarts fresh; each pane keeps its xats identity key.",
                Style::default().fg(theme.dimmed),
            ))),
            chunks[3],
        );

        let identity_lines: Vec<Line> = self
            .identities
            .iter()
            .map(|(slot, holder)| {
                let (text, warn) = identity_line(*slot, holder);
                let color = if warn { theme.error } else { theme.text };
                Line::from(Span::styled(text, Style::default().fg(color)))
            })
            .collect();
        frame.render_widget(Paragraph::new(identity_lines), chunks[4]);

        let hint = Line::from(vec![
            Span::styled("←/→", Style::default().fg(theme.accent)),
            Span::styled(" agent  ", Style::default().fg(theme.dimmed)),
            Span::styled("Enter", Style::default().fg(theme.accent)),
            Span::styled(" switch  ", Style::default().fg(theme.dimmed)),
            Span::styled("Esc", Style::default().fg(theme.accent)),
            Span::styled(" cancel", Style::default().fg(theme.dimmed)),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[6]);
    }
}

/// The line describing what identity a pane comes back as, and whether it
/// deserves a warning.
fn identity_line(slot: i64, holder: &KeyHolder) -> (String, bool) {
    let pane = slot + 1;
    match holder {
        KeyHolder::Held { team, name, .. } => {
            (format!("Pane {pane}: comes back as {name}@{team}"), false)
        }
        KeyHolder::NotFound => (
            format!("Pane {pane}: no xats identity holds its key; it must register again"),
            true,
        ),
        KeyHolder::Unsupported => (
            format!("Pane {pane}: xats daemon too old to check its identity"),
            true,
        ),
        KeyHolder::Unavailable(error) => (
            format!("Pane {pane}: xats identity check failed: {error}"),
            true,
        ),
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
        let mut dialog = SwitchAgentDialog::new("id-1", "Carthage", "claude", Vec::new());
        assert_eq!(
            submitted(dialog.handle_key(key(KeyCode::Enter))).as_deref(),
            Some("codex")
        );
        let mut dialog = SwitchAgentDialog::new("id-1", "Carthage", "codex", Vec::new());
        assert_eq!(
            submitted(dialog.handle_key(key(KeyCode::Enter))).as_deref(),
            Some("claude")
        );
    }

    #[test]
    fn choosing_the_current_agent_cancels() {
        let mut dialog = SwitchAgentDialog::new("id-1", "Carthage", "claude", Vec::new());
        dialog.handle_key(key(KeyCode::Right));
        assert!(matches!(
            dialog.handle_key(key(KeyCode::Enter)),
            DialogResult::Cancel
        ));
    }

    #[test]
    fn identity_lines_warn_unless_a_holder_is_known() {
        let held = KeyHolder::Held {
            team: "mie".to_string(),
            name: "mie-main".to_string(),
            agent_type: None,
            active: true,
        };
        assert_eq!(
            identity_line(0, &held),
            ("Pane 1: comes back as mie-main@mie".to_string(), false)
        );
        assert!(identity_line(1, &KeyHolder::NotFound).1);
        assert!(identity_line(1, &KeyHolder::Unsupported).1);
        assert!(identity_line(1, &KeyHolder::Unavailable("down".to_string())).1);
    }

    #[test]
    fn esc_cancels() {
        let mut dialog = SwitchAgentDialog::new("id-1", "Carthage", "claude", Vec::new());
        assert!(matches!(
            dialog.handle_key(key(KeyCode::Esc)),
            DialogResult::Cancel
        ));
    }
}
