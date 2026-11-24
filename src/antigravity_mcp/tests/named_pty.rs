use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

const TEAM: &str = "-lab/+ &x";
const LAUNCH: &str = "5bf66983-ad13-4af3-8a41-865b86bd39e4";
const KEY: &str = "9910463c-c901-4cbe-bb42-f507d4ff4b5d";
const AGENT: &str = "86f6ef9e-9e87-4226-bd5b-102e552f9d39";

fn launch() -> named::Launch {
    named::Launch::new(LAUNCH.into(), "-reader".into(), TEAM.into(), AGENT).unwrap()
}

fn handshake(r: &Request) -> Option<Response> {
    if r.path != "/mcp" {
        assert_eq!(r.headers["authorization"], format!("Bearer {TOKEN}"));
        let body = match (r.verb.as_str(), r.path.as_str()) {
            ("POST", "/api/identity-key/lookup") => {
                assert_eq!(r.msg, json!({"protocol_version":1,"identity_key":KEY}));
                json!({"ok":true,"protocol_version":1,"identity_key":KEY,
                    "holder":{"name":"-reader","team":TEAM,"agent_type":"custom","active":false}})
            }
            ("GET", "/api/agents?team=-lab%2F%2B+%26x") => {
                assert!(r.msg.is_null());
                json!({"agents":[{"agent_id":AGENT, "name":"-reader","team":TEAM,
                    "agent_type":"custom","agent_type_name":"Antigravity"}]})
            }
            other => panic!("Unexpected directory request: {other:?}"),
        };
        return Some(Response {
            status: 200,
            headers: "Content-Type: application/json\r\n".into(),
            body: body.to_string(),
        });
    }
    if r.msg["method"] == "tools/list" {
        return Some(Response::rpc(
            r,
            "result",
            json!({"tools":[{
                "name":"register_agent", "inputSchema":{"type":"object","properties":{
                    "agent_type":{"type":"string","enum":["custom","antigravity"]},
                    "agy_launch_id":{"type":"string"}, "name":{"type":"string"},
                    "team":{"type":"string"}, "identity_key":{"type":"string"}
                }}
            }]}),
        ));
    }
    Response::handshake(r)
}

fn assert_registration(r: &Request) {
    assert_eq!(r.headers["authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(
        r.msg["params"],
        json!({"name":"register_agent","arguments":{
            "agent_type":"antigravity","agy_launch_id":LAUNCH,"identity_key":KEY,
            "name":"-reader","team":TEAM
        }})
    );
}

#[test]
fn named_arguments_require_an_exact_launch_uuid_v4_and_both_labels() {
    for id in [
        "",
        "not-a-uuid",
        "10a39009-d75b-11ed-afa1-0242ac120002",
        "5bf66983-ad13-4af3-7a41-865b86bd39e4",
    ] {
        assert!(named::Launch::new(id.into(), "a".into(), "b".into(), AGENT).is_err());
    }
    for (name, team) in [("", "dev"), ("a", ""), (" a", "dev"), ("a", "dev ")] {
        assert!(named::Launch::new(LAUNCH.into(), name.into(), team.into(), AGENT).is_err());
    }
}

#[tokio::test]
async fn idle_named_bridge_registers_and_reopens_without_model_calls() {
    let pings = AtomicUsize::new(0);
    let (tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let server = Server::new(move |r| {
        if let Some(reply) = handshake(r) {
            return reply;
        }
        if r.msg["method"] == "ping" {
            tx.send(r.sid().to_owned()).unwrap();
            if pings.fetch_add(1, Ordering::SeqCst) == 1 {
                return Response::status(404);
            }
            return Response::rpc(r, "result", json!({}));
        }
        assert_registration(r);
        Response::tool(r, json!({"agent_id":AGENT,"team":TEAM}))
    })
    .await;
    let mut c = Client::start(
        server.named_bridge(launch(), KEY.into()),
        Duration::from_millis(50),
    );
    assert!(c.init().await["result"].is_object());
    let sessions = tokio::time::timeout(Duration::from_secs(4), async {
        let mut sessions = Vec::new();
        for _ in 0..3 {
            sessions.push(seen.recv().await.unwrap());
        }
        sessions
    })
    .await
    .unwrap();
    assert_eq!(sessions[0], sessions[1]);
    assert_ne!(sessions[1], sessions[2]);

    let tools = c.rpc("tools/list", json!({})).await["result"]["tools"].clone();
    let tools = tools.as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "send_message"));
    for name in ["reconnect", "register_agent"] {
        assert!(!tools.iter().any(|t| t["name"] == name));
        assert_eq!(c.call(name, json!({})).await["error"]["code"], -32602);
    }
    c.close().await;
    let log = server.log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|r| r.msg["method"] == "tools/call")
            .count(),
        2
    );
    assert_eq!(log.last().unwrap().verb, "DELETE");
}

