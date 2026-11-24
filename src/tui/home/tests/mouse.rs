use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, layout::Rect, Terminal};
use serial_test::serial;

use super::{create_test_env_empty, create_test_env_with_sessions};
use crate::tui::{dialogs::InfoDialog, styles::Theme};

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
#[serial]
fn wheel_moves_the_viewport_immediately_and_the_next_render_keeps_it() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(50);
    let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
    terminal
        .draw(|frame| {
            env.view
                .render(frame, frame.area(), &Theme::default(), None)
        })
        .unwrap();
    let area = env.view.list_area;
    env.view
        .handle_mouse(mouse(MouseEventKind::ScrollDown, area.x, area.y));
    assert_eq!(env.view.list_offset, 3);
    assert_eq!(env.view.cursor, 3);
    terminal
        .draw(|frame| {
            env.view
                .render(frame, frame.area(), &Theme::default(), None)
        })
        .unwrap();
    assert_eq!(env.view.list_offset, 3);
    assert!(env.view.selected_session.is_some() || env.view.selected_group.is_some());
    env.view
        .handle_mouse(mouse(MouseEventKind::ScrollUp, area.x, area.y));
    assert_eq!(env.view.list_offset, 0);
    assert_eq!(env.view.cursor, 3);
}

#[test]
#[serial]
fn scrolling_is_clamped_at_both_ends_and_ignores_other_mouse_events() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(30);
    env.view.list_area = Rect::new(1, 1, 40, 10);
    env.view.handle_mouse(mouse(MouseEventKind::ScrollUp, 2, 2));
    assert_eq!(env.view.list_offset, 0);
    for _ in 0..30 {
        env.view
            .handle_mouse(mouse(MouseEventKind::ScrollDown, 2, 2));
    }
    assert_eq!(env.view.list_offset, env.view.flat_items.len() - 10);
    assert!((env.view.list_offset..env.view.list_offset + 10).contains(&env.view.cursor));
    let offset = env.view.list_offset;
    env.view
        .handle_mouse(mouse(MouseEventKind::Down(MouseButton::Middle), 2, 2));
    env.view
        .handle_mouse(mouse(MouseEventKind::ScrollLeft, 2, 2));
    assert_eq!(env.view.list_offset, offset);
}

#[test]
#[serial]
fn preview_status_bar_search_bar_and_modal_do_not_scroll_the_background_list() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(30);
    env.view.search_active = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
    terminal
        .draw(|frame| {
            env.view
                .render(frame, frame.area(), &Theme::default(), None)
        })
        .unwrap();
    let area = env.view.list_area;
    for (x, y) in [(area.right() + 3, area.y), (area.x, area.bottom()), (0, 19)] {
        env.view
            .handle_mouse(mouse(MouseEventKind::ScrollDown, x, y));
    }
    assert_eq!(env.view.list_offset, 0);
    env.view.info_dialog = Some(InfoDialog::new("Info", "test"));
    env.view
        .handle_mouse(mouse(MouseEventKind::ScrollDown, area.x, area.y));
    assert_eq!(env.view.list_offset, 0);
}

#[test]
#[serial]
fn empty_short_and_zero_height_lists_do_not_scroll() {
    crate::tmux::isolate_tmux_socket();
    let mut empty = create_test_env_empty();
    empty.view.list_area = Rect::new(1, 1, 20, 10);
    empty
        .view
        .handle_mouse(mouse(MouseEventKind::ScrollDown, 2, 2));
    assert_eq!(empty.view.list_offset, 0);
    let mut short = create_test_env_with_sessions(2);
    short.view.list_area = Rect::new(1, 1, 20, 10);
    short
        .view
        .handle_mouse(mouse(MouseEventKind::ScrollDown, 2, 2));
    assert_eq!(short.view.list_offset, 0);
    short.view.list_area.height = 0;
    short
        .view
        .handle_mouse(mouse(MouseEventKind::ScrollDown, 2, 2));
    assert_eq!(short.view.list_offset, 0);
}
