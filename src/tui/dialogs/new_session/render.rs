//! Rendering for NewSessionDialog

use ratatui::prelude::*;
use ratatui::widgets::*;

use super::{
    FieldHelp, NewSessionDialog, PaneDialogState, PaneTarget, FIELD_HELP, HELP_DIALOG_WIDTH,
    SPINNER_FRAMES,
};
use crate::tui::components::{render_text_field, render_text_field_with_ghost};
use crate::tui::styles::Theme;

impl NewSessionDialog {
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.loading {
            self.render_loading(frame, area, theme);
            return;
        }
        if self.tool_config_mode {
            self.render_tool_config(frame, area, theme);
            return;
        }
        if self.worktree_config_mode {
            self.render_worktree_config(frame, area, theme);
            return;
        }

        let layout = self.field_layout();
        let mut constraints = vec![
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
        ];
        if layout.yolo != super::layout::ABSENT || layout.cross_agent_team != super::layout::ABSENT
        {
            constraints.push(Constraint::Length(2));
        }
        if layout.xats_team != super::layout::ABSENT {
            constraints.push(Constraint::Length(2));
        }
        constraints.extend([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(2),
        ]);
        if self.secondary.is_some() {
            constraints.push(Constraint::Length(2));
            if layout.right_pane_yolo != super::layout::ABSENT
                || layout.right_pane_cross_agent_team != super::layout::ABSENT
            {
                constraints.push(Constraint::Length(2));
            }
            if layout.right_pane_xats_team != super::layout::ABSENT {
                constraints.push(Constraint::Length(2));
            }
            constraints.push(Constraint::Length(2));
        }
        constraints.push(Constraint::Min(1));

        let dialog_width = crate::tui::dialogs::responsive_width(area, 120);
        let content_height: u16 = constraints
            .iter()
            .map(|constraint| match constraint {
                Constraint::Length(value) | Constraint::Min(value) => *value,
                _ => 0,
            })
            .sum();
        let dialog_area = crate::tui::dialogs::centered_rect(
            area,
            dialog_width,
            (content_height + 4).min(area.height),
        );
        frame.render_widget(Clear, dialog_area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(" New Session ")
            .title_style(Style::default().fg(theme.title).bold());
        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);
        let chunks = Layout::vertical(constraints).margin(1).split(inner);
        let mut row = 0;

        render_text_field(
            frame,
            chunks[row],
            "Title:",
            &self.title,
            self.focused_field == layout.title,
            Some("(random civ)"),
            theme,
        );
        row += 1;
        render_text_field_with_ghost(
            frame,
            chunks[row],
            "Group:",
            &self.group,
            self.focused_field == layout.group,
            (!self.existing_groups.is_empty() && self.focused_field == layout.group)
                .then_some("(Ctrl+P to browse groups)"),
            self.group_ghost_text(),
            theme,
        );
        row += 1;
        self.render_tool_row(frame, chunks[row], PaneTarget::Primary, "Tool:", theme);
        row += 1;
        self.primary.path.render(
            frame,
            chunks[row],
            "Path:",
            self.focused_field == layout.path,
            (self.focused_field == layout.path).then_some("(Ctrl+P to browse directories)"),
            theme,
        );
        row += 1;
        if layout.yolo != super::layout::ABSENT || layout.cross_agent_team != super::layout::ABSENT
        {
            self.render_flags_row(frame, chunks[row], PaneTarget::Primary, theme);
            row += 1;
        }
        if layout.xats_team != super::layout::ABSENT {
            self.render_declared_identity_row(frame, chunks[row], PaneTarget::Primary, theme);
            row += 1;
        }
        self.render_worktree_row(frame, chunks[row], PaneTarget::Primary, theme);
        row += 1;

