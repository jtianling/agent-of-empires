use super::*;
use std::fs::{self, File};
use std::io::{self, Cursor, Write};

const OTHER: &str = "00000000-0000-4000-8000-000000000002";

fn persisted_with_ids(ids: Value, compressed: bool) -> Result<Option<RuntimeSettings>> {
    let dir = tempfile::tempdir()?;
    let plain = dir.path().join("rollout.jsonl");
    let mut payload = ids;
    payload["cwd"] = json!(dir.path());
    let header = json!({"type": "session_meta", "payload": payload});
    let body = format!("{header}\n{}", record(THREAD, Some("priority")));
    if compressed {
        fs::write(
            plain.with_extension("jsonl.zst"),
            zstd::encode_all(body.as_bytes(), 1)?,
        )?;
    } else {
        fs::write(&plain, body)?;
    }
    read_persisted_at(&plain, THREAD, dir.path())
}

#[test]
fn header_accepts_either_id_spelling_and_requires_agreement_when_both_exist() {
    for compressed in [false, true] {
        for ids in [
            json!({"id": THREAD}),
            json!({"session_id": THREAD}),
            json!({"id": THREAD, "session_id": THREAD}),
        ] {
            let snapshot = persisted_with_ids(ids, compressed).unwrap().unwrap();
            assert_eq!(snapshot.service_tier.as_deref(), Some("priority"));
        }
        for (ids, why) in [
            (json!({}), "Missing Codex metadata id"),
            (json!({"session_id": &THREAD[..8]}), "different thread"),
            (json!({"id": THREAD, "session_id": OTHER}), "Conflicting"),
            (json!({"id": OTHER, "session_id": THREAD}), "Conflicting"),
            (json!({"id": null, "session_id": THREAD}), "invalid type"),
            (json!({"id": THREAD, "session_id": 42}), "invalid type"),
        ] {
            let reason = format!("{:#}", persisted_with_ids(ids, compressed).unwrap_err());
            assert!(
                reason.contains("line 1") && reason.contains(why),
                "{reason}"
            );
        }
    }
}

#[test]
fn record_limit_counts_bytes_before_line_endings_and_stops_without_a_delimiter() {
    let mut buffer = Vec::new();
    for end in ["", "\n", "\r\n"] {
        let mut cursor = Cursor::new(format!("中{end}"));
        assert_eq!(
            read_record(&mut cursor, &mut buffer, 2, 3).unwrap(),
            Some("中")
        );
        assert_eq!(read_record(&mut cursor, &mut buffer, 3, 3).unwrap(), None);
    }
    let mut cursor = Cursor::new([b'x'; 256]);
    let err = read_record(&mut cursor, &mut buffer, 7, 4).unwrap_err();
    assert!(err
        .to_string()
        .contains("line 7 exceeds the 4-byte record limit"));
    assert_eq!(cursor.position(), 6);
}

fn write_oversize(mut out: impl Write, prefix: &[u8], limit: usize) {
    out.write_all(prefix).unwrap();
    io::copy(&mut io::repeat(b' ').take(limit as u64 + 1), &mut out).unwrap();
}

#[test]
fn oversized_unterminated_header_and_later_record_fail_in_both_formats() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("rollout.jsonl");
    let prefix = format!("{}\n{}\n", header(THREAD, dir.path()), record(THREAD, None));
    for (prefix, limit, line) in [
        ("", MAX_HEADER_BYTES, 1),
        (&prefix[..], MAX_RECORD_BYTES, 3),
    ] {
        for compressed in [false, true] {
            let path = if compressed {
                plain.with_extension("jsonl.zst")
            } else {
                plain.clone()
            };
            let file = File::create(&path).unwrap();
            if compressed {
                let mut encoder = zstd::Encoder::new(file, 1).unwrap();
                write_oversize(&mut encoder, prefix.as_bytes(), limit);
                encoder.finish().unwrap();
            } else {
                write_oversize(file, prefix.as_bytes(), limit);
            }
            let reason = read_persisted_at(&plain, THREAD, dir.path())
                .unwrap_err()
                .to_string();
            assert!(reason.contains(&format!(
                "line {line} exceeds the {limit}-byte record limit"
            )));
            fs::remove_file(path).unwrap();
        }
    }
}

#[test]
fn corrupt_record_reports_its_line_and_underlying_cause_after_a_valid_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = format!("{}\n{}\n", header(THREAD, dir.path()), record(THREAD, None));
    let path = dir.path().join("rollout.jsonl.zst");
    for (tail, why) in [
        (&b"\xff"[..], "invalid utf-8"),
        (&b"{"[..], "EOF while parsing"),
    ] {
        let bytes = [prefix.as_bytes(), tail].concat();
        fs::write(&path, zstd::encode_all(&bytes[..], 1).unwrap()).unwrap();
        let reason = format!(
            "{:#}",
            read_persisted_at(&path, THREAD, dir.path()).unwrap_err()
        );
        assert!(
            reason.contains("line 3") && reason.contains(why),
            "{reason}"
        );
    }
}

#[test]
fn history_can_exceed_the_record_limit_and_hold_records_larger_than_the_header_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout.jsonl.zst");
    let mut out = zstd::Encoder::new(File::create(&path).unwrap(), 1).unwrap();
    writeln!(out, "{}", header(THREAD, dir.path())).unwrap();
    let record = json!({"type": "response_item", "payload": "x".repeat(MAX_HEADER_BYTES)});
    let large_line = record.to_string();
    for _ in 0..=MAX_RECORD_BYTES / large_line.len() {
        writeln!(out, "{large_line}").unwrap();
    }
    write!(out, "{}", super::record(THREAD, Some("priority"))).unwrap();
    out.finish().unwrap();
    let snapshot = read_persisted_at(&path, THREAD, dir.path())
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.service_tier.as_deref(), Some("priority"));
}
