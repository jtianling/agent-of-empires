use super::*;
use futures_util::StreamExt;
use serde_json::Value;

mod compressed;
mod records;

const THREAD: &str = "00000000-0000-4000-8000-000000000001";

fn header(owner: &str, cwd: &Path) -> Value {
    json!({"type": "session_meta", "payload": {"id": owner, "cwd": cwd}})
}

fn record(owner: &str, tier: Option<&str>) -> Value {
    let mut snapshot = json!({"model": "gpt-6.1-sol", "reasoning_effort": "xhigh"});
    if let Some(tier) = tier {
        snapshot["service_tier"] = json!(tier);
    }
    json!({"type": "event_msg", "payload": {"type": "thread_settings_applied",
        "thread_id": owner, "thread_settings": snapshot}})
}

#[test]
fn latest_owned_snapshot_can_turn_ultrafast_off_without_reusing_old_tier() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout.jsonl");
    let records = [
        header(THREAD, dir.path()),
        record(THREAD, Some("ultrafast")),
        record(THREAD, None),
        record("other-thread", Some("priority")),
        json!({"type": "response_item"}),
    ];
    let body = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, body).unwrap();
    let settings = read_persisted_at(&path, THREAD, dir.path())
        .unwrap()
        .unwrap();
    assert_eq!(settings.model, "gpt-6.1-sol");
    assert_eq!(settings.effort.as_deref(), Some("xhigh"));
    assert_eq!(settings.service_tier, None);
}

#[test]
fn incomplete_or_unowned_history_cannot_supply_a_snapshot() {
    let mut unowned = record(THREAD, Some("priority"));
    unowned["payload"]
        .as_object_mut()
        .unwrap()
        .remove("thread_id");
    assert_eq!(settings_from_record(&unowned, THREAD).unwrap(), None);
    let mut invalid = record(THREAD, None);
    invalid["payload"]["thread_settings"]["model"] = json!(null);
    assert!(settings_from_record(&invalid, THREAD).is_err());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollout.jsonl");
    std::fs::write(
        &path,
        format!(
            "{}\n{}\n{{",
            header(THREAD, directory.path()),
            record(THREAD, Some("priority"))
        ),
    )
    .unwrap();
    assert!(read_persisted_at(&path, THREAD, directory.path()).is_err());
}

#[test]
fn settings_before_a_large_tail_are_retained_without_using_another_threads_snapshot() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large-rollout.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "{}", header(THREAD, dir.path())).unwrap();
    writeln!(file, "{}", record(THREAD, Some("ultrafast"))).unwrap();
    writeln!(file, "{}", record(THREAD, None)).unwrap();
    let message = json!({"type": "response_item", "payload": {"text": "x".repeat(8192)}});
    for _ in 0..600 {
        writeln!(file, "{message}").unwrap();
    }
    writeln!(file, "{}", record("other-thread", Some("priority"))).unwrap();
    drop(file);
    assert!(std::fs::metadata(&path).unwrap().len() > 4 * 1024 * 1024);
    let settings = read_persisted_at(&path, THREAD, dir.path())
        .unwrap()
        .unwrap();
    assert_eq!(settings.model, "gpt-6.1-sol");
    assert_eq!(settings.effort.as_deref(), Some("xhigh"));
    assert_eq!(settings.service_tier, None);
}

async fn server(thread: Value, resume: Option<Value>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(3), async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for (method, result) in [
                ("initialize", json!({})),
                ("thread/read", json!({"thread": thread})),
            ] {
                let request = socket.next().await.unwrap().unwrap();
                let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
                assert_eq!(request["method"], method);
                socket
                    .send(Message::Text(
                        json!({"id": request["id"], "result": result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                if method == "initialize" {
                    let initialized = socket.next().await.unwrap().unwrap();
                    assert_eq!(
                        serde_json::from_str::<Value>(initialized.to_text().unwrap()).unwrap()
                            ["method"],
                        "initialized"
                    );
                }
            }
            if let Some(result) = resume {
                let request = socket.next().await.unwrap().unwrap();
                let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
                assert_eq!(request["method"], "thread/resume");
                assert_eq!(
                    request["params"],
                    json!({"threadId": THREAD, "excludeTurns": true})
                );
                socket
                    .send(Message::Text(
                        json!({"id": request["id"], "result": result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
            let remaining = socket.next().await;
            assert!(matches!(
                remaining,
                None | Some(Ok(Message::Close(_))) | Some(Err(_))
            ));
        })
        .await
        .expect("Codex settings RPC timed out");
    });
    (endpoint, task)
}

#[tokio::test]
async fn loaded_capture_reads_live_settings_without_config_overrides() {
    for tier in [None, Some("priority"), Some("ultrafast")] {
        let (endpoint, task) = server(
            json!({"id": THREAD, "status": {"type": "idle"}}),
            Some(json!({"thread": {"id": THREAD},
            "model": "gpt-6.1-sol", "reasoningEffort": "xhigh", "serviceTier": tier})),
        )
        .await;
        let settings = read(&endpoint, THREAD).await.unwrap().unwrap();
        assert_eq!(settings.service_tier.as_deref(), tier);
        assert_eq!(settings.effort.as_deref(), Some("xhigh"));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn unloaded_thread_is_never_resumed_just_to_observe_its_settings() {
    let (endpoint, task) =
        server(json!({"id": THREAD, "status": {"type": "notLoaded"}}), None).await;
    assert_eq!(read(&endpoint, THREAD).await.unwrap(), None);
    task.await.unwrap();
}
