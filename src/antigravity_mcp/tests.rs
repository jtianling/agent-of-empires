mod http;
mod named_pty;

use std::collections::HashMap;
use std::sync::Mutex;

use http::{Request, Response, Server};
use tokio::io::{duplex, split, AsyncBufReadExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::task::JoinHandle;

use super::*;

const TOKEN: &str = "fixture-bearer";
const VERSION: &str = "2025-06-18";

struct Client {
    input: WriteHalf<DuplexStream>,
    output: BufReader<ReadHalf<DuplexStream>>,
    worker: JoinHandle<Result<()>>,
}

impl Client {
    fn start(mut bridge: Bridge, idle: Duration) -> Self {
        let (peer, pipe) = duplex(65536);
        let (rx, tx) = split(pipe);
        let worker = tokio::spawn(async move {
            let result = bridge.serve(rx, tx, idle).await;
            bridge.finish(result).await
        });
        let (rx, input) = split(peer);
        Self {
            input,
            output: BufReader::new(rx),
            worker,
        }
    }

    async fn write(&mut self, value: Value) {
        self.input
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.write(json!({"jsonrpc":"2.0","id":"agy","method":method,"params":params}))
            .await;
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(4), self.output.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], "agy");
        reply
    }

    async fn init(&mut self) -> Value {
        let reply = self
            .rpc(
                "initialize",
                json!({"protocolVersion":VERSION,
            "clientInfo":{"name":"agy","version":"0"},"capabilities":{}}),
            )
            .await;
        if reply.get("result").is_some() {
            self.write(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                .await;
        }
        reply
    }

    async fn call(&mut self, name: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({"name":name,"arguments":args}))
            .await
    }

    async fn close(mut self) {
        self.input.shutdown().await.unwrap();
        self.worker.await.unwrap().unwrap();
    }
}

