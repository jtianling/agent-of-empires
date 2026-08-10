use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

pub(crate) struct FakeReply {
    pub(crate) status: u16,
    pub(crate) body: String,
}

impl FakeReply {
    pub(crate) fn ok(body: &str) -> Self {
        Self {
            status: 200,
            body: body.to_string(),
        }
    }

    pub(crate) fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }
}

pub(crate) struct FakeKimi {
    pub(crate) base_url: String,
    pub(crate) requests: Receiver<String>,
    pub(crate) worker: JoinHandle<()>,
}

pub(crate) fn spawn_fake_kimi(replies: Vec<FakeReply>) -> FakeKimi {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, requests) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        for reply in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            sender.send(read_request(&mut stream)).unwrap();
            let wire = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.status,
                if reply.status == 200 { "OK" } else { "Error" },
                reply.body.len(),
                reply.body
            );
            let _ = stream.write_all(wire.as_bytes());
        }
    });
    FakeKimi {
        base_url: format!("http://127.0.0.1:{port}"),
        requests,
        worker,
    }
}

pub(crate) fn spawn_fake_kimi_without_content_length(body_len: usize) -> FakeKimi {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, requests) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        sender.send(read_request(&mut stream)).unwrap();
        let header =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all("x".repeat(body_len).as_bytes());
    });
    FakeKimi {
        base_url: format!("http://127.0.0.1:{port}"),
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
        let Ok(count) = stream.read(&mut buffer) else {
            break;
        };
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
    String::from_utf8_lossy(&request).into_owned()
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}
