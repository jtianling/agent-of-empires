use super::*;
use ratatui::backend::TestBackend;
use tui_input::InputRequest;

fn render(input: &Input, focused: bool, width: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, 2)).unwrap();
    terminal
        .draw(|frame| {
            render_text_field_with_ghost(
                frame,
                frame.area(),
                "Directory:",
                input,
                focused,
                Some("/path/to/project"),
                Some("-completion/"),
                &Theme::default(),
            );
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, 0)].symbol())
        .collect()
}

#[test]
fn label_underline_does_not_fill_spacing_or_blank_rows() {
    let buffer = render(&Input::default(), true, 32);
    for x in 0..10 {
        assert!(buffer[(x, 0)].modifier.contains(Modifier::UNDERLINED));
    }
    assert!(!buffer[(10, 0)].modifier.contains(Modifier::UNDERLINED));
    for x in 0..32 {
        assert_eq!(buffer[(x, 1)].symbol(), " ");
        assert!(!buffer[(x, 1)].modifier.contains(Modifier::UNDERLINED));
    }
}

#[test]
fn long_directory_tracks_cursor_and_keeps_label_visible() {
    let value = "/Users/jtianling/workspace/monkeys-workspace/monkeys";
    let mut input = Input::new(value.into());
    let theme = Theme::default();
    let end = render(&input, true, 32);
    assert_eq!(text(&end), "Directory: ys-workspace/monkeys ");
    assert_eq!(end[(31, 0)].bg, theme.accent);

    input.handle(InputRequest::GoToStart);
    let start = render(&input, true, 32);
    assert!(text(&start).starts_with("Directory: /Users/jtianling/"));
    assert_eq!(start[(11, 0)].bg, theme.accent);

    input.handle(InputRequest::GoToEnd);
    input.handle(InputRequest::DeletePrevChar);
    input.handle(InputRequest::InsertChar('X'));
    let edited = render(&input, true, 32);
    assert!(text(&edited).ends_with("/monkeyX "));
    assert_eq!(edited[(31, 0)].bg, theme.accent);
    assert_eq!(
        input.value(),
        value.strip_suffix('s').unwrap().to_owned() + "X"
    );
}

#[test]
fn wide_cursor_character_stays_visible_at_right_edge() {
    let mut input = Input::new("/very/long/目录/中文后缀".into());
    input.handle(InputRequest::GoToStart);
    let theme = Theme::default();
    for character in input.value().to_string().chars() {
        let buffer = render(&input, true, 24);
        let cursor_x = (11..24)
            .find(|&x| buffer[(x, 0)].bg == theme.accent)
            .expect("cursor must remain visible");
        assert_eq!(buffer[(cursor_x, 0)].symbol(), character.to_string());
        input.handle(InputRequest::GoToNextChar);
    }
}

#[test]
fn unfocused_directory_starts_at_beginning_and_narrow_fields_are_safe() {
    let input = Input::new("/Users/jtianling/workspace/monkeys-workspace".into());
    assert!(text(&render(&input, false, 32)).starts_with("Directory: /Users/"));
    for width in 1..=12 {
        let buffer = render(&input, true, width);
        assert_eq!(buffer.area.width, width);
    }
}
