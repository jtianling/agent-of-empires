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
