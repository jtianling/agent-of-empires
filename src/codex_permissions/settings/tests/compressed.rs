use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

const OTHER: &str = "00000000-0000-4000-8000-000000000002";

fn encode(bytes: &[u8]) -> Vec<u8> {
    zstd::encode_all(bytes, 1).unwrap()
}

fn rollout(cwd: &Path, thread: &str) -> String {
    let records = [
        header(thread, cwd),
        record(THREAD, None),
        json!({"type": "response_item", "payload": {"text": "中".repeat(9000)}}),
        record(THREAD, Some("priority")),
        record(OTHER, Some("ultrafast")),
    ];
    records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn sidecar_and_explicit_path_read_across_frames_without_materializing_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("rollout.jsonl");
    let compressed = plain.with_extension("jsonl.zst");
    let body = rollout(dir.path(), THREAD);
    let split = body.find('中').unwrap() + 1;
    let mut bytes = encode(&body.as_bytes()[..split]);
    bytes.extend(encode(&body.as_bytes()[split..]));
    fs::write(&compressed, &bytes).unwrap();
    fs::set_permissions(&compressed, fs::Permissions::from_mode(0o400)).unwrap();
    for path in [&plain, &compressed] {
        let snapshot = read_persisted_at(path, THREAD, dir.path())
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.service_tier.as_deref(), Some("priority"));
    }
    assert_eq!(fs::read(&compressed).unwrap(), bytes);
    assert_eq!(
        fs::metadata(&compressed).unwrap().permissions().mode() & 0o777,
        0o400
    );
    assert!(!plain.exists());
}

#[test]
fn truncated_frame_or_json_after_valid_settings_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout.jsonl.zst");
    let mut bytes = encode(rollout(dir.path(), THREAD).as_bytes());
    let tail = encode(b"\n{}\n");
    bytes.extend_from_slice(&tail[..tail.len() - 1]);
    fs::write(&path, bytes).unwrap();
    let error = read_persisted_at(&path, THREAD, dir.path()).unwrap_err();
    assert_eq!(
        error
            .root_cause()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    fs::write(
        &path,
        encode(format!("{}\n{{", rollout(dir.path(), THREAD)).as_bytes()),
    )
    .unwrap();
    assert!(format!(
        "{:#}",
        read_persisted_at(&path, THREAD, dir.path()).unwrap_err()
    )
    .contains("Invalid JSON in Codex settings history at line 6"));
}

#[test]
fn plain_errors_take_precedence_over_a_valid_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("rollout.jsonl");
    let compressed = plain.with_extension("jsonl.zst");
    fs::write(&compressed, encode(rollout(dir.path(), THREAD).as_bytes())).unwrap();
    for (body, expected) in [
        ("{".into(), "metadata"),
        (rollout(dir.path(), OTHER), "different thread"),
        (
            rollout(dir.path().parent().unwrap(), THREAD),
            "different directory",
        ),
    ] {
        fs::write(&plain, body).unwrap();
        let error = read_persisted_at(&plain, THREAD, dir.path()).unwrap_err();
        assert!(format!("{error:#}").contains(expected));
    }
    fs::write(
        &plain,
        format!("{}\n{}", header(THREAD, dir.path()), record(OTHER, None)),
    )
    .unwrap();
    assert_eq!(read_persisted_at(&plain, THREAD, dir.path()).unwrap(), None);
    fs::remove_file(&plain).unwrap();
    fs::write(&compressed, encode(rollout(dir.path(), OTHER).as_bytes())).unwrap();
    assert!(read_persisted_at(&plain, THREAD, dir.path()).is_err());
}

#[tokio::test]
async fn unloaded_rpc_reads_the_compressed_snapshot_for_the_exact_thread_and_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("rollout.jsonl");
    let packed = plain.with_extension("jsonl.zst");
    fs::write(&packed, encode(rollout(dir.path(), THREAD).as_bytes())).unwrap();
    for path in [&plain, &packed] {
        let thread = json!({"id": THREAD, "cwd": dir.path(), "path": path,
                            "status": {"type": "notLoaded"}});
        let (endpoint, task) = server(thread, None).await;
        let settings = read(&endpoint, THREAD).await.unwrap().unwrap();
        assert_eq!(settings.service_tier.as_deref(), Some("priority"));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn compressed_settings_restore_requires_the_requested_permissions_on_the_original_thread() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("rollout.jsonl");
    fs::write(
        plain.with_extension("jsonl.zst"),
        encode(rollout(dir.path(), THREAD).as_bytes()),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let cwd = dir.path().to_str().unwrap();
    for granted in [false, true] {
        let settings = read_persisted_at(&plain, THREAD, dir.path()).unwrap();
        let client = async {
            let (mut socket, _) = connect_async(&endpoint).await.unwrap();
            crate::codex_permissions::restore_thread(
                &mut socket,
                THREAD,
                cwd,
                true,
                &endpoint,
                settings,
            )
            .await
        };
        let server = serve_restore(&listener, cwd, granted);
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            tokio::join!(client, server)
        })
        .await
        .unwrap();
        if granted {
            result.unwrap();
        } else {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("requested YOLO permissions"));
        }
    }
}

async fn serve_restore(listener: &tokio::net::TcpListener, cwd: &str, granted: bool) {
    let (stream, _) = listener.accept().await.unwrap();
    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
    let resume = json!({"threadId": THREAD, "cwd": cwd, "serviceTier": "priority"});
    let response = |sandbox| {
        json!({"thread": {"id": THREAD}, "model": "gpt-6.1-sol",
        "reasoningEffort": "xhigh", "serviceTier": "priority", "modelProvider": "local",
        "approvalPolicy": "never", "sandbox": {"type": sandbox}})
    };
    let expected_sandbox = if granted {
        "dangerFullAccess"
    } else {
        "workspaceWrite"
    };
    let history = json!({"thread": {"id": THREAD, "modelProvider": "local",
        "cwd": cwd, "status": {"type": "notLoaded"}}});
    let permission = json!({"threadId": THREAD, "approvalPolicy": "never",
        "sandboxPolicy": {"type": "dangerFullAccess"}});
    for (method, expected_params, result) in [
        (
            "config/read",
            json!({"cwd": cwd}),
            json!({"config": {"model_provider": "local"}}),
        ),
        ("thread/read", json!({"threadId": THREAD}), history),
        ("thread/resume", resume.clone(), response("workspaceWrite")),
        ("thread/settings/update", permission, json!({})),
        ("thread/resume", resume, response(expected_sandbox)),
    ] {
        let frame = socket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
        assert_eq!(request["method"], method);
        for (field, value) in expected_params.as_object().unwrap() {
            assert_eq!(&request["params"][field], value);
        }
        let reply = json!({"id": request["id"], "result": result});
        socket
            .send(Message::Text(reply.to_string().into()))
            .await
            .unwrap();
    }
}
