use super::*;

use std::process::Command as ProcessCommand;

use tokio::io::BufReader;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn streamable_http_initializes_and_lists_tools() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let url = format!("http://{addr}/mcp");

    let server = tokio::spawn(async move {
        for step in 0..3 {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read");
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let request_lc = request.to_ascii_lowercase();

            if step == 0 {
                assert!(request_lc.contains("\r\naccept:"), "request={request}");
                assert!(request_lc.contains("application/json"), "request={request}");
                assert!(
                    request_lc.contains("text/event-stream"),
                    "request={request}"
                );
                assert!(request.contains("\"method\":\"initialize\""));
                let body = serde_json::json!({
                    "jsonrpc":"2.0",
                    "id": 1,
                    "result": {
                        "protocolVersion": DEFAULT_PROTOCOL_VERSION,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "stub", "version": "0"}
                    }
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: sess-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            } else if step == 1 {
                // notifications/initialized
                assert!(
                    request_lc.contains("mcp-session-id: sess-1"),
                    "request={request}"
                );
                let response =
                    "HTTP/1.1 202 Accepted\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
                socket.write_all(response.as_bytes()).await.expect("write");
            } else {
                assert!(
                    request_lc.contains("mcp-session-id: sess-1"),
                    "request={request}"
                );
                assert!(
                    request_lc.contains("mcp-protocol-version: 2025-11-25"),
                    "request={request}"
                );
                assert!(request.contains("\"method\":\"tools/list\""));
                let body = serde_json::json!({
                    "jsonrpc":"2.0",
                    "id": 2,
                    "result": {
                        "tools": [
                            {
                                "name": "hello",
                                "description": "hi",
                                "inputSchema": {"type":"object","additionalProperties":false}
                            }
                        ]
                    }
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            }
        }
    });

    let session = McpHttpSession::connect(&url, None).await.expect("connect");
    assert_eq!(session.session_id(), Some("sess-1"));
    let tools = session.list_tools().await.expect("list_tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "hello");

    server.await.expect("server");
}

#[tokio::test]
async fn framed_stdio_parser_reads_jsonrpc_payload() {
    let (mut writer, reader) = tokio::io::duplex(1024);
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "result": {"ok": true}
    })
    .to_string();
    let framed = format!(
        "Content-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
        payload.len(),
        payload
    );

    tokio::spawn(async move {
        writer
            .write_all(framed.as_bytes())
            .await
            .expect("write frame");
    });

    let mut reader = BufReader::new(reader);
    let value = read_jsonrpc_frame(&mut reader).await.expect("parse frame");
    assert_eq!(value.get("jsonrpc").and_then(Value::as_str), Some("2.0"));
    assert_eq!(value.get("id").and_then(Value::as_u64), Some(7));
}

#[test]
fn matching_id_accepts_number_and_string() {
    assert!(is_matching_id(Some(&serde_json::json!(3)), 3));
    assert!(is_matching_id(Some(&serde_json::json!("3")), 3));
    assert!(!is_matching_id(Some(&serde_json::json!("abc")), 3));
}

#[tokio::test]
async fn stdio_session_round_trip_for_tools_and_resources() {
    if ProcessCommand::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping stdio MCP round-trip test: python3 not available");
        return;
    }

    let script = r#"
import json
import os
import sys

def read_message():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        name, value = line.decode("utf-8").split(":", 1)
        headers[name.strip().lower()] = value.strip()
    length = int(headers.get("content-length", "0"))
    payload = sys.stdin.buffer.read(length)
    return json.loads(payload.decode("utf-8"))

def send_message(payload):
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("utf-8"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

if os.getenv("MCP_AUTH_TOKEN") != "token-123":
    sys.exit(1)

message = read_message()
if message is None or message.get("method") != "initialize":
    sys.exit(1)
send_message({
    "jsonrpc": "2.0",
    "id": message["id"],
    "result": {
        "protocolVersion": "2025-11-25",
        "capabilities": {"tools": {}, "resources": {}},
        "serverInfo": {"name": "stub", "version": "0"}
    }
})

message = read_message()
if message is None or message.get("method") != "notifications/initialized":
    sys.exit(1)

message = read_message()
if message is None or message.get("method") != "tools/list":
    sys.exit(1)
send_message({
    "jsonrpc": "2.0",
    "id": message["id"],
    "result": {
        "tools": [
            {
                "name": "hello",
                "description": "hi",
                "inputSchema": {"type": "object"}
            }
        ]
    }
})

message = read_message()
if message is None or message.get("method") != "resources/read":
    sys.exit(1)
uri = message.get("params", {}).get("uri")
send_message({
    "jsonrpc": "2.0",
    "id": message["id"],
    "result": {
        "contents": [
            {
                "uri": uri,
                "mimeType": "text/plain",
                "text": "from-stdio"
            }
        ]
    }
})

message = read_message()
if message is None or message.get("method") != "tools/call":
    sys.exit(1)
send_message({
    "jsonrpc": "2.0",
    "id": message["id"],
    "result": {
        "isError": False,
        "content": [{"type": "text", "text": "ok"}]
    }
})
"#;

    let args = vec!["-u".to_string(), "-c".to_string(), script.to_string()];
    let session = McpStdioSession::connect(
        "python3",
        &args,
        &BTreeMap::new(),
        Some("token-123".to_string()),
    )
    .await
    .expect("connect stdio session");

    let tools = session.list_tools().await.expect("list tools over stdio");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "hello");

    let resource = session
        .read_resource("file:///tmp/demo")
        .await
        .expect("read resource over stdio");
    assert!(
        resource.to_string().contains("from-stdio"),
        "resource={resource}"
    );

    let result = session
        .call_tool("hello", serde_json::json!({"name": "world"}))
        .await
        .expect("call tool over stdio");
    assert!(result.to_string().contains("\"isError\":false"));
}
