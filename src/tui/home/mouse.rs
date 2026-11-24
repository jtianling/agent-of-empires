use crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use super::HomeView;
use crate::tui::app::Action;
use crate::tui::diff::DiffAction;

const SCROLL_ROWS: usize = 3;

impl HomeView {
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Option<Action> {
        if let Some(diff) = &mut self.diff_view {
            return match diff.handle_mouse(mouse) {
                DiffAction::Continue => None,
                DiffAction::Close => {
                    self.diff_view = None;
                    None
                }
                DiffAction::EditFile(path) => Some(Action::EditFile(path)),
            };
        }
        if self.has_dialog()
            || !self
                .list_area
                .contains(Position::new(mouse.column, mouse.row))
        {
            return None;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_list(false),
            MouseEventKind::ScrollDown => self.scroll_list(true),
            _ => {}
        }
        None
    }

    fn scroll_list(&mut self, down: bool) {
        let rows = usize::from(self.list_area.height);
        if self.flat_items.is_empty() || rows == 0 {
            return;
        }
        let limit = self.flat_items.len().saturating_sub(rows);
        self.list_offset = if down {
            self.list_offset.saturating_add(SCROLL_ROWS).min(limit)
        } else {
            self.list_offset.saturating_sub(SCROLL_ROWS).min(limit)
        };
        let last = (self.list_offset + rows - 1).min(self.flat_items.len() - 1);
        self.cursor = self.cursor.clamp(self.list_offset, last);
        self.update_selected();
    }
}
