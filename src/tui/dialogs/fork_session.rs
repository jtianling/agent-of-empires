//! Fork session dialog.
//!
//! Opens on `f` from the home view with the currently selected session as the
//! parent. Lets the user edit the forked title, group, and the right-pane tool
//! (defaulting to `shell` so the shell pane lands in the parent's working
//! directory alongside the native agent fork).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use super::DialogResult;
use crate::tui::components::render_text_field;
use crate::tui::styles::Theme;

/// What the dialog returns on submit.
#[derive(Debug, Clone)]
pub struct ForkSessionData {
    pub parent_id: String,
    pub title: String,
    pub group: Option<String>,
    pub right_pane_tool: Option<String>,
}

/// Tools offered for the right pane. Order must match `right_pane_label`.
const RIGHT_PANE_OPTIONS: &[&str] = &["none", "shell", "claude", "codex", "opencode"];

pub struct ForkSessionDialog {
    parent_id: String,
    parent_title: String,
    parent_tool: String,
    title: Input,
    group: Input,
    right_pane_index: usize,
    focused_field: usize,
    error: Option<String>,
}

impl ForkSessionDialog {
    pub fn new(parent_id: &str, parent_title: &str, parent_tool: &str, parent_group: &str) -> Self {
        let default_title = format!("{}-fork", parent_title);
        Self {
            parent_id: parent_id.to_string(),
            parent_title: parent_title.to_string(),
            parent_tool: parent_tool.to_string(),
            title: Input::new(default_title),
            group: Input::new(parent_group.to_string()),
            // Default to `shell` so the shell pane lands in the same cwd as
            // the parent. Index 1 in RIGHT_PANE_OPTIONS.
            right_pane_index: 1,
            focused_field: 0,
            error: None,
        }
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
    }

    fn focused_input(&mut self) -> Option<&mut Input> {
        match self.focused_field {
            0 => Some(&mut self.title),
            1 => Some(&mut self.group),
            _ => None,
        }
    }

    fn is_right_pane_field(&self) -> bool {
        self.focused_field == 2
    }

    fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % 3;
    }

    fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            2
        } else {
            self.focused_field - 1
        };
    }

    fn selected_right_pane(&self) -> Option<String> {
        let name = RIGHT_PANE_OPTIONS[self.right_pane_index];
        if name == "none" {
            None
        } else {
            Some(name.to_string())
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<ForkSessionData> {
        self.error = None;

        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => {
                let title_value = self.title.value().trim().to_string();
                if title_value.is_empty() {
                    self.error = Some("Title cannot be empty".to_string());
                    return DialogResult::Continue;
                }
                let group_value = self.group.value().trim().to_string();
                let group = if group_value.is_empty() {
                    None
                } else {
                    Some(group_value)
                };
                DialogResult::Submit(ForkSessionData {
                    parent_id: self.parent_id.clone(),
                    title: title_value,
                    group,
                    right_pane_tool: self.selected_right_pane(),
                })
            }
            KeyCode::Tab => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.prev_field();
                } else {
                    self.next_field();
                }
                DialogResult::Continue
            }
            KeyCode::Down => {
                self.next_field();
                DialogResult::Continue
            }
            KeyCode::Up => {
                self.prev_field();
                DialogResult::Continue
            }
            KeyCode::Left if self.is_right_pane_field() => {
                self.right_pane_index = if self.right_pane_index == 0 {
                    RIGHT_PANE_OPTIONS.len() - 1
                } else {
                    self.right_pane_index - 1
                };
                DialogResult::Continue
            }
            KeyCode::Right | KeyCode::Char(' ') if self.is_right_pane_field() => {
                self.right_pane_index = (self.right_pane_index + 1) % RIGHT_PANE_OPTIONS.len();
                DialogResult::Continue
            }
            _ => {
                if let Some(input) = self.focused_input() {
                    input.handle_event(&crossterm::event::Event::Key(key));
                }
                DialogResult::Continue
            }
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let dialog_width = super::responsive_width(area, 120);
        let dialog_height = 15;
        let dialog_area = super::centered_rect(area, dialog_width, dialog_height);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(" Fork Session ")
            .title_style(Style::default().fg(theme.title).bold());

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1), // Parent
                Constraint::Length(1), // Tool
                Constraint::Length(1), // Spacer
                Constraint::Length(1), // Title field
                Constraint::Length(1), // Group field
                Constraint::Length(1), // Right pane selector
                Constraint::Length(1), // Spacer
                Constraint::Length(1), // Error
                Constraint::Min(1),    // Hint
            ])
            .split(inner);

        let parent_line = Line::from(vec![
            Span::styled("Parent: ", Style::default().fg(theme.dimmed)),
            Span::styled(&self.parent_title, Style::default().fg(theme.text)),
        ]);
        frame.render_widget(Paragraph::new(parent_line), chunks[0]);

        let tool_line = Line::from(vec![
            Span::styled("Tool:   ", Style::default().fg(theme.dimmed)),
            Span::styled(&self.parent_tool, Style::default().fg(theme.text)),
        ]);
        frame.render_widget(Paragraph::new(tool_line), chunks[1]);

        render_text_field(
            frame,
            chunks[3],
            "Title: ",
            &self.title,
            self.focused_field == 0,
            None,
            theme,
        );
        render_text_field(
            frame,
            chunks[4],
            "Group: ",
            &self.group,
            self.focused_field == 1,
            Some("(optional)"),
            theme,
        );

        let right_pane_name = RIGHT_PANE_OPTIONS[self.right_pane_index];
        let right_pane_style = if self.is_right_pane_field() {
            Style::default().fg(theme.accent).bold()
        } else {
            Style::default().fg(theme.text)
        };
        let right_pane_line = Line::from(vec![
            Span::styled("Right Pane: ", Style::default().fg(theme.dimmed)),
            Span::styled(format!("◀ {} ▶", right_pane_name), right_pane_style),
        ]);
        frame.render_widget(Paragraph::new(right_pane_line), chunks[5]);

        if let Some(err) = &self.error {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    err.clone(),
                    Style::default().fg(theme.error),
                ))),
                chunks[7],
            );
        }

        let hint = Line::from(vec![
            Span::styled("Tab", Style::default().fg(theme.accent)),
            Span::styled(" next  ", Style::default().fg(theme.dimmed)),
            Span::styled("←/→", Style::default().fg(theme.accent)),
            Span::styled(" cycle  ", Style::default().fg(theme.dimmed)),
            Span::styled("Enter", Style::default().fg(theme.accent)),
            Span::styled(" fork  ", Style::default().fg(theme.dimmed)),
            Span::styled("Esc", Style::default().fg(theme.accent)),
            Span::styled(" cancel", Style::default().fg(theme.dimmed)),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[8]);
    }
}
