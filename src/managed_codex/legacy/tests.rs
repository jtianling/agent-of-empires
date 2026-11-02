use super::*;
use crate::managed_codex::{provider_tests::serve, ClientConfig};

const THREAD: &str = "019d1af9-a899-7df1-8f7d-a244126e5def";

fn config(endpoint: String) -> ClientConfig {
    ClientConfig {
        thread_id: THREAD.into(),
        ws_url: endpoint,
        profile_name: "test".into(),
        profile_path: "unused".into(),
        fresh: false,
        owned: false,
    }
}

fn response(status: &str) -> Value {
    json!({"thread": {"id": THREAD, "status": {"type": status}}})
}

#[tokio::test]
async fn idle_preflight_does_not_subscribe_or_change_configuration() {
    let (endpoint, server) = serve(vec![(
        "thread/read",
        json!({"threadId": THREAD, "includeTurns": false}),
        response("idle"),
    )])
    .await;
    tokio::task::spawn_blocking(move || preflight(&config(endpoint)))
        .await
        .unwrap()
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn active_preflight_rejects_before_subscription_or_configuration_changes() {
    let (endpoint, server) = serve(vec![(
        "thread/read",
        json!({"threadId": THREAD, "includeTurns": false}),
        response("active"),
    )])
    .await;
    let error = tokio::task::spawn_blocking(move || preflight(&config(endpoint)))
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("Cannot migrate active legacy"));
    server.await.unwrap();
}

#[tokio::test]
async fn retire_applies_zero_delay_only_to_exact_thread_and_waits_for_its_unload() {
    let (endpoint, server) = serve(vec![
        (
            "thread/read",
            json!({"threadId": THREAD, "includeTurns": false}),
            response("idle"),
        ),
        (
            "thread/resume",
            json!({"threadId": THREAD, "excludeTurns": true,
            "config": {"thread_unload_delay_secs": 0}}),
            response("idle"),
        ),
        (
            "thread/unsubscribe",
            json!({"threadId": THREAD}),
            json!({"status": "unsubscribed"}),
        ),
        (
            "thread/loaded/list",
            json!({}),
            json!({"data": [THREAD], "nextCursor": null}),
        ),
        (
            "thread/loaded/list",
            json!({}),
            json!({
            "data": ["019d1af9-a899-7df1-8f7d-a244126e5ded"], "nextCursor": null}),
        ),
    ])
    .await;
    tokio::task::spawn_blocking(move || retire(&config(endpoint)))
        .await
        .unwrap()
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn already_unloaded_legacy_thread_is_not_resumed() {
    let (endpoint, server) = serve(vec![(
        "thread/read",
        json!({"threadId": THREAD, "includeTurns": false}),
        response("notLoaded"),
    )])
    .await;
    tokio::task::spawn_blocking(move || retire(&config(endpoint)))
        .await
        .unwrap()
        .unwrap();
    server.await.unwrap();
}