#[tokio::test]
async fn named_transport_never_replays_a_send_or_accepts_a_changed_agent() {
    let registrations = AtomicUsize::new(0);
    let server = Server::new(move |r| {
        if let Some(reply) = handshake(r) {
            return reply;
        }
        if r.msg["method"] == "ping" {
            return Response::rpc(r, "result", json!({}));
        }
        if r.msg["params"]["name"] == "send_message" {
            return Response::status(404);
        }
        assert_registration(r);
        let id = if registrations.fetch_add(1, Ordering::SeqCst) == 0 {
            AGENT
        } else {
            LAUNCH
        };
        Response::rpc(
            r,
            "result",
            json!({
                "content":[], "structuredContent":{"agent_id":id,"team":TEAM}
            }),
        )
    })
    .await;
    let mut c = Client::start(server.named_bridge(launch(), KEY.into()), IDLE);
    assert!(c.init().await["result"].is_object());
    let send = c
        .call(
            "send_message",
            json!({"to_agent_name":"peer","body":"once"}),
        )
        .await;
    assert!(send["error"]["message"].as_str().unwrap().contains("404"));
    let list = c.call("list_agents", json!({})).await;
    assert!(list["error"]["message"]
        .as_str()
        .unwrap()
        .contains("different identity"));
    c.close().await;
    let log = server.log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|r| r.msg["params"]["name"] == "send_message")
            .count(),
        1
    );
}

#[tokio::test]
async fn named_registration_rejection_is_redacted_and_blocks_discovery() {
    let server = Server::new(|r| {
        if let Some(reply) = handshake(r) {
            return reply;
        }
        assert_registration(r);
        Response::tool(
            r,
            json!({"error":"agy_launcher_not_ready",
            "detail":format!("{KEY}/{TOKEN}/{}",r.sid())}),
        )
    })
    .await;
    let mut c = Client::start(server.named_bridge(launch(), KEY.into()), IDLE);
    assert!(c.init().await["result"].is_object());
    for reply in [
        c.rpc("tools/list", json!({})).await,
        c.call("get_inbox", json!({})).await,
    ] {
        let msg = reply["error"]["message"].as_str().unwrap();
        assert!(msg.contains("agy_launcher_not_ready"));
        assert!(msg.contains("[redacted]/[redacted]/[redacted]"));
    }
    c.close().await;
}

