use super::{command::encode, frame::response};
use std::io::{BufReader, Cursor, ErrorKind};

const FENCE: &str = "%begin 30 8 1\nnonce\n%end 30 8 1\n";

#[test]
fn pane_bytes_and_fake_control_frames_survive_fragmented_reads() {
    let body = b"\nquote\r\n%exit\n%error 30 7 1\n%begin 30 8 1\nold nonce\n%end 30 8 1\n%end 30 7 1\n%output %1 hi\n\xff\n";
    let input = [
        b"%output %3 async\n%begin 30 7 1\n".as_slice(),
        body,
        b"%end 30 7 1\n%session-changed $1 test\n",
        FENCE.as_bytes(),
    ]
    .concat();
    let mut reader = BufReader::with_capacity(3, Cursor::new(input));
    let out = response(&mut reader, 1, b"nonce").unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, body);
}

#[test]
fn command_failure_does_not_consume_next_response() {
    let input = format!(
        "%begin 30 7 1\ncan't find pane\n%error 30 7 1\n{FENCE}{}",
        "%begin 30 9 1\n%end 30 9 1\n%begin 30 10 1\nnext\n%end 30 10 1\n",
    );
    let mut reader = input.as_bytes();
    let out = response(&mut reader, 1, b"nonce").unwrap();
    assert!(!out.status.success());
    assert_eq!(out.stderr, b"can't find pane\n");
    assert!(response(&mut reader, 1, b"next").unwrap().status.success());
}

#[test]
fn startup_consumes_command_group_and_notifications_before_barrier() {
    let input = format!(
        "%begin 1 2 0\n%end 1 2 0\n%begin 1 3 0\nmore\n%end 1 3 0\n%sessions-changed\n{FENCE}"
    );
    let out = response(&mut input.as_bytes(), 0, b"nonce").unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn mismatched_or_missing_guards_cannot_complete_a_response() {
    for (before, after) in [
        ("%end 30 6 1\n", FENCE),
        ("%end 31 7 1\n", FENCE),
        ("%end 30 7 0\n", FENCE),
        ("%end 30 7 1\nextra\n", FENCE),
        ("%end 30 7 1\n", "%begin 30 8 0\nnonce\n%end 30 8 0\n"),
        ("%end 30 7 1\n", "%begin 30 8 1\nnonce\n%end 30 9 1\n"),
        ("%end 30 7 1\n", "%begin 30 8 1\nnonce\n%error 30 8 1\n"),
    ] {
        let input = format!("%begin 30 7 1\n{before}{after}");
        assert_eq!(
            response(&mut input.as_bytes(), 1, b"nonce")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
    }
}

#[test]
fn disconnect_preserves_startup_diagnostic() {
    let mut input = b"%begin 1 2 0\ncan't find session\n%error 1 2 0\n%exit\n".as_slice();
    let error = response(&mut input, 0, b"nonce").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert!(error.to_string().contains("can't find session"));
}

#[test]
fn unterminated_line_is_bounded() {
    let data = vec![b'x'; 8 * 1024 * 1024 + 1];
    let mut input = data.as_slice();
    assert_eq!(
        response(&mut input, 1, b"nonce").unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(input.len(), 1);
}

#[test]
fn encodes_arguments_without_interpretation_and_protects_titles() {
    let title = "'\"\n\r\t\\; $HOME 中文 #{pane_id} #[red] ~";
    let cmd = encode(&["select-pane", "-t", "%1", "-T", title]).unwrap();
    assert_eq!(cmd, "select-pane \"-t\" \"%1\" \"-T\" \"'\\042\\012\\015\\011\\134; \\044HOME 中文 #{a:35}{pane_id} #{a:35}[red] \\176\"\n");
    let key = "@aoe_notification_monitor_pid_中 文.a#{pane_id}";
    let value = "#{pane_id}";
    let cmd = encode(&["set-option", "-g", key, value]).unwrap();
    assert!(cmd.ends_with(
        " \"--\" \"@aoe_notification_monitor_pid_中 文.a#{a:35}{pane_id}\" \"#{pane_id}\"\n"
    ));
    assert!(encode(&["show-options", "-gqv", key])
        .unwrap()
        .contains("中 文.a#{a:35}"));
    assert!(encode(&["list-clients", "-f", "#{?@aoe_control_observer,0,1}"]).is_ok());
    assert!(encode(&["list-panes", "-F", "#{?#{m:aoe_*,#{session_name}},1,0}"]).is_ok());
}

#[test]
fn rejects_invalid_input_before_writing() {
    for args in [
        &[][..],
        &["select-pane; display-message"],
        &["run-shell", "echo x"],
        &["set-option", "-F", "@key", "x"],
        &["capture-pane", "-S", "#{history_size}"],
        &["set-option", "-t", "%1", "@key", "x", ";", "list-panes"],
        &["set-option", "@key", "a\0b"],
    ] {
        assert_eq!(encode(args).unwrap_err().kind(), ErrorKind::InvalidInput);
    }
}

#[test]
fn startup_reports_failure_in_a_later_initial_command() {
    let data =
        format!("%begin 2 2 0\n%end 2 2 0\n%begin 2 3 0\nmissing pane\n%error 2 3 0\n{FENCE}");
    assert!(response(&mut data.as_bytes(), 0, b"nonce")
        .unwrap_err()
        .to_string()
        .contains("missing pane"));
}

#[test]
fn native_option_and_title_slots_expand_only_literal_text() {
    let raw = "@aoe_notification_monitor_pid_p#{pid};";
    let literal = "@aoe_notification_monitor_pid_p#{a:35}{pid}#{a:59}";
    for (args, suffix) in [
        (
            vec!["set-option", "-gq", raw, "#{value}"],
            vec!["--", literal, "#{value}"],
        ),
        (vec!["show-option", "-gv", raw], vec!["--", literal]),
        (vec!["set-option", "-gqu", raw], vec!["--", literal]),
        (
            vec!["select-pane", "-t", "%1", "-T", raw],
            vec!["-T", literal],
        ),
    ] {
        let cmd = super::super::native_command(None, &args).unwrap();
        let argv: Vec<_> = cmd.get_args().map(|arg| arg.to_str().unwrap()).collect();
        assert!(argv.ends_with(&suffix), "{argv:?}");
    }
}

#[test]
fn native_profile_query_keeps_utf8_flag_and_trusted_format() {
    let cmd = super::super::native_command(Some("p#{pid}"), &["list-panes", "-F", "#{pane_title}"])
        .unwrap();
    let argv: Vec<_> = cmd.get_args().map(|arg| arg.to_str().unwrap()).collect();
    assert!(argv.ends_with(&["-u", "list-panes", "-F", "#{pane_title}"]));
}