fn payload(reply: Value) -> Value {
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn disabled_advertises_no_tools_and_rejects_calls() {
    assert!(validate_key("bad").is_err());
    assert!(validate_key(&Uuid::nil().to_string()).is_err());
    assert!(validate_key(&Uuid::new_v4().to_string()).is_ok());
    let mut c = Client::start(Bridge::new(None), IDLE);
    assert_eq!(c.init().await["result"]["protocolVersion"], VERSION);
    assert_eq!(
        c.rpc("tools/list", json!({})).await["result"]["tools"],
        json!([])
    );
    assert!(c.call("reconnect", json!({})).await["error"].is_object());
    c.close().await;
}

#[tokio::test]
async fn concurrent_clients_bind_distinct_sessions_and_restart_with_the_same_key() {
    let identities = Mutex::new(HashMap::<String, String>::new());
    let sessions = Mutex::new(HashMap::<String, String>::new());
    let server = Server::new(move |req| {
        assert_eq!(req.headers["authorization"], format!("Bearer {TOKEN}"));
        if req.msg["method"] != "initialize" {
            assert_eq!(req.headers["mcp-protocol-version"], VERSION);
        }
        if let Some(response) = Response::handshake(req) {
            return response;
        }
        let args = &req.msg["params"]["arguments"];
        let mut bindings = sessions.lock().unwrap();
        let mut names = identities.lock().unwrap();
        let body = match req.msg["params"]["name"].as_str().unwrap() {
            "reconnect" => {
                let key = args["identity_key"].as_str().unwrap();
                assert_eq!(*args, json!({"identity_key":key}));
                bindings.insert(req.sid().into(), key.into());
                match names.get(key) {
                    None => json!({"need_register":true}),
                    Some(name) => json!({"ok":true,"name":name,"team":"dev","agent_id":name}),
                }
            }
            "register_agent" => {
                let key = &bindings[req.sid()];
                let name = args["name"].as_str().unwrap();
                assert_eq!(
                    args,
                    &json!({"agent_type":"custom","agent_type_name":"Antigravity",
                    "team":"dev","name":name,"identity_key":key})
                );
                names.insert(key.clone(), name.into());
                json!({"agent_id":name,"team":"dev","identity_key_bound":true,"echo":key,"token":TOKEN})
            }
            "get_inbox" => json!({"mailbox":names[&bindings[req.sid()]],
                "messages":[{"from_agent_id":"sender-id"}]}),
            "send_message" | "send_message_by_id" => json!({"accepted":args}),
            _ => panic!("unexpected tool"),
        };
        Response::tool(req, body)
    })
    .await;
    let (ka, kb) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
    let mut a = Client::start(server.bridge(&ka, Some(TOKEN)), IDLE);
    let mut b = Client::start(server.bridge(&kb, Some(TOKEN)), IDLE);
    let (ia, ib) = tokio::join!(a.init(), b.init());
    assert!(ia["result"].is_object() && ib["result"].is_object());
    let (ra, rb) = tokio::join!(
        a.call("register_agent", json!({"name":"a","team":"dev"})),
        b.call("register_agent", json!({"name":"b","team":"dev"}))
    );
    for (reply, key) in [(ra, &ka), (rb, &kb)] {
        assert!(!reply.to_string().contains(key) && !reply.to_string().contains(TOKEN));
        let registered = payload(reply);
        assert_eq!(registered["identity_key_bound"], true);
        assert!(registered["hint"].as_str().unwrap().contains("小写 r"));
    }
    let inbox = payload(b.call("get_inbox", json!({})).await);
    assert_eq!(inbox["mailbox"], "b");
    let reply = json!({"to_agent_id":inbox["messages"][0]["from_agent_id"],"body":"received"});
    assert_eq!(
        payload(b.call("send_message_by_id", reply.clone()).await)["accepted"],
        reply
    );
    let args = json!({"to_agent_name":"b","body":"hello"});
    assert_eq!(
        payload(a.call("send_message", args.clone()).await)["accepted"],
        args
    );
    a.close().await;
    let mut replacement = Client::start(server.bridge(&ka, Some(TOKEN)), IDLE);
    assert!(replacement.init().await["result"].is_object());
    assert_eq!(
        payload(replacement.call("get_inbox", json!({})).await)["mailbox"],
        "a"
    );
    tokio::join!(replacement.close(), b.close());
    let log = server.log.lock().unwrap();
    let closed = log
        .iter()
        .filter(|r| r.verb == "DELETE")
        .map(Request::sid)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(closed.len(), 3);
    assert_eq!(
        log.iter()
            .filter(|r| r.msg["params"]["name"] == "register_agent")
            .count(),
        2
    );
}

#[tokio::test]
async fn conflicts_and_rpc_errors_do_not_disclose_secrets_or_retry() {
    let key = Uuid::new_v4().to_string();
    let echo = format!("conflict: {key} / {TOKEN}");
    let server = Server::new(move |req| {
        if let Some(reply) = Response::handshake(req) {
            return reply;
        }
        match req.msg["params"]["name"].as_str().unwrap() {
            "reconnect" => Response::tool(req, json!({"need_register":true})),
            "register_agent" => Response::tool(
                req,
                json!({"error":"identity_key_conflict",
                "identity_key_bound":false,"detail":echo}),
            ),
            "get_inbox" => Response::rpc(req, "error", json!({"code":-32042,"message":echo})),
            "send_message" => Response::status(503),
            _ => panic!("unexpected tool"),
        }
    })
    .await;
    let mut c = Client::start(server.bridge(&key, Some(TOKEN)), IDLE);
    assert!(c.init().await["result"].is_object());
    let conflict = payload(
        c.call("register_agent", json!({"name":"requested","team":"dev"}))
            .await,
    );
    assert_eq!(conflict["error"], "identity_key_conflict");
    assert_eq!(conflict["identity_key_bound"], false);
    let error = c.call("get_inbox", json!({})).await;
    assert_eq!(error["error"]["code"], -32042);
    for value in [conflict, error] {
        let wire = value.to_string();
        assert!(!wire.contains(&key) && !wire.contains(TOKEN));
    }
    let send = c
        .call("send_message", json!({"to_agent_name":"test","body":"hi"}))
        .await;
    assert!(send["error"]["message"].as_str().unwrap().contains("503"));
    c.close().await;
    assert_eq!(
        server
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.msg["params"]["name"] == "send_message")
            .count(),
        1
    );
}

