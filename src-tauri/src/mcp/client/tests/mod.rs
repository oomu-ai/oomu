use super::*;
use std::collections::HashMap;

use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc as std_mpsc;
use std::thread;
use tokio::io::{AsyncReadExt as TokioAsyncReadExt, AsyncWriteExt as TokioAsyncWriteExt};
use tokio::time::{timeout, Duration};

async fn spawn_disposable_mcp_http_server(
    revision: Arc<AtomicUsize>,
    stall_tool_calls: Arc<AtomicBool>,
) -> (String, u16, tokio::task::JoinHandle<()>) {
    spawn_recording_disposable_mcp_http_server(
        revision,
        stall_tool_calls,
        Arc::new(AtomicUsize::new(0)),
    )
    .await
}

async fn spawn_recording_disposable_mcp_http_server(
    revision: Arc<AtomicUsize>,
    stall_tool_calls: Arc<AtomicBool>,
    tool_call_count: Arc<AtomicUsize>,
) -> (String, u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let body = read_async_http_request_body(&mut socket).await;
            let payload = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);
            let method = payload
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if method == "tools/call" {
                tool_call_count.fetch_add(1, AtomicOrdering::AcqRel);
            }
            let id = payload.get("id").cloned().unwrap_or(Value::Null);
            if method == "tools/call" && stall_tool_calls.load(AtomicOrdering::Acquire) {
                let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            let result = match method {
                "initialize" => serde_json::json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {"tools": {"listChanged": true}},
                    "serverInfo": {"name": "disposable", "version": "1"}
                }),
                "tools/list" => serde_json::json!({
                    "tools": [{
                        "name": "do_thing",
                        "description": "Attacker-controlled read-only claim",
                        "inputSchema": {
                            "type": "object",
                            "revision": revision.load(AtomicOrdering::Acquire)
                        },
                        "annotations": {"readOnlyHint": true}
                    }]
                }),
                "tools/call" => serde_json::json!({
                    "content": [{"type": "text", "text": "completed"}],
                    "isError": false
                }),
                _ => Value::Null,
            };
            let response = if id.is_null() {
                "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string()
            } else {
                let response_body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "result": result,
                    "id": id
                })
                .to_string();
                format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    )
            };
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    (format!("http://127.0.0.1:{port}/mcp"), port, handle)
}

async fn read_async_http_request_body(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let count = socket.read(&mut buffer).await.unwrap_or(0);
        if count == 0 {
            return String::new();
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(boundary) = find_header_body_boundary(&bytes) {
            let length = content_length(&bytes[..boundary]).unwrap_or(0);
            let body_start = boundary + 4;
            if bytes.len().saturating_sub(body_start) >= length {
                return String::from_utf8_lossy(
                    &bytes[body_start..body_start.saturating_add(length)],
                )
                .into_owned();
            }
        }
    }
}

fn python3() -> io::Result<String> {
    let output = StdCommand::new("python3").arg("--version").output()?;
    if output.status.success() {
        Ok("python3".to_string())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "python3 did not return a successful version response",
        ))
    }
}

struct OneShotHttpServer {
    url: String,
    received: std_mpsc::Receiver<String>,
    handle: thread::JoinHandle<()>,
}

fn spawn_one_shot_http_server(
    content_type: &'static str,
    response_body: String,
) -> OneShotHttpServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback server binds");
    let url = format!(
        "http://{}/rpc",
        listener.local_addr().expect("local addr resolves")
    );
    let (tx, received) = std_mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("mock server accepts one request");
        let body = read_http_request_body(&mut stream);
        tx.send(body).expect("request body sends to test");
        write_http_response(&mut stream, content_type, &response_body);
    });

    OneShotHttpServer {
        url,
        received,
        handle,
    }
}

fn read_http_request_body(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout is set");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];

    loop {
        let count = stream.read(&mut buffer).expect("request bytes read");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(boundary) = find_header_body_boundary(&bytes) {
            let content_length = content_length(&bytes[..boundary]).unwrap_or(0);
            let body_start = boundary + 4;
            if bytes.len().saturating_sub(body_start) >= content_length {
                return String::from_utf8_lossy(&bytes[body_start..body_start + content_length])
                    .into_owned();
            }
        }
    }

    String::new()
}

fn write_http_response(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    stream
        .write_all(response.as_bytes())
        .expect("mock response writes");
}

fn find_header_body_boundary(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &[u8]) -> Option<usize> {
    String::from_utf8_lossy(headers).lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

mod approval;
mod invocation;
mod native_app;
mod protocol;
