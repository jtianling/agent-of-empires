//! Switch-agent dialog.
//!
//! Opens on `a`/`A` from the home view for the selected session. Shows one row
//! per claude/codex pane, each set either to keep its agent or to switch to
//! another one; a switched pane discards its conversation and keeps its xats
//! identity, and a kept pane is left alone.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::DialogResult;
use crate::tui::styles::Theme;
use crate::xats_identity::KeyHolder;

/// A pane the dialog can switch.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchPane {
    pub slot: i64,
    pub label: String,
    pub agent: String,
    /// Who xats says holds the pane's identity key: the identity the new agent
    /// will come back as. `None` when the pane is not on xats.
    pub identity: Option<KeyHolder>,
}

/// What a confirmed switch hands which pane to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchChoice {
    pub slot: i64,
    pub target: String,
}

pub struct SwitchAgentDialog {
    session_id: String,
    session_title: String,
    panes: Vec<SwitchPane>,
    /// Per pane, the index into `tools` it is set to.
    choices: Vec<usize>,
    focus: usize,
    tools: Vec<&'static str>,
}

impl SwitchAgentDialog {
    /// A lone pane starts set to the other agent, since switching it is the
    /// only reason to open the dialog; split panes all start kept.
    pub fn new(session_id: &str, session_title: &str, panes: Vec<SwitchPane>) -> Self {
        let tools = crate::agents::SWITCHABLE_AGENTS.to_vec();
        let lone = panes.len() == 1;
        let choices = panes
            .iter()
            .map(|pane| {
                tools
                    .iter()
                    .position(|&tool| (tool == pane.agent) != lone)
                    .unwrap_or(0)
            })
            .collect();
        Self {
            session_id: session_id.to_string(),
            session_title: session_title.to_string(),
            panes,
            choices,
            focus: 0,
            tools,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[cfg(test)]
    pub fn panes(&self) -> &[SwitchPane] {
        &self.panes
    }

    fn switched(&self) -> Vec<SwitchChoice> {
        self.panes
            .iter()
            .zip(&self.choices)
            .map(|(pane, &choice)| (pane, self.tools[choice]))
            .filter(|(pane, target)| pane.agent != *target)
            .map(|(pane, target)| SwitchChoice {
                slot: pane.slot,
                target: target.to_string(),
            })
            .collect()
    }

    /// Submits every pane set to a new agent; with none, Enter cancels.
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<Vec<SwitchChoice>> {
        let tools = self.tools.len().max(1);
        let panes = self.panes.len().max(1);
        let step = match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => {
                let switched = self.switched();
                return if switched.is_empty() {
                    DialogResult::Cancel
                } else {
                    DialogResult::Submit(switched)
                };
            }
            KeyCode::Up => {
                self.focus = (self.focus + panes - 1) % panes;
                return DialogResult::Continue;
            }
            KeyCode::Down => {
                self.focus = (self.focus + 1) % panes;
                return DialogResult::Continue;
            }
            KeyCode::Left | KeyCode::BackTab => tools - 1,
            KeyCode::Right | KeyCode::Tab | KeyCode::Char(' ') => 1,
            _ => return DialogResult::Continue,
        };
        if let Some(choice) = self.choices.get_mut(self.focus) {
            *choice = (*choice + step) % tools;
        }
        DialogResult::Continue
    }

    fn pane_row<'a>(&'a self, idx: usize, label: &'a str, theme: &Theme) -> Line<'a> {
        let pane = &self.panes[idx];
        let focused = idx == self.focus && self.panes.len() > 1;
        let label_style = if focused {
            Style::default().fg(theme.accent).bold()
        } else {
            Style::default().fg(theme.accent)
        };
        let mut spans = vec![
            Span::styled(if focused { "› " } else { "  " }, label_style),
            Span::styled(label, label_style.underlined()),
            Span::raw(" "),
        ];
        for (tool_idx, tool) in self.tools.iter().enumerate() {
            let is_selected = tool_idx == self.choices[idx];
            let style = if is_selected {
                Style::default().fg(theme.accent).bold()
            } else {
                Style::default().fg(theme.dimmed)
            };
            if tool_idx > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(if is_selected { "● " } else { "○ " }, style));
            spans.push(Span::styled(*tool, style));
            if *tool == pane.agent {
                spans.push(Span::styled(" (now)", Style::default().fg(theme.dimmed)));
            }
        }
        Line::from(spans)
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let multi_pane = self.panes.len() > 1;
        let pane_rows = self.panes.len().max(1) as u16;
        let switching: Vec<usize> = self
            .panes
            .iter()
            .zip(&self.choices)
            .enumerate()
            .filter(|(_, (pane, &choice))| pane.agent != self.tools[choice])
            .map(|(idx, _)| idx)
            .collect();
        let identity_lines: Vec<Line> = switching
            .iter()
            .filter_map(|&idx| {
                let pane = &self.panes[idx];
                pane.identity.as_ref().map(|holder| {
                    let (text, warn) = identity_line(&pane.label, holder);
                    let color = if warn { theme.error } else { theme.text };
                    Line::from(Span::styled(text, Style::default().fg(color)))
                })
            })
            .collect();
        let identity_rows = identity_lines.len() as u16;
        let dialog_width = super::responsive_width(area, 70);
        let dialog_area = super::centered_rect(area, dialog_width, 10 + pane_rows + identity_rows);

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
                Constraint::Length(pane_rows),
                Constraint::Length(1),
                Constraint::Length(identity_rows),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        let session_line = Line::from(vec![
            Span::styled("Session: ", Style::default().fg(theme.dimmed)),
            Span::styled(&self.session_title, Style::default().fg(theme.text)),
        ]);
        frame.render_widget(Paragraph::new(session_line), chunks[0]);

        let width = self
            .panes
            .iter()
            .map(|pane| pane.label.chars().count())
            .max()
            .unwrap_or(0);
        let labels: Vec<String> = self
            .panes
            .iter()
            .map(|pane| {
                if multi_pane {
                    format!("{:<width$}", format!("{}:", pane.label), width = width + 1)
                } else {
                    "Agent:".to_string()
                }
            })
            .collect();
        let rows: Vec<Line> = labels
            .iter()
            .enumerate()
            .map(|(idx, label)| self.pane_row(idx, label, theme))
            .collect();
        frame.render_widget(Paragraph::new(rows), chunks[2]);

        let note = if multi_pane {
            "Only panes set to a new agent restart; the rest stay as they are."
        } else {
            "Restarts fresh; the pane keeps its xats identity key."
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                note,
                Style::default().fg(theme.dimmed),
            ))),
            chunks[3],
        );

        frame.render_widget(Paragraph::new(identity_lines), chunks[4]);

        let mut hint = Vec::new();
        if multi_pane {
            hint.push(Span::styled("↑/↓", Style::default().fg(theme.accent)));
            hint.push(Span::styled(" pane  ", Style::default().fg(theme.dimmed)));
        }
        hint.extend([
            Span::styled("←/→", Style::default().fg(theme.accent)),
            Span::styled(" agent  ", Style::default().fg(theme.dimmed)),
            Span::styled("Enter", Style::default().fg(theme.accent)),
            Span::styled(" switch  ", Style::default().fg(theme.dimmed)),
            Span::styled("Esc", Style::default().fg(theme.accent)),
            Span::styled(" cancel", Style::default().fg(theme.dimmed)),
        ]);
        frame.render_widget(Paragraph::new(Line::from(hint)), chunks[6]);
    }
}