#[tokio::test]
async fn unauthorized_or_redirected_initialize_fails_closed() {
    for status in [401, 307] {
        let server = Server::new(move |req| {
            assert!(!req.headers.contains_key("authorization"));
            let mut response = Response::status(status);
            response.headers = "Location: /other\r\n".into();
            response
        })
        .await;
        let mut c = Client::start(server.bridge(&Uuid::new_v4().to_string(), None), IDLE);
        let reply = c.init().await;
        assert!(reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&status.to_string()));
        assert!(c
            .call("register_agent", json!({"name":"x","team":"dev"}))
            .await
            .get("error")
            .is_some());
        c.close().await;
        assert_eq!(server.log.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn idle_closes_upstream_and_wake_reconnects_before_call() {
    let (tx, mut deletes) = tokio::sync::mpsc::unbounded_channel();
    let server = Server::new(move |req| {
        assert!(!req.headers.contains_key("authorization"));
        if req.verb == "DELETE" {
            tx.send(req.sid().to_owned()).unwrap();
        }
        Response::handshake(req)
            .unwrap_or_else(|| Response::tool(req, json!({"need_register":true})))
    })
    .await;
    let mut c = Client::start(
        server.bridge(&Uuid::new_v4().to_string(), None),
        Duration::from_millis(40),
    );
    assert!(c.init().await.get("result").is_some());
    let old = tokio::time::timeout(Duration::from_secs(3), deletes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        payload(c.call("reconnect", json!({})).await)["need_register"],
        true
    );
    c.close().await;
    assert_ne!(deletes.recv().await.unwrap(), old);
    assert_eq!(
        server
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.msg["params"]["name"] == "reconnect")
            .count(),
        2
    );
}

#[test]
fn tool_schema_and_runtime_reject_untrusted_bindings() {
    for tool in tools::list() {
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert!(tool["inputSchema"]["properties"]
            .get("identity_key")
            .is_none());
    }
    for tool in [
        "reconnect",
        "register_agent",
        "get_inbox",
        "list_agents",
        "send_message_by_id",
        "bind_runtime_identity",
    ] {
        for field in [
            "identity_key",
            "ui_pid",
            "thread_id",
            "agent_type",
            "delivery",
            "from_agent_id",
        ] {
            assert!(tools::prepare(
                &json!({"name":tool,"arguments":{field:"invalid"}}),
                "fixture"
            )
            .is_err());
        }
    }
    assert!(tools::prepare(
        &json!({"name":"register_agent","arguments":{"name":"a","team":"  "}}),
        "x"
    )
    .is_err());
    assert_eq!(
        tools::prepare(
            &json!({"name":"broadcast","arguments":{"body":"hi","auto_poke":false}}),
            "x"
        )
        .unwrap()["arguments"]["auto_poke"],
        false
    );
}

#[tokio::test]
async fn oversized_stdio_line_stops_without_forwarding() {
    let mut bridge = Bridge::new(None);
    let bytes = vec![b'x'; LIMIT + 1];
    let err = bridge
        .serve(bytes.as_slice(), tokio::io::sink(), IDLE)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("max line length"));
}

#[test]
fn pane_lease_is_exclusive_across_processes_and_reusable_after_exit() {
    const CHILD: &str = "AOE_TEST_ANTIGRAVITY_LEASE_HOLDER";
    const KEY: &str = "fixture-pane-one";
    if let Some(dir) = std::env::var_os(CHILD) {
        let _holder = lease::acquire(std::path::Path::new(&dir), KEY).unwrap();
        println!("LOCK-READY");
        let mut byte = [0; 1];
        std::io::Read::read_exact(&mut std::io::stdin(), &mut byte).unwrap();
        return;
    }
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact","antigravity_mcp::tests::pane_lease_is_exclusive_across_processes_and_reusable_after_exit","--nocapture"])
        .env(CHILD,dir.path()).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.contains("LOCK-READY") {
        line.clear();
        assert_ne!(stdout.read_line(&mut line).unwrap(), 0);
    }
    let busy = lease::acquire_within(dir.path(), KEY, Duration::from_millis(80))
        .unwrap_err()
        .to_string();
    assert!(busy.contains("already active") && !busy.contains(KEY));
    let _other = lease::acquire(dir.path(), "fixture-pane-two").unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        stdin.write_all(b"x").unwrap();
    });
    let _next = lease::acquire_within(dir.path(), KEY, Duration::from_secs(2)).unwrap();
    releaser.join().unwrap();
    assert!(child.wait().unwrap().success());
}

