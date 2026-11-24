//! Lightweight loopback mock xats daemon and executable fixtures for
//! Antigravity end-to-end tests.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[derive(Default)]
pub struct MockState {
    pub registered_key: Option<String>,
    pub agent_id: String,
    pub name: String,
    pub team: String,
    pub named_registrations: Vec<Value>,
    pub inbox_calls: usize,
    pub next_session: usize,
}

pub struct MockXatsDaemon {
    pub addr: SocketAddr,
    pub state: Arc<Mutex<MockState>>,
}

impl MockXatsDaemon {
    pub fn start(agent_id: &str, name: &str, team: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock xats daemon");
        let addr = listener.local_addr().expect("mock daemon addr");
        let state = Arc::new(Mutex::new(MockState {
            registered_key: None,
            agent_id: agent_id.to_string(),
            name: name.to_string(),
            team: team.to_string(),
            named_registrations: Vec::new(),
            inbox_calls: 0,
            next_session: 0,
        }));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = Arc::clone(&shared);
                thread::spawn(move || handle_connection(stream, s));
            }
        });
        Self { addr, state }
    }

    pub fn write_pid_file(&self, home: &Path) {
        let dir = home.join(".cross-agent-teams-mcp");
        std::fs::create_dir_all(&dir).expect("create mock xats dir");
        let body = json!({ "pid": std::process::id(), "port": self.addr.port() });
        std::fs::write(dir.join("daemon.pid"), body.to_string()).expect("write daemon.pid");
    }

    pub fn bind_identity(&self, key: &str) {
        self.state.lock().unwrap().registered_key = Some(key.to_string());
    }
}

fn handle_connection(mut stream: TcpStream, state: Arc<Mutex<MockState>>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let Some((method, path, body)) = read_http_request(&mut reader) else {
        return;
    };
    let (status, headers, payload) = route_request(&method, &path, &body, &state);
    let reply = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.flush();
}

fn read_http_request(reader: &mut BufReader<TcpStream>) -> Option<(String, String, Vec<u8>)> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut len = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; len];
    if len > 0 {
        reader.read_exact(&mut body).ok()?;
    }
    Some((method, path, body))
}

fn route_request(
    method: &str,
    path: &str,
    body: &[u8],
    state: &Arc<Mutex<MockState>>,
) -> (&'static str, String, String) {
    let mut st = state.lock().unwrap();
    match (method, path) {
        ("POST", "/api/identity-key/lookup") => {
            let req: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            let key = req["identity_key"].as_str().unwrap_or_default();
            let resp = if st.registered_key.as_deref() == Some(key) {
                json!({
                    "ok": true, "protocol_version": 1, "identity_key": key,
                    "holder": { "name": st.name, "team": st.team, "agent_type": "antigravity", "active": true }
                })
            } else {
                json!({ "ok": true, "protocol_version": 1, "identity_key": key, "holder": null })
            };
            (
                "200 OK",
                "Content-Type: application/json\r\n".into(),
                resp.to_string(),
            )
        }
        ("GET", p) if p.starts_with("/api/agents") => {
            let agents = if st.registered_key.is_some() {
                vec![json!({
                    "agent_id": st.agent_id, "name": st.name, "team": st.team,
                    "agent_type": "antigravity", "agent_type_name": null
                })]
            } else {
                Vec::new()
            };
            (
                "200 OK",
                "Content-Type: application/json\r\n".into(),
                json!({ "agents": agents }).to_string(),
            )
        }
        ("DELETE", "/mcp") => ("200 OK", String::new(), String::new()),
        ("POST", "/mcp") => route_mcp(body, &mut st),
        _ => ("404 Not Found", String::new(), String::new()),
    }
}

fn route_mcp(body: &[u8], st: &mut MockState) -> (&'static str, String, String) {
    let msg: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let method = msg["method"].as_str().unwrap_or_default();
    if method == "notifications/initialized" {
        return ("202 Accepted", String::new(), String::new());
    }
    let id = msg["id"].clone();
    match method {
        "initialize" => {
            st.next_session += 1;
            let sid = format!("mock-sid-{}", st.next_session);
            let reply = json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "protocolVersion": "2025-06-18", "capabilities": { "tools": {} } }
            });
            (
                "200 OK",
                format!("Content-Type: application/json\r\nMcp-Session-Id: {sid}\r\n"),
                reply.to_string(),
            )
        }
        "ping" => (
            "200 OK",
            "Content-Type: application/json\r\n".into(),
            json!({ "jsonrpc": "2.0", "id": id, "result": {} }).to_string(),
        ),
        "tools/list" => (
            "200 OK",
            "Content-Type: application/json\r\n".into(),
            json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": daemon_tools() } })
                .to_string(),
        ),
        "tools/call" => route_tool_call(&id, &msg["params"], st),
        _ => ("400 Bad Request", String::new(), String::new()),
    }
}