#[tokio::test]
async fn named_capability_probe_rejects_an_old_daemon_before_registration() {
    let server = Server::new(|r| {
        if let Some(reply) = Response::handshake(r) {
            return reply;
        }
        assert_eq!(r.msg["method"], "tools/list");
        if r.msg["params"].get("cursor").is_none() {
            return Response::rpc(r, "result", json!({"tools":[], "nextCursor":"next"}));
        }
        assert_eq!(r.msg["params"], json!({"cursor":"next"}));
        Response::rpc(
            r,
            "result",
            json!({"tools":[{
                "name":"register_agent", "inputSchema":{"properties":{
                    "agent_type":{"enum":["custom","antigravity"]}
                }}
            }]}),
        )
    })
    .await;
    let mut c = Client::start(server.named_bridge(launch(), KEY.into()), IDLE);
    assert!(c.init().await["result"].is_object());
    let reply = c.call("get_inbox", json!({})).await;
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .contains("lacks named"));
    c.close().await;
    let log = server.log.lock().unwrap();
    assert!(log.iter().all(|r| !r.msg.to_string().contains(KEY)));
    assert_eq!(log.last().unwrap().verb, "DELETE");
}

#[tokio::test]
async fn named_attach_rejects_directory_drift_and_first_registration_mismatch() {
    for drift_before_register in [true, false] {
        let server = Server::new(move |request| {
            if drift_before_register && request.verb == "GET" {
                let mut response = handshake(request).unwrap();
                response.body = response.body.replace(AGENT, LAUNCH);
                return response;
            }
            if let Some(response) = handshake(request) {
                return response;
            }
            assert_registration(request);
            Response::tool(request, json!({"agent_id":LAUNCH,"team":TEAM}))
        })
        .await;
        let mut client = Client::start(server.named_bridge(launch(), KEY.into()), IDLE);
        client.init().await;
        let reply = client.call("get_inbox", json!({})).await;
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(
            message.contains(if drift_before_register {
                "identity changed before registration"
            } else {
                "registration returned a different identity"
            }),
            "{message}"
        );
        client.close().await;
        let log = server.log.lock().unwrap();
        assert!(log.iter().any(|r| r.verb == "GET"));
        assert_eq!(
            log.iter()
                .any(|r| r.msg["params"]["name"] == "register_agent"),
            !drift_before_register
        );
        assert!(log.iter().all(|r| r.msg["params"]["name"] != "get_inbox"));
        assert_eq!(log.last().unwrap().verb, "DELETE");
    }
}

#[tokio::test]
async fn named_bridge_forwards_upstream_communication_tools_and_await_ack() {
    let server = Server::new(|r| {
        if r.path == "/mcp" && r.msg["method"] == "tools/list" {
            return Response::rpc(
                r,
                "result",
                json!({"tools":[
                    {"name":"register_agent", "inputSchema":{"type":"object","properties":{
                        "agent_type":{"type":"string","enum":["custom","antigravity"]},
                        "agy_launch_id":{"type":"string"}
                    }}},
                    {"name":"send_message", "description":"upstream-send-desc", "inputSchema":{"type":"object"}},
                    {"name":"broadcast_to_role", "description":"upstream-role-desc", "inputSchema":{"type":"object"}},
                    {"name":"get_delivery_status", "description":"upstream-status-desc", "inputSchema":{"type":"object"}},
                    {"name":"cancel_unread_alert", "description":"upstream-cancel-desc", "inputSchema":{"type":"object"}},
                    {"name":"reconnect", "description":"must-be-filtered", "inputSchema":{"type":"object"}}
                ]}),
            );
        }
        if let Some(reply) = handshake(r) {
            return reply;
        }
        if r.msg["method"] == "ping" {
            return Response::rpc(r, "result", json!({}));
        }
        match r.msg["params"]["name"].as_str().unwrap() {
            "register_agent" => {
                assert_registration(r);
                Response::tool(r, json!({"agent_id":AGENT,"team":TEAM}))
            }
            "send_message" => Response::tool(r, json!({"accepted":r.msg["params"]["arguments"]})),
            "cancel_unread_alert" => {
                Response::tool(r, json!({"cancelled":r.msg["params"]["arguments"]}))
            }
            other => panic!("unexpected tool: {other}"),
        }
    })
    .await;
    let mut c = Client::start(server.named_bridge(launch(), KEY.into()), IDLE);
    assert!(c.init().await["result"].is_object());
    let tools = c.rpc("tools/list", json!({})).await["result"]["tools"].clone();
    let tools = tools.as_array().unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(
        names,
        vec![
            "send_message",
            "broadcast_to_role",
            "get_delivery_status",
            "cancel_unread_alert"
        ]
    );
    assert_eq!(tools[0]["description"], "upstream-send-desc");
    let send = payload(
        c.call(
            "send_message",
            json!({"to_agent_name":"peer","body":"hi","await_ack_s":0}),
        )
        .await,
    );
    assert_eq!(send["accepted"]["await_ack_s"], 0);
    let cancel = payload(
        c.call(
            "cancel_unread_alert",
            json!({"message_id":"msg-1","same_recipient_pending":true}),
        )
        .await,
    );
    assert_eq!(cancel["cancelled"]["message_id"], "msg-1");
    assert_eq!(
        c.call(
            "send_message",
            json!({"to_agent_name":"peer","body":"hi","await_ack_s":31}),
        )
        .await["error"]["code"],
        -32602
    );
    c.close().await;
}