#[tokio::test]
async fn eof_drains_the_pending_send_once_then_deletes_the_session() {
    let server = Server::new(|req| {
        if let Some(reply) = Response::handshake(req) {
            return reply;
        }
        match req.msg["params"]["name"].as_str().unwrap() {
            "reconnect" => Response::tool(req, json!({"need_register":true})),
            "send_message" => Response::tool(req, json!({"message_id":"receipt"})),
            _ => panic!("unexpected call"),
        }
    })
    .await;
    let mut c = Client::start(server.bridge(&Uuid::new_v4().to_string(), None), IDLE);
    assert!(c.init().await.get("result").is_some());
    c.write(
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{
        "name":"send_message","arguments":{"to_agent_name":"peer","body":"test"}}}),
    )
    .await;
    c.input.shutdown().await.unwrap();
    let mut line = String::new();
    assert_ne!(c.output.read_line(&mut line).await.unwrap(), 0);
    let response = serde_json::from_str(&line).unwrap();
    assert_eq!(payload(response)["message_id"], "receipt");
    c.worker.await.unwrap().unwrap();
    let log = server.log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|r| r.msg["params"]["name"] == "send_message")
            .count(),
        1
    );
    assert_eq!(log.last().unwrap().verb, "DELETE");
}

#[tokio::test]
async fn expired_session_requires_explicit_reconnect_and_never_replays_send() {
    let server = Server::new(|r| {
        Response::handshake(r).unwrap_or_else(|| match r.msg["params"]["name"].as_str() {
            Some("reconnect") => Response::tool(r, json!({"need_register":true})),
            Some("send_message") => Response::status(404),
            _ => panic!("unexpected call"),
        })
    })
    .await;
    let mut c = Client::start(server.bridge(&Uuid::new_v4().to_string(), None), IDLE);
    c.init().await;
    let refused = c
        .call(
            "send_message",
            json!({"to_agent_name":"peer","body":"test"}),
        )
        .await;
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("404"));
    let refused = c.call("list_agents", json!({})).await;
    assert!(refused.get("error").is_some());
    assert_eq!(
        payload(c.call("reconnect", json!({})).await)["need_register"],
        true
    );
    c.close().await;
    let log = server.log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|r| r.msg["params"]["name"] == "send_message")
            .count(),
        1
    );
    assert_eq!(
        log.iter()
            .filter(|r| r.msg["method"] == "initialize")
            .count(),
        2
    );
}

#[tokio::test]
async fn confirmed_binding_rejects_name_and_team_changes_before_http() {
    for source in ["initialize", "register_agent", "reconnect"] {
        let recoveries = std::sync::atomic::AtomicUsize::new(0);
        let server = Server::new(move |r| {
            if let Some(reply) = Response::handshake(r) {
                return reply;
            }
            let body = match r.msg["params"]["name"].as_str().unwrap() {
                "register_agent" => json!({"agent_id":"fixture","team":"dev"}),
                "reconnect" => {
                    let count = recoveries.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if source == "initialize" || (source == "reconnect" && count > 0) {
                        json!({"ok":true,"agent_id":"fixture","name":"alice","team":"dev"})
                    } else {
                        json!({"need_register":true})
                    }
                }
                _ => panic!("unexpected request"),
            };
            Response::tool(r, body)
        })
        .await;
        let mut c = Client::start(server.bridge(&Uuid::new_v4().to_string(), None), IDLE);
        assert!(c.init().await.get("result").is_some());
        let same = json!({"name":"alice","team":"dev"});
        match source {
            "register_agent" => assert!(c.call(source, same.clone()).await.get("result").is_some()),
            "reconnect" => assert_eq!(payload(c.call(source, json!({})).await)["ok"], true),
            _ => {}
        }
        let before = server.log.lock().unwrap().len();
        for changed in [
            json!({"name":"bob","team":"dev"}),
            json!({"name":"alice","team":"ops"}),
        ] {
            assert_eq!(
                c.call("register_agent", changed).await["error"]["code"],
                -32602
            );
        }
        assert_eq!(server.log.lock().unwrap().len(), before);
        assert_eq!(
            payload(c.call("register_agent", same).await)["agent_id"],
            "fixture"
        );
        c.close().await;
    }
}
