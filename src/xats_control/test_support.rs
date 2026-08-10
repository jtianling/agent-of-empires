use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{discover_control_plane_in, ControlPlane};

pub(crate) struct FakeResponse {
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) delay: Duration,
}

pub(crate) struct FakeServer {
    pub(crate) port: u16,
    pub(crate) requests: Receiver<String>,
    pub(crate) worker: JoinHandle<()>,
}

pub(crate) fn spawn_fake_server(responses: Vec<FakeResponse>) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, requests) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            sender.send(request).unwrap();
            std::thread::sleep(response.delay);
            let reason = if response.status == 200 {
                "OK"
            } else {
                "Error"
            };
            let wire = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status,
                reason,
                response.body.len(),
                response.body
            );
            let _ = stream.write_all(wire.as_bytes());
        }
    });
    FakeServer {
        port,
        requests,
        worker,
    }
}

pub(crate) fn spawn_fake_server_without_content_length(body: String) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, requests) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        sender.send(read_request(&mut stream)).unwrap();
        let header =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body.as_bytes());
    });
    FakeServer {
        port,
        requests,
        worker,
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        let Some(header_end) = find_header_end(&request) else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() >= header_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8(request).unwrap()
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

pub(crate) fn write_pid_file(directory: &Path, port: u16, pid: u32) {
    std::fs::write(
        directory.join("daemon.pid"),
        format!(r#"{{"pid":{pid},"port":{port}}}"#),
    )
    .unwrap();
}

pub(crate) fn control_for(
    server: &FakeServer,
    token: Option<&str>,
) -> (tempfile::TempDir, ControlPlane) {
    let directory = tempfile::tempdir().unwrap();
    write_pid_file(directory.path(), server.port, std::process::id());
    let control = discover_control_plane_in(
        directory.path(),
        token.map(std::string::ToString::to_string),
    )
    .unwrap();
    (directory, control)
}
