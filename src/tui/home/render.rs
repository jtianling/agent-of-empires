//! Rendering for HomeView

use ratatui::prelude::*;
use ratatui::widgets::*;
use std::time::Instant;

use super::{
    get_indent, HomeView, ICON_COLLAPSED, ICON_DELETING, ICON_ERROR, ICON_EXPANDED, ICON_IDLE,
    ICON_RUNNING, ICON_STARTING, ICON_STOPPED, ICON_UNKNOWN, ICON_WAITING,
};
use crate::session::{Item, Status};
use crate::tui::components::{HelpOverlay, Preview};
use crate::tui::styles::Theme;
use crate::update::UpdateInfo;

const STATUS_BAR_SPINNER_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];

impl HomeView {
    pub fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
        update_info: Option<&UpdateInfo>,
    ) {
        // Settings view takes over the whole screen
        if let Some(ref mut settings) = self.settings_view {
            settings.render(frame, area, theme);
            // Render unsaved changes confirmation dialog over settings
            if self.settings_close_confirm {
                if let Some(dialog) = &self.confirm_dialog {
                    dialog.render(frame, area, theme);
                }
            }
            return;
        }

        // Diff view takes over the whole screen
        if let Some(ref mut diff) = self.diff_view {
            // Compute diff for selected file if not cached
            let _ = diff.get_current_diff();

            diff.render(frame, area, theme);
            return;
        }

        // Layout: main area + status bar + optional update bar at bottom
        let constraints = if update_info.is_some() {
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
            .constraints(constraints)
            .split(area);

        // Layout: left panel (list) and right panel (preview)
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(self.list_width), Constraint::Min(40)])
            .split(main_chunks[0]);

        self.render_list(frame, chunks[0], theme);
        self.render_preview(frame, chunks[1], theme);
        self.render_status_bar(frame, main_chunks[1], theme);

        if let Some(info) = update_info {
            self.render_update_bar(frame, main_chunks[2], theme, info);
        }

        // Render dialogs on top
        if self.show_help {
            HelpOverlay::render(frame, area, theme, self.sort_order);
        }

        if let Some(dialog) = &self.new_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.confirm_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.unified_delete_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.group_delete_options_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.group_rename_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.rename_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.hook_trust_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.welcome_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.changelog_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.info_dialog {
            dialog.render(frame, area, theme);
        }

        if let Some(dialog) = &self.profile_picker_dialog {
            dialog.render(frame, area, theme);
        }
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let title = format!(" Agent of Empires [{}] ", self.storage.profile());
        let (border_color, title_color) = (theme.border, theme.title);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(title)
            .title_style(Style::default().fg(title_color).bold());

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.instances.is_empty() && self.groups.is_empty() {
            let empty_text = vec![
                Line::from(""),
                Line::from("No sessions yet").style(Style::default().fg(theme.dimmed)),
                Line::from(""),
                Line::from("Press 'n' to create one").style(Style::default().fg(theme.hint)),
                Line::from("or 'agent-of-empires add .'").style(Style::default().fg(theme.hint)),
            ];
            let para = Paragraph::new(empty_text).alignment(Alignment::Center);
            frame.render_widget(para, inner);
            return;
        }

        let stable_session_indices = self.stable_session_indices();

        let list_items: Vec<ListItem> = self
            .flat_items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let is_selected = idx == self.cursor;
                let is_match =
                    !self.search_matches.is_empty() && self.search_matches.contains(&idx);
                let session_num = match item {
                    Item::Session { id, .. } => stable_session_indices
                        .get(id)
                        .copied()
                        .filter(|index| *index <= 99),
                    Item::Group { .. } | Item::ProfileHeader { .. } => None,
                };
                self.render_item(item, is_selected, is_match, session_num, theme)
            })
            .collect();

        let list =
            List::new(list_items).highlight_style(Style::default().bg(theme.session_selection));

        frame.render_widget(list, inner);

        // Render search bar if active
        if self.search_active {
            let search_area = Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            };

            let value = self.search_query.value();
            let cursor_pos = self.search_query.visual_cursor();
            let cursor_style = Style::default().fg(theme.background).bg(theme.search);
            let text_style = Style::default().fg(theme.search);

            // Split value into: before cursor, char at cursor, after cursor
            let before: String = value.chars().take(cursor_pos).collect();
            let cursor_char: String = value
                .chars()
                .nth(cursor_pos)
                .map(|c| c.to_string())
                .unwrap_or_else(|| " ".to_string());
            let after: String = value.chars().skip(cursor_pos + 1).collect();

            let mut spans = vec![Span::styled("/", text_style)];
            if !before.is_empty() {
                spans.push(Span::styled(before, text_style));
            }
            spans.push(Span::styled(cursor_char, cursor_style));
            if !after.is_empty() {
                spans.push(Span::styled(after, text_style));
            }

            if !self.search_matches.is_empty() {
                let count_text = format!(
                    " [{}/{}]",
                    self.search_match_index + 1,
                    self.search_matches.len()
                );
                spans.push(Span::styled(count_text, Style::default().fg(theme.dimmed)));
            } else if !value.is_empty() {
                spans.push(Span::styled(" [0/0]", Style::default().fg(theme.dimmed)));
            }

            frame.render_widget(Paragraph::new(Line::from(spans)), search_area);
        }
    }

    fn render_item(
        &self,
        item: &Item,
        is_selected: bool,
        is_match: bool,
        session_num: Option<usize>,
        theme: &Theme,
    ) -> ListItem<'_> {
        let indent = get_indent(item.depth());

        use std::borrow::Cow;

        let (icon, text, style): (&str, Cow<str>, Style) = match item {
            Item::Group {
                name,
                collapsed,
                session_count,
                ..
            } => {
                let icon = if *collapsed {
                    ICON_COLLAPSED
                } else {
                    ICON_EXPANDED
                };
                let text = Cow::Owned(format!("{} ({})", name, session_count));
                let style = Style::default().fg(theme.group).bold();
                (icon, text, style)
            }
            Item::ProfileHeader {
                name,
                collapsed,
                session_count,
            } => {
                let icon = if *collapsed {
                    ICON_COLLAPSED
                } else {
                    ICON_EXPANDED
                };
                let text = Cow::Owned(format!("{} ({})", name, session_count));
                let style = Style::default().fg(theme.group).bold();
                (icon, text, style)
            }
            Item::Session { id, .. } => {
                if let Some(inst) = self.get_instance(id) {
                    {
                        let icon = match inst.status {
                            Status::Running => ICON_RUNNING,
                            Status::Waiting => ICON_WAITING,
                            Status::Idle => ICON_IDLE,
                            Status::Unknown => ICON_UNKNOWN,
                            Status::Stopped => ICON_STOPPED,
                            Status::Error => ICON_ERROR,
                            Status::Starting => ICON_STARTING,
                            Status::Restarting => ICON_STARTING,
                            Status::Deleting => ICON_DELETING,
                        };
                        let color = match inst.status {
                            Status::Running => theme.running,
                            Status::Waiting => theme.waiting,
                            Status::Idle => theme.idle,
                            Status::Unknown => theme.waiting,
                            Status::Stopped => theme.dimmed,
                            Status::Error => theme.error,
                            Status::Starting => theme.dimmed,
                            Status::Restarting => theme.dimmed,
                            Status::Deleting => theme.waiting,
                        };
                        let style = Style::default().fg(color);
                        (icon, Cow::Borrowed(&inst.title), style)
                    }
                } else {
                    (
                        "?",
                        Cow::Borrowed(id.as_str()),
                        Style::default().fg(theme.dimmed),
                    )
                }
            }
        };

        let mut line_spans = Vec::with_capacity(6);
        // Right-aligned 2-char numeric index prefix (blank for groups)
        let num_prefix = match session_num {
            Some(n) => format!("{:>2} ", n),
            None => "   ".to_string(),
        };
        line_spans.push(Span::styled(num_prefix, Style::default().fg(theme.dimmed)));
        line_spans.push(Span::raw(indent));
        let icon_style = if is_match {
            Style::default().fg(theme.search)
        } else {
            style
        };
        line_spans.push(Span::styled(format!("{} ", icon), icon_style));
        line_spans.push(Span::styled(
            text.into_owned(),
            if is_selected { style.bold() } else { style },
        ));

        if let Item::Session { id, .. } = item {
            if let Some(inst) = self.get_instance(id) {
                if let Some(ws_info) = &inst.workspace_info {
                    line_spans.push(Span::styled(
                        format!("  {} [{} repos]", ws_info.branch, ws_info.repos.len()),
                        Style::default().fg(theme.branch),
                    ));
                } else if let Some(wt_info) = &inst.worktree_info {
                    line_spans.push(Span::styled(
                        format!("  {}", wt_info.branch),
                        Style::default().fg(theme.branch),
                    ));
                }
                if inst.is_sandboxed() {
                    line_spans.push(Span::styled(
                        " [sandbox]",
                        Style::default().fg(theme.sandbox),
                    ));
                }
            }
        }

        let line = Line::from(line_spans);

        if is_selected {
            ListItem::new(line).style(Style::default().bg(theme.session_selection))
        } else {
            ListItem::new(line)
        }
    }

    /// Refresh preview cache if needed (session changed, dimensions changed, or timer expired)
    pub(super) fn refresh_preview_cache_if_needed(&mut self, width: u16, height: u16) -> bool {
        const PREVIEW_REFRESH_MS: u128 = 250; // Refresh preview 4x/second max

        let needs_refresh = match &self.selected_session {
            Some(id) => {
                self.preview_cache.session_id.as_ref() != Some(id)
                    || self.preview_cache.dimensions != (width, height)
                    || self.preview_cache.last_refresh.elapsed().as_millis() > PREVIEW_REFRESH_MS
            }
            None => false,
        };

        if needs_refresh {
            if let Some(id) = &self.selected_session {
                if let Some(inst) = self.get_instance(id) {
                    let new_content = inst
                        .capture_output_with_size(height as usize, width, height)
                        .unwrap_or_default();

                    let changed = new_content != self.preview_cache.content
                        || self.preview_cache.session_id.as_ref() != Some(id);

                    self.preview_cache.content = new_content;
                    self.preview_cache.session_id = Some(id.clone());
                    self.preview_cache.dimensions = (width, height);
                    self.preview_cache.last_refresh = Instant::now();
                    return changed;
                }
            }
        }
        false
    }

    /// Refresh terminal preview cache if needed (for host terminals)
    fn render_preview(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .title(" Preview ")
            .title_style(Style::default().fg(theme.title));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if let Some(id) = &self.selected_session {
            if let Some(inst) = self.get_instance(id) {
                Preview::render_with_cache(frame, inner, inst, &self.preview_cache.content, theme);
            }
        } else {
            let hint = Paragraph::new("Select a session to preview")
                .style(Style::default().fg(theme.dimmed))
                .alignment(Alignment::Center);
            frame.render_widget(hint, inner);
        }
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let key_style = Style::default().fg(theme.accent).bold();
        let desc_style = Style::default().fg(theme.dimmed);
        let sep_style = Style::default().fg(theme.border);

        let mut spans = vec![
            Span::styled(" j/k", key_style),
            Span::styled(" Nav ", desc_style),
        ];
        if let Some(enter_action_text) = match self.flat_items.get(self.cursor) {
            Some(Item::Group {
                collapsed: true, ..
            }) => Some(" Expand "),
            Some(Item::Group {
                collapsed: false, ..
            }) => Some(" Collapse "),
            Some(Item::ProfileHeader {
                collapsed: true, ..
            }) => Some(" Expand "),
            Some(Item::ProfileHeader {
                collapsed: false, ..
            }) => Some(" Collapse "),
            Some(Item::Session { .. }) => Some(" Attach "),
            None => None,
        } {
            spans.extend([
                Span::styled("│", sep_style),
                Span::styled(" Enter", key_style),
                Span::styled(enter_action_text, desc_style),
            ])
        }

        spans.extend([
            Span::styled("│", sep_style),
            Span::styled(" n", key_style),
            Span::styled(" New ", desc_style),
        ]);

        if !self.flat_items.is_empty() {
            spans.extend([
                Span::styled("│", sep_style),
                Span::styled(" d", key_style),
                Span::styled(" Del ", desc_style),
            ]);
        }

        spans.extend([
            Span::styled("│", sep_style),
            Span::styled(" /", key_style),
            Span::styled(" Search ", desc_style),
            Span::styled("│", sep_style),
            Span::styled(" D", key_style),
            Span::styled(" Diff ", desc_style),
            Span::styled("│", sep_style),
            Span::styled(" ?", key_style),
            Span::styled(" Help ", desc_style),
            Span::styled("│", sep_style),
            Span::styled(" q", key_style),
            Span::styled(" Quit", desc_style),
        ]);

        if let Some(ref pending) = self.pending_jump {
            spans.extend([
                Span::styled("│", sep_style),
                Span::styled(
                    format!(" jump: {}_ ", pending.first_digit),
                    Style::default().fg(theme.waiting).bold(),
                ),
            ]);
        }

        if let Some(selected_id) = &self.selected_session {
            if let Some(inst) = self.get_instance(selected_id) {
                if inst.status == Status::Restarting {
                    let frame = (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_millis() / 150)
                        .unwrap_or(0) as usize)
                        % STATUS_BAR_SPINNER_FRAMES.len();
                    spans.extend([
                        Span::styled("│", sep_style),
                        Span::styled(
                            format!(" {} ", STATUS_BAR_SPINNER_FRAMES[frame]),
                            Style::default().fg(theme.waiting).bold(),
                        ),
                        Span::styled(" Restarting... ", desc_style),
                    ]);
                }
            }
        }

        let status = Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.selection));
        frame.render_widget(status, area);
    }

    fn render_update_bar(&self, frame: &mut Frame, area: Rect, theme: &Theme, info: &UpdateInfo) {
        let update_style = Style::default().fg(theme.waiting).bold();
        let text = format!(
            " update available {} -> {}",
            info.current_version, info.latest_version
        );
        let bar = Paragraph::new(Line::from(Span::styled(text, update_style)))
            .style(Style::default().bg(theme.selection));
        frame.render_widget(bar, area);
    }
}
