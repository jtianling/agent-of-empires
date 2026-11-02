use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::*;

pub(super) type Exchange = (&'static str, Value, Value);

pub(super) async fn serve(exchanges: Vec<Exchange>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let initialized = socket.next().await.unwrap().unwrap();
            let initialized: Value = serde_json::from_str(initialized.to_text().unwrap()).unwrap();
            assert_eq!(initialized["method"], "initialize");
            socket
                .send(Message::Text(
                    json!({"id": initialized["id"], "result": {}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let notification = socket.next().await.unwrap().unwrap();
            let notification: Value =
                serde_json::from_str(notification.to_text().unwrap()).unwrap();
            assert_eq!(notification["method"], "initialized");
            for (method, params, result) in exchanges {
                let message = socket.next().await.unwrap().unwrap();
                let message: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                assert_eq!(message["method"], method);
                assert_eq!(message["params"], params);
                socket
                    .send(Message::Text(
                        json!({"id": message["id"], "result": result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
            let message = socket.next().await;
            assert!(matches!(
                message,
                None | Some(Ok(Message::Close(_))) | Some(Err(_))
            ));
        })
        .await
        .expect("Managed Codex RPC exchange timed out");
    });
    (endpoint, task)
}

fn configuration(provider: &str) -> Value {
    json!({"config": {"model_provider": provider, "mcp_servers": {
        "xats": {"url": "http://127.0.0.1:1234/mcp"}
    }}})
}

fn response(thread_id: &str, provider: &str) -> Value {
    json!({
        "thread": {"id": thread_id},
        "modelProvider": provider,
        "approvalPolicy": "on-request",
        "sandbox": {"type": "workspaceWrite"}
    })
}

fn exchanges(previous: Option<&str>, fresh: bool, provider: &str) -> Vec<Exchange> {
    let mut requests = vec![(
        "config/read",
        json!({"includeLayers": false, "cwd": "/project"}),
        configuration("current-provider"),
    )];
    if let Some(thread_id) = previous {
        requests.push((
            "thread/unsubscribe",
            json!({"threadId": thread_id}),
            json!({"status": "notSubscribed"}),
        ));
    }
    let mut params = json!({"cwd": "/project", "modelProvider": "current-provider",
    "config": {
        "mcp_servers.xats.http_headers": {"X-Xats-Managed-Token": "test-token"},
        "mcp_servers.xats.url": "http://127.0.0.1:1234/mcp"
    }});
    let thread_id = previous
        .filter(|_| !fresh)
        .unwrap_or("019d1af9-a899-7df1-8f7d-a244126e5ded");
    let method = if previous.is_some() && !fresh {
        requests.push((
            "thread/read",
            json!({"threadId": thread_id, "includeTurns": false}),
            json!({"thread": {"id": thread_id, "modelProvider": "current-provider",
                "cwd": "/project", "path": null, "status": {"type": "notLoaded"}}}),
        ));
        params["threadId"] = json!(thread_id);
        params["excludeTurns"] = json!(true);
        "thread/resume"
    } else {
        "thread/start"
    };
    requests.push((method, params, response(thread_id, provider)));
    if provider == "current-provider" {
        requests.push((
            "thread/name/set",
            json!({"threadId": thread_id, "name": "worker"}),
            json!({}),
        ));
    }
    requests
}

#[tokio::test]
async fn uses_current_provider_for_resume_start_and_fresh_restart() {
    let previous = "019d1af9-a899-7df1-8f7d-a244126e5def";
    for (stored, fresh) in [
        (Some(previous), false),
        (None, false),
        (Some(previous), true),
    ] {
        let (endpoint, server) = serve(exchanges(stored, fresh, "current-provider")).await;
        let prepared = prepare_thread(
            &endpoint,
            &url::Url::parse("http://127.0.0.1:1234/mcp").unwrap(),
            "/project",
            "worker",
            "test-token",
            stored,
            fresh,
            false,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            prepared.thread_id,
            stored
                .filter(|_| !fresh)
                .unwrap_or("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
        assert_eq!(prepared.provider, "current-provider");
        assert!(prepared.error.is_none());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn rejects_loaded_thread_that_keeps_the_old_provider() {
    let previous = "019d1af9-a899-7df1-8f7d-a244126e5def";
    let (endpoint, server) = serve(exchanges(Some(previous), false, "old-provider")).await;
    let result = prepare_thread(
        &endpoint,
        &url::Url::parse("http://127.0.0.1:1234/mcp").unwrap(),
        "/project",
        "worker",
        "test-token",
        Some(previous),
        false,
        false,
        None,
        None,
    )
    .await;
    let prepared = result.unwrap();
    assert_eq!(prepared.thread_id, previous);
    assert!(prepared
        .error
        .unwrap()
        .to_string()
        .contains("current model provider"));
    server.await.unwrap();
}

#[test]
fn generated_profile_keeps_the_selected_provider_for_the_remote_tui() {
    let directory = tempfile::tempdir().unwrap();
    let config = ClientConfig {
        thread_id: uuid::Uuid::new_v4().to_string(),
        ws_url: "ws://127.0.0.1:8799".into(),
        profile_name: "test-profile".into(),
        profile_path: directory.path().join("test-profile.config.toml"),
        fresh: false,
        owned: false,
    };
    let mut prepared = PreparedThread {
        thread_id: config.thread_id.clone(),
        provider: "current-provider".into(),
        server_name: "xats".into(),
        headers: json!({"X-Xats-Managed-Token": "test-token"}),
        yolo: true,
        model: Some("gpt-6.1-sol".into()),
        effort: Some("xhigh".into()),
        error: None,
    };
    let cwd = "/project/with spaces.and\"quotes";
    write_profile(&config, &prepared, cwd).unwrap();
    let profile: toml::Value =
        toml::from_str(&fs::read_to_string(&config.profile_path).unwrap()).unwrap();
    assert_eq!(profile["model_provider"].as_str(), Some("current-provider"));
    assert_eq!(profile["model"].as_str(), Some("gpt-6.1-sol"));
    assert_eq!(profile["model_reasoning_effort"].as_str(), Some("xhigh"));
    assert_eq!(profile["approval_policy"].as_str(), Some("never"));
    assert_eq!(profile["sandbox_mode"].as_str(), Some("danger-full-access"));
    assert_eq!(
        profile["projects"][cwd]["trust_level"].as_str(),
        Some("trusted")
    );
    assert_eq!(profile["projects"].as_table().unwrap().len(), 1);
    assert_eq!(
        profile["mcp_servers"]["xats"]["http_headers"][HEADER].as_str(),
        Some("test-token")
    );
    prepared.yolo = false;
    write_profile(&config, &prepared, cwd).unwrap();
    let profile: toml::Value =
        toml::from_str(&fs::read_to_string(&config.profile_path).unwrap()).unwrap();
    assert!(profile.get("projects").is_none());
}

#[tokio::test]
async fn requested_model_is_checked_for_new_and_resumed_threads() {
    let previous = "019d1af9-a899-7df1-8f7d-a244126e5def";
    for stored in [None, Some(previous)] {
        for returned in ["gpt-6.1-terra", "another-model"] {
            let mut calls = exchanges(stored, false, "current-provider");
            let index = calls
                .iter()
                .position(|(method, _, _)| matches!(*method, "thread/start" | "thread/resume"))
                .unwrap();
            calls[index].1["model"] = json!("gpt-6.1-terra");
            calls[index].2["model"] = json!(returned);
            if returned != "gpt-6.1-terra" {
                calls.pop();
            }
            let (endpoint, server) = serve(calls).await;
            let prepared = prepare_thread(
                &endpoint,
                &url::Url::parse("http://127.0.0.1:1234/mcp").unwrap(),
                "/project",
                "worker",
                "test-token",
                stored,
                false,
                false,
                Some("gpt-6.1-terra"),
                None,
            )
            .await
            .unwrap();
            assert_eq!(prepared.error.is_none(), returned == "gpt-6.1-terra");
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn verifies_explicit_effort_for_sol_and_terra_on_start_and_resume() {
    let previous = "019d1af9-a899-7df1-8f7d-a244126e5def";
    for model in ["gpt-6.1-sol", "gpt-6.1-terra"] {
        for (stored, fresh) in [
            (None, false),
            (Some(previous), false),
            (Some(previous), true),
        ] {
            for returned in ["xhigh", "none"] {
                let mut calls = exchanges(stored, fresh, "current-provider");
                let (_, params, response) = calls
                    .iter_mut()
                    .find(|(method, _, _)| matches!(*method, "thread/start" | "thread/resume"))
                    .unwrap();
                params["model"] = json!(model);
                params["config"]["model_reasoning_effort"] = json!("xhigh");
                response["model"] = json!(model);
                response["reasoningEffort"] = json!(returned);
                if returned != "xhigh" {
                    calls.pop();
                }
                let (endpoint, server) = serve(calls).await;
                let prepared = prepare_thread(
                    &endpoint,
                    &url::Url::parse("http://127.0.0.1:1234/mcp").unwrap(),
                    "/project",
                    "worker",
                    "test-token",
                    stored,
                    fresh,
                    false,
                    Some(model),
                    Some("xhigh"),
                )
                .await
                .unwrap();
                assert_eq!(prepared.error.is_none(), returned == "xhigh");
                server.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn prepares_and_verifies_yolo_on_start_and_resume() {
    let previous = "019d1af9-a899-7df1-8f7d-a244126e5def";
    for stored in [None, Some(previous)] {
        let mut requests = exchanges(stored, false, "current-provider");
        let (_, params, response) = requests
            .iter_mut()
            .find(|(method, _, _)| matches!(*method, "thread/start" | "thread/resume"))
            .unwrap();
        params["approvalPolicy"] = json!("never");
        params["sandbox"] = json!("danger-full-access");
        response["approvalPolicy"] = json!("never");
        response["sandbox"] = json!({"type": "dangerFullAccess"});
        let (endpoint, server) = serve(requests).await;
        let prepared = prepare_thread(
            &endpoint,
            &url::Url::parse("http://127.0.0.1:1234/mcp").unwrap(),
            "/project",
            "worker",
            "test-token",
            stored,
            false,
            true,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(prepared.yolo);
        assert!(prepared.error.is_none());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn retains_created_thread_when_yolo_is_rejected() {
    let mut requests = exchanges(None, false, "current-provider");
    requests.pop();
    let (_, params, _) = requests.last_mut().unwrap();
    params["approvalPolicy"] = json!("never");
    params["sandbox"] = json!("danger-full-access");
    let (endpoint, server) = serve(requests).await;
    let prepared = prepare_thread(
        &endpoint,
        &url::Url::parse("http://127.0.0.1:1234/mcp").unwrap(),
        "/project",
        "worker",
        "test-token",
        None,
        false,
        true,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(prepared.thread_id, "019d1af9-a899-7df1-8f7d-a244126e5ded");
    assert!(prepared
        .error
        .unwrap()
        .to_string()
        .contains("Codex did not retain the requested YOLO permissions"));
    server.await.unwrap();
}