/// The line describing what identity a pane comes back as, and whether it
/// deserves a warning.
fn identity_line(pane: &str, holder: &KeyHolder) -> (String, bool) {
    match holder {
        KeyHolder::Held { team, name, .. } => {
            (format!("{pane}: comes back as {name}@{team}"), false)
        }
        KeyHolder::NotFound => (
            format!("{pane}: no xats identity holds its key; it must register again"),
            true,
        ),
        KeyHolder::Unsupported => (
            format!("{pane}: xats daemon too old to check its identity"),
            true,
        ),
        KeyHolder::Unavailable(error) => {
            (format!("{pane}: xats identity check failed: {error}"), true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn pane(slot: i64, label: &str, agent: &str) -> SwitchPane {
        SwitchPane {
            slot,
            label: label.to_string(),
            agent: agent.to_string(),
            identity: None,
        }
    }

    fn dialog(panes: Vec<SwitchPane>) -> SwitchAgentDialog {
        SwitchAgentDialog::new("id-1", "Carthage", panes)
    }

    fn submitted(result: DialogResult<Vec<SwitchChoice>>) -> Option<Vec<SwitchChoice>> {
        match result {
            DialogResult::Submit(choices) => Some(choices),
            _ => None,
        }
    }

    fn choices(picks: &[(i64, &str)]) -> Option<Vec<SwitchChoice>> {
        Some(
            picks
                .iter()
                .map(|(slot, target)| SwitchChoice {
                    slot: *slot,
                    target: target.to_string(),
                })
                .collect(),
        )
    }

    fn split() -> SwitchAgentDialog {
        dialog(vec![pane(0, "Left", "claude"), pane(1, "Right", "codex")])
    }

    #[test]
    fn lone_pane_preselects_the_other_agent() {
        let mut d = dialog(vec![pane(0, "Pane 1", "claude")]);
        assert_eq!(
            submitted(d.handle_key(key(KeyCode::Enter))),
            choices(&[(0, "codex")])
        );
        let mut d = dialog(vec![pane(0, "Pane 1", "codex")]);
        assert_eq!(
            submitted(d.handle_key(key(KeyCode::Enter))),
            choices(&[(0, "claude")])
        );
    }

    #[test]
    fn choosing_the_current_agent_cancels() {
        let mut d = dialog(vec![pane(0, "Pane 1", "claude")]);
        d.handle_key(key(KeyCode::Right));
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Cancel
        ));
    }

    #[test]
    fn split_panes_start_kept_so_enter_alone_cancels() {
        let mut d = split();
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Cancel
        ));
    }

    #[test]
    fn each_row_switches_on_its_own() {
        let mut d = split();
        d.handle_key(key(KeyCode::Down));
        d.handle_key(key(KeyCode::Right));
        assert_eq!(
            submitted(d.handle_key(key(KeyCode::Enter))),
            choices(&[(1, "claude")])
        );

        let mut d = split();
        d.handle_key(key(KeyCode::Right));
        assert_eq!(
            submitted(d.handle_key(key(KeyCode::Enter))),
            choices(&[(0, "codex")])
        );
    }

    #[test]
    fn both_rows_can_switch_together() {
        let mut d = split();
        d.handle_key(key(KeyCode::Right));
        d.handle_key(key(KeyCode::Down));
        d.handle_key(key(KeyCode::Left));
        assert_eq!(
            submitted(d.handle_key(key(KeyCode::Enter))),
            choices(&[(0, "codex"), (1, "claude")])
        );
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
            identity_line("Left", &held),
            ("Left: comes back as mie-main@mie".to_string(), false)
        );
        assert!(identity_line("Right", &KeyHolder::NotFound).1);
        assert!(identity_line("Right", &KeyHolder::Unsupported).1);
        assert!(identity_line("Right", &KeyHolder::Unavailable("down".to_string())).1);
    }

    #[test]
    fn esc_cancels() {
        let mut d = dialog(vec![pane(0, "Pane 1", "claude")]);
        assert!(matches!(
            d.handle_key(key(KeyCode::Esc)),
            DialogResult::Cancel
        ));
    }
}