        let divider = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme.border));
        frame.render_widget(divider, chunks[row]);
        row += 1;

        self.render_right_pane_selector(frame, chunks[row], theme);
        row += 1;
        if let Some(secondary) = self.secondary.as_ref() {
            secondary.path.render(
                frame,
                chunks[row],
                "Right Pane Path:",
                self.focused_field == layout.right_pane_path,
                (self.focused_field == layout.right_pane_path)
                    .then_some("(empty = primary path | Ctrl+P to browse)"),
                theme,
            );
            row += 1;
            if layout.right_pane_yolo != super::layout::ABSENT
                || layout.right_pane_cross_agent_team != super::layout::ABSENT
            {
                self.render_flags_row(frame, chunks[row], PaneTarget::Secondary, theme);
                row += 1;
            }
            if layout.right_pane_xats_team != super::layout::ABSENT {
                self.render_declared_identity_row(frame, chunks[row], PaneTarget::Secondary, theme);
                row += 1;
            }
            self.render_worktree_row(frame, chunks[row], PaneTarget::Secondary, theme);
            row += 1;
        }
        self.render_footer(frame, chunks[row], theme);

        if self.show_help {
            self.render_help_overlay(frame, area, theme);
        }
        if self.group_picker.is_active() {
            self.group_picker.render(frame, area, theme);
        }
        if self.branch_picker.is_active() {
            self.branch_picker.render(frame, area, theme);
        }
        if self.dir_picker.is_active() {
            self.dir_picker.render(frame, area, theme);
        }
    }

    fn render_tool_row(
        &self,
        frame: &mut Frame,
        area: Rect,
        target: PaneTarget,
        label: &str,
        theme: &Theme,
    ) {
        let layout = self.field_layout();
        let field = match target {
            PaneTarget::Primary => layout.tool,
            PaneTarget::Secondary => layout.right_pane,
        };
        let selected = self.pane(target).map_or(0, |pane| pane.tool_index);
        let focused = self.focused_field == field;
        let label_style = if focused {
            Style::default().fg(theme.accent).underlined()
        } else {
            Style::default().fg(theme.text)
        };
        let mut spans = vec![Span::styled(label, label_style), Span::raw(" ")];
        for (index, tool) in self.available_tools.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            let active = index == selected;
            let style = if active {
                Style::default().fg(theme.accent).bold()
            } else {
                Style::default().fg(theme.dimmed)
            };
            spans.push(Span::styled(if active { "● " } else { "○ " }, style));
            spans.push(Span::styled(*tool, style));
        }
        if target == PaneTarget::Primary
            && (!self.extra_args.value().is_empty() || !self.command_override.value().is_empty())
        {
            spans.push(Span::styled(
                "  (configured)",
                Style::default().fg(theme.dimmed),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_right_pane_selector(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let focused = self.focused_field == self.field_layout().right_pane;
        let label_style = if focused {
            Style::default().fg(theme.accent).underlined()
        } else {
            Style::default().fg(theme.text)
        };
        let selected = self.right_pane_selection_index();
        let mut spans = vec![
            Span::styled("Right Pane Agent:", label_style),
            Span::raw(" "),
        ];
        for index in 0..=self.available_tools.len() {
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            let active = index == selected;
            let style = if active {
                Style::default().fg(theme.accent).bold()
            } else {
                Style::default().fg(theme.dimmed)
            };
            spans.push(Span::styled(if active { "● " } else { "○ " }, style));
            let name = if index == 0 {
                "none"
            } else {
                self.available_tools[index - 1]
            };
            spans.push(Span::styled(name, style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_flags_row(&self, frame: &mut Frame, area: Rect, target: PaneTarget, theme: &Theme) {
        let Some(pane) = self.pane(target) else {
            return;
        };
        let layout = self.field_layout();
        let (yolo_field, cat_field) = match target {
            PaneTarget::Primary => (layout.yolo, layout.cross_agent_team),
            PaneTarget::Secondary => (layout.right_pane_yolo, layout.right_pane_cross_agent_team),
        };
        let mut spans = Vec::new();
        if yolo_field != super::layout::ABSENT {
            spans.extend(self.checkbox_spans(
                "YOLO Mode:",
                pane.yolo_mode,
                self.focused_field == yolo_field,
                theme,
            ));
        }
        if cat_field != super::layout::ABSENT {
            if !spans.is_empty() {
                spans.push(Span::raw("    "));
            }
            spans.extend(self.checkbox_spans(
                "Cross Agent Team:",
                pane.cross_agent_team,
                self.focused_field == cat_field,
                theme,
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// The pane's declared xats identity: team and agent name side by side, both
    /// optional. Shown only while that pane's Cross Agent Team is on, which is
    /// the only state in which a declaration travels anywhere.
    fn render_declared_identity_row(
        &self,
        frame: &mut Frame,
        area: Rect,
        target: PaneTarget,
        theme: &Theme,
    ) {
        let Some(pane) = self.pane(target) else {
            return;
        };
        let layout = self.field_layout();
        let (team_field, name_field) = match target {
            PaneTarget::Primary => (layout.xats_team, layout.xats_agent_name),
            PaneTarget::Secondary => (
                layout.right_pane_xats_team,
                layout.right_pane_xats_agent_name,
            ),
        };
        let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        render_text_field(
            frame,
            columns[0],
            "xats Team:",
            &pane.xats_team,
            self.focused_field == team_field,
            Some("(undeclared)"),
            theme,
        );
        render_text_field(
            frame,
            columns[1],
            "xats Name:",
            &pane.xats_agent_name,
            self.focused_field == name_field,
            Some("(undeclared)"),
            theme,
        );
    }

    fn checkbox_spans<'a>(
        &self,
        label: &'a str,
        enabled: bool,
        focused: bool,
        theme: &Theme,
    ) -> Vec<Span<'a>> {
        let label_style = if focused {
            Style::default().fg(theme.accent).underlined()
        } else {
            Style::default().fg(theme.text)
        };
        let value_style = if enabled {
            Style::default().fg(theme.accent).bold()
        } else {
            Style::default().fg(theme.dimmed)
        };
        vec![
            Span::styled(label, label_style),
            Span::raw(" "),
            Span::styled(if enabled { "[x]" } else { "[ ]" }, value_style),
        ]
    }

    fn render_worktree_row(
        &self,
        frame: &mut Frame,
        area: Rect,
        target: PaneTarget,
        theme: &Theme,
    ) {
        let Some(pane) = self.pane(target) else {
            return;
        };
        let layout = self.field_layout();
        let field = match target {
            PaneTarget::Primary => layout.worktree,
            PaneTarget::Secondary => layout.right_pane_worktree,
        };
        let focused = self.focused_field == field;
        let placeholder = if focused {
            Some("(leave empty to skip | Ctrl+P to configure)")
        } else {
            Some("(leave empty to skip)")
        };
        render_text_field(
            frame,
            area,
            "Worktree:",
            &pane.worktree_branch,
            focused,
            placeholder,
            theme,
        );
        if !pane.worktree_branch.value().is_empty() && !focused {
            let summary = if pane.workspace_repos.is_empty() {
                if pane.create_new_branch {
                    "new branch".to_string()
                } else {
                    "existing branch".to_string()
                }
            } else {
                format!("{} repos", pane.workspace_repos.len() + 1)
            };
            let x = area
                .x
                .saturating_add(area.width.saturating_sub(summary.len() as u16 + 2));
            frame.render_widget(
                Paragraph::new(summary).style(Style::default().fg(theme.dimmed)),
                Rect::new(x, area.y, area.right().saturating_sub(x), 1),
            );
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if let Some(confirm) = &self.confirm_create_dirs {
            let line = Line::from(vec![
                Span::styled(
                    format!("Create {}? ", confirm.dirs.join(", ")),
                    Style::default().fg(theme.error),
                ),
                Span::styled(
                    "[y]es",
                    if confirm.yes_selected {
                        Style::default().fg(theme.accent).bold()
                    } else {
                        Style::default().fg(theme.dimmed)
                    },
                ),
                Span::raw(" "),
                Span::styled(
                    "[N]o",
                    if confirm.yes_selected {
                        Style::default().fg(theme.dimmed)
                    } else {
                        Style::default().fg(theme.accent).bold()
                    },
                ),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        if let Some(error) = &self.error_message {
            frame.render_widget(
                Paragraph::new(format!("Error: {error}"))
                    .style(Style::default().fg(theme.error))
                    .wrap(Wrap { trim: true }),
                area,
            );
            return;
        }
        let line = Line::from(vec![
            Span::styled("Tab", Style::default().fg(theme.hint)),
            Span::raw(" next  "),
            Span::styled("Enter", Style::default().fg(theme.hint)),
            Span::raw(" create  "),
            Span::styled("?", Style::default().fg(theme.hint)),
            Span::raw(" help  "),
            Span::styled("Esc", Style::default().fg(theme.hint)),
            Span::raw(" cancel"),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_tool_config(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let selected_tool = self.pane_tool(PaneTarget::Primary).unwrap_or("claude");
        let dialog_area = crate::tui::dialogs::centered_rect(area, 80, 10);
        frame.render_widget(Clear, dialog_area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(format!(" Tool Configuration: {selected_tool} "));
        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);
        let chunks = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .margin(1)
        .split(inner);
        render_text_field(
            frame,
            chunks[0],
            "Command:",
            &self.command_override,
            self.tool_config_focused_field == 0,
            Some("(default binary)"),
            theme,
        );
        render_text_field(
            frame,
            chunks[1],
            "Extra Args:",
            &self.extra_args,
            self.tool_config_focused_field == 1,
            Some("(none)"),
            theme,
        );
        frame.render_widget(Paragraph::new("Tab next  Enter done  Esc back"), chunks[2]);
    }

    fn render_worktree_config(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let Some(pane) = self.pane(self.worktree_config_target) else {
            return;
        };
        let repos_height = if pane.workspace_repos_expanded {
            (pane.workspace_repos.len() as u16 + 2).clamp(4, 8)
        } else {
            2
        };
        let dialog_area =
            crate::tui::dialogs::centered_rect(area, 90, (repos_height + 8).min(area.height));
        frame.render_widget(Clear, dialog_area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(format!(" Worktree: {} ", pane.worktree_branch.value()));
        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);
        let chunks = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(repos_height),
            Constraint::Min(1),
        ])
        .margin(1)
        .split(inner);
        let line = Line::from(self.checkbox_spans(
            if pane.create_new_branch {
                "Create new branch:"
            } else {
                "Attach existing branch:"
            },
            pane.create_new_branch,
            self.worktree_config_focused_field == 0,
            theme,
        ));
        frame.render_widget(Paragraph::new(line), chunks[0]);
        self.render_extra_repos_field(frame, chunks[1], pane, theme);
        frame.render_widget(
            Paragraph::new("Tab next  Space toggle  Ctrl+P branches  Enter done  Esc back"),
            chunks[2],
        );
        if self.branch_picker.is_active() {
            self.branch_picker.render(frame, area, theme);
        }
        if self.dir_picker.is_active() {
            self.dir_picker.render(frame, area, theme);
        }
    }

    fn render_extra_repos_field(
        &self,
        frame: &mut Frame,
        area: Rect,
        pane: &PaneDialogState,
        theme: &Theme,
    ) {
        let focused = self.worktree_config_focused_field == 1;
        let label_style = if focused {
            Style::default().fg(theme.accent).underlined()
        } else {
            Style::default().fg(theme.text)
        };
        if !pane.workspace_repos_expanded {
            let summary = if pane.workspace_repos.is_empty() {
                "(empty - Enter to edit)".to_string()
            } else {
                format!("[{} repos]", pane.workspace_repos.len())
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("Extra Repos:", label_style),
                    Span::raw(" "),
                    Span::styled(summary, Style::default().fg(theme.dimmed)),
                ])),
                area,
            );
            return;
        }

        let mut lines = vec![Line::from(vec![
            Span::styled("Extra Repos:", label_style),
            Span::styled(
                " (a)dd (d)elete Enter edit Ctrl+P browse Esc close",
                Style::default().fg(theme.dimmed),
            ),
        ])];
        for (index, entry) in pane.workspace_repos.iter().enumerate() {
            let editing = pane.workspace_repo_editing_input.as_ref().filter(|_| {
                !pane.workspace_repo_adding_new && pane.workspace_repo_selected_index == index
            });
            let value = editing.map_or(entry.as_str(), |input| input.value());
            let selected = pane.workspace_repo_selected_index == index;
            lines.push(Line::from(Span::styled(
                format!("{}{}", if selected { "  > " } else { "    " }, value),
                if selected {
                    Style::default().fg(theme.accent).bold()
                } else {
                    Style::default().fg(theme.text)
                },
            )));
        }
        if pane.workspace_repo_adding_new {
            if let Some(input) = pane.workspace_repo_editing_input.as_ref() {
                lines.push(Line::from(Span::styled(
                    format!("  + {}", input.value()),
                    Style::default().fg(theme.accent),
                )));
            }
        } else if pane.workspace_repos.is_empty() {
            lines.push(Line::from(Span::styled(
                "    (press 'a' to add a repository path)",
                Style::default().fg(theme.dimmed),
            )));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn render_help_overlay(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let shown: Vec<&FieldHelp> = FIELD_HELP
            .iter()
            .filter(|help| self.help_entry_visible(help.visibility))
            .collect();
        let dialog_area =
            crate::tui::dialogs::centered_rect(area, HELP_DIALOG_WIDTH, shown.len() as u16 * 3 + 5);
        frame.render_widget(Clear, dialog_area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title(" New Session Help ");
        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);
        let mut lines = Vec::new();
        for help in shown {
            lines.push(Line::from(Span::styled(
                help.name,
                Style::default().fg(theme.accent).bold(),
            )));
            lines.push(Line::from(format!("  {}", help.description)));
            lines.push(Line::from(""));
        }
        lines.push(Line::from("Press ? or Esc to close"));
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_loading(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let dialog_area = crate::tui::dialogs::centered_rect(area, 70, 12.min(area.height));
        frame.render_widget(Clear, dialog_area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(if self.has_hooks {
                " Running Hooks "
            } else {
                " Creating Session "
            });
        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);
        let spinner = SPINNER_FRAMES[self.spinner_frame];
        let mut lines = vec![Line::from(format!(
            " {spinner} {}",
            self.current_hook
                .as_deref()
                .unwrap_or("Creating session...")
        ))];
        lines.extend(
            self.hook_output
                .iter()
                .rev()
                .take(6)
                .rev()
                .map(|line| Line::from(format!("  {line}"))),
        );
        lines.push(Line::from(""));
        lines.push(Line::from(" Press Esc to cancel"));
        frame.render_widget(Paragraph::new(lines), inner);
    }
}