#[tokio::test]
async fn named_registration_retries_transient_handoff_collision_then_succeeds() {
    let attempts = AtomicUsize::new(0);
    let server = Server::new(move |r| {
        if let Some(reply) = handshake(r) {
            return reply;
        }
        if r.msg["method"] == "ping" {
            return Response::rpc(r, "result", json!({}));
        }
        match r.msg["params"]["name"].as_str().unwrap() {
            "register_agent" => {
                assert_registration(r);
                match attempts.fetch_add(1, Ordering::SeqCst) {
                    0 => Response::tool(r, json!({"error": "agent_id_collision"})),
                    1 => Response::tool(r, json!({"error": "agy_launcher_not_ready"})),
                    _ => Response::tool(r, json!({"agent_id": AGENT, "team": TEAM})),
                }
            }
            "get_inbox" => Response::tool(r, json!({"messages": []})),
            other => panic!("unexpected tool: {other}"),
        }
    })
    .await;
    let mut c = Client::start(
        server.named_bridge_with_retry(launch(), KEY.into(), 3, Duration::from_millis(10)),
        IDLE,
    );
    assert!(c.init().await["result"].is_object());
    let inbox = payload(c.call("get_inbox", json!({})).await);
    assert_eq!(inbox["messages"], json!([]));
    c.close().await;
    let log = server.log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|r| r.msg["params"]["name"] == "register_agent")
            .count(),
        3
    );
}

#[tokio::test]
async fn named_communication_success_preserves_short_token_words_in_mail_payload() {
    let server = Server::new(|r| {
        if let Some(reply) = handshake(r) {
            return reply;
        }
        if r.msg["method"] == "ping" {
            return Response::rpc(r, "result", json!({}));
        }
        match r.msg["params"]["name"].as_str().unwrap() {
            "register_agent" => {
                assert_registration(r);
                Response::tool(r, json!({"agent_id": AGENT, "team": TEAM}))
            }
            "get_inbox" => Response::tool(
                r,
                json!({
                    "messages": [{
                        "from_name": format!("{TOKEN}-agy"),
                        "from_team": TOKEN,
                        "body": format!("hello {TOKEN} secret-key={KEY}"),
                        "token": "explicit-secret-field"
                    }]
                }),
            ),
            other => panic!("unexpected tool: {other}"),
        }
    })
    .await;
    let mut c = Client::start(server.named_bridge(launch(), KEY.into()), IDLE);
    assert!(c.init().await["result"].is_object());
    let inbox = payload(c.call("get_inbox", json!({})).await);
    let msg = &inbox["messages"][0];
    assert_eq!(msg["from_name"], format!("{TOKEN}-agy"));
    assert_eq!(msg["from_team"], TOKEN);
    assert_eq!(msg["body"], format!("hello {TOKEN} secret-key=[redacted]"));
    assert_eq!(msg["token"], "[redacted]");
    c.close().await;
}