fn route_tool_call(
    id: &Value,
    params: &Value,
    st: &mut MockState,
) -> (&'static str, String, String) {
    let tool = params["name"].as_str().unwrap_or_default();
    let args = &params["arguments"];
    let payload = match tool {
        "register_agent" => {
            st.named_registrations.push(args.clone());
            json!({ "agent_id": st.agent_id, "team": st.team, "name": st.name })
        }
        "get_inbox" => {
            st.inbox_calls += 1;
            json!({ "messages": [], "has_more": false, "last_event_id": 1 })
        }
        _ => json!({ "ok": true }),
    };
    let reply = json!({
        "jsonrpc": "2.0", "id": id,
        "result": { "content": [{ "type": "text", "text": payload.to_string() }] }
    });
    (
        "200 OK",
        "Content-Type: application/json\r\n".into(),
        reply.to_string(),
    )
}

fn daemon_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "register_agent",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_type": { "type": "string", "enum": ["custom", "antigravity"] },
                    "agy_launch_id": { "type": "string" }
                }
            }
        }),
        json!({ "name": "get_inbox", "description": "upstream-inbox", "inputSchema": { "type": "object" } }),
        json!({ "name": "send_message", "description": "upstream-send", "inputSchema": { "type": "object" } }),
        json!({ "name": "broadcast_to_role", "description": "upstream-role", "inputSchema": { "type": "object" } }),
        json!({ "name": "get_delivery_status", "description": "upstream-status", "inputSchema": { "type": "object" } }),
        json!({ "name": "cancel_unread_alert", "description": "upstream-cancel", "inputSchema": { "type": "object" } }),
    ]
}

/// Compile a tiny native `agy` executable so `ps` reports its process name as
/// `agy` (matching AoE's process-ancestry check), and delegate runtime work to
/// a sibling Python script.
pub fn install_native_agy(stub_dir: &Path, driver: &Path) -> Option<PathBuf> {
    let src = stub_dir.join("agy_driver.c");
    let code = format!(
        r#"#include <unistd.h>
#include <sys/wait.h>
#include <stdlib.h>
#include <stdio.h>
int main(int argc, char **argv) {{
    pid_t p = fork();
    if (p == 0) {{
        char **args = calloc((size_t)(argc + 3), sizeof(char *));
        args[0] = "python3";
        args[1] = "{}";
        for (int i = 1; i < argc; i++) args[i + 1] = argv[i];
        execvp("python3", args);
        _exit(127);
    }}
    int status = 0;
    waitpid(p, &status, 0);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 1;
}}
"#,
        driver.display()
    );
    std::fs::write(&src, code).ok()?;
    let bin = stub_dir.join("agy");
    let out = std::process::Command::new("cc")
        .arg("-o")
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    out.status.success().then_some(bin)
}

pub fn sqlite_query(db: &Path, sql: &str) -> String {
    let out = std::process::Command::new("sqlite3")
        .arg(db)
        .arg(sql)
        .output()
        .expect("run sqlite3");
    assert!(
        out.status.success(),
        "sqlite3 failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn wait_for_jsonl_count(path: &Path, expected: usize, label: &str) -> Vec<Value> {
    let start = Instant::now();
    loop {
        let rows: Vec<Value> = std::fs::read_to_string(path)
            .ok()
            .into_iter()
            .flat_map(|s| {
                s.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| serde_json::from_str(l).expect("valid jsonl line"))
                    .collect::<Vec<_>>()
            })
            .collect();
        if rows.len() >= expected {
            return rows;
        }
        if start.elapsed() > Duration::from_secs(15) {
            panic!("Timed out waiting for {expected} {label} entries; got {rows:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn assert_pid_exited(pid: u64) {
    let start = Instant::now();
    loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|s| s.success());
        if !alive {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("Previous xats-agy pid {pid} survived respawn");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
