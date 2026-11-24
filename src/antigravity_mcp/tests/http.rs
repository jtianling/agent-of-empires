use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use super::*;
use crate::xats_control::{discover_control_plane_in, test_support::write_pid_file};

pub(super) struct Request {
    pub verb: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub msg: Value,
}

impl Request {
    pub fn sid(&self) -> &str {
        &self.headers["mcp-session-id"]
    }
}

pub(super) struct Response {
    pub status: u16,
    pub headers: String,
    pub body: String,
}

impl Response {
    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: String::new(),
            body: String::new(),
        }
    }

    pub fn rpc(req: &Request, field: &str, value: Value) -> Self {
        let body = json!({"id":req.msg["id"],"jsonrpc":"2.0",field:value});
        Self {
            status: 200,
            headers: "Content-Type: application/json\r\n".into(),
            body: body.to_string(),
        }
    }

    pub fn tool(req: &Request, body: Value) -> Self {
        Self::rpc(
            req,
            "result",
            json!({"content":[{"type":"text","text":body.to_string()}]}),
        )
    }

    pub fn handshake(req: &Request) -> Option<Self> {
        match (req.verb.as_str(), req.msg["method"].as_str()) {
            ("DELETE", _) => Some(Self::status(204)),
            (_, Some("initialize")) => {
                assert!(!req.headers.contains_key("mcp-session-id"));
                let mut res = Self::rpc(
                    req,
                    "result",
                    json!({"protocolVersion": "2025-06-18",
                    "serverInfo":{"name":"fixture","version":"0"}, "capabilities":{"tools":{}}}),
                );
                res.headers = format!(
                    "Content-Type: text/event-stream\r\nMcp-Session-Id: {}\r\n",
                    Uuid::new_v4()
                );
                res.body = format!(": hello\r\n\r\nevent: message\r\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/test\"}}\r\n\r\nevent: message\r\ndata: {}\r\n\r\n",res.body);
                Some(res)
            }
            (_, Some("notifications/initialized")) => Some(Self::status(202)),
            _ => None,
        }
    }
}

async fn read(socket: &mut TcpStream) -> Request {
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let mut start = line.split_whitespace();
    let verb = start.next().unwrap().to_owned();
    let path = start.next().unwrap().to_owned();
    assert!(
        path == "/mcp"
            || path == "/api/identity-key/lookup"
            || path.starts_with("/api/agents?team=")
    );
    let mut headers = HashMap::new();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).await.unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').unwrap();
        headers.insert(name.to_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .map_or(0, |s| s.parse().unwrap());
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await.unwrap();
    let msg = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    Request {
        verb,
        path,
        headers,
        msg,
    }
}

pub(super) struct Server {
    dir: tempfile::TempDir,
    worker: JoinHandle<()>,
    pub log: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    pub async fn new(handler: impl Fn(&Request) -> Response + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_pid_file(
            dir.path(),
            listener.local_addr().unwrap().port(),
            std::process::id(),
        );
        let log = Arc::new(Mutex::new(Vec::new()));
        let records = Arc::clone(&log);
        let worker = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let req = read(&mut socket).await;
                let reply = handler(&req);
                records.lock().unwrap().push(req);
                let wire = format!(
                    "HTTP/1.1 {} Mock\r\n{}Connection: close\r\nContent-Length: {}\r\n\r\n{}",
                    reply.status,
                    reply.headers,
                    reply.body.len(),
                    reply.body
                );
                socket.write_all(wire.as_bytes()).await.unwrap();
            }
        });
        Self { dir, worker, log }
    }

    pub(super) fn named_bridge(&self, launch: named::Launch, key: String) -> Bridge {
        self.named_bridge_with_retry(launch, key, 0, Duration::ZERO)
    }

    pub(super) fn named_bridge_with_retry(
        &self,
        launch: named::Launch,
        key: String,
        retries: usize,
        delay: Duration,
    ) -> Bridge {
        let dir = self.dir.path().to_owned();
        let mode = named::Named::with_retry(
            launch,
            move || {
                let control = discover_control_plane_in(&dir, Some(TOKEN.to_owned()))?;
                Upstream::new(control, key.clone())
            },
            retries,
            delay,
        );
        Bridge::with_mode(Mode::Named(mode))
    }

    pub fn bridge(&self, key: &str, token: Option<&str>) -> Bridge {
        let control = discover_control_plane_in(self.dir.path(), token.map(str::to_owned)).unwrap();
        Bridge::new(Some(Upstream::new(control, key.to_owned()).unwrap()))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
