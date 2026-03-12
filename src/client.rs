use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::env;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::time::{Duration, sleep, timeout};

const VALID_EVENTS: &[&str] = &[
    "command_done", "idle", "activity", "exit", "bell", "title",
];

fn socket_path() -> String {
    env::var("T4A_SOCKET").unwrap_or_else(|_| "/tmp/t4a.sock".into())
}

pub async fn ensure_daemon() -> Result<()> {
    if request(&serde_json::json!({"cmd": "list"})).await.is_ok() {
        return Ok(());
    }
    if let Ok(mut child) = Command::new("t4a")
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        tokio::spawn(async move { let _ = child.wait().await; });
    }
    for _ in 0..25 {
        sleep(Duration::from_millis(100)).await;
        if request(&serde_json::json!({"cmd": "list"})).await.is_ok() {
            return Ok(());
        }
    }
    bail!("failed to start t4a daemon")
}

pub async fn request(req: &Value) -> Result<Value> {
    let mut stream = UnixStream::connect(socket_path())
        .await
        .context("connect to t4a socket")?;
    let mut buf = serde_json::to_vec(req)?;
    buf.push(b'\n');
    stream.write_all(&buf).await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if line.is_empty() {
        bail!("t4a closed connection without responding");
    }
    let resp: Value = serde_json::from_str(line.trim())?;

    if resp.get("ok") == Some(&Value::Bool(true)) {
        Ok(resp)
    } else {
        let err = resp["error"].as_str().unwrap_or("unknown error");
        bail!("{err}")
    }
}

pub async fn screenshot(id: &str) -> Result<Vec<u8>> {
    let mut stream = UnixStream::connect(socket_path())
        .await
        .context("connect to t4a socket")?;
    let req = serde_json::json!({
        "cmd": "screenshot",
        "id": id,
        "cursor": true,
        "pad": 1,
        "scale": 66
    });
    let mut buf = serde_json::to_vec(&req)?;
    buf.push(b'\n');
    stream.write_all(&buf).await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if line.is_empty() {
        bail!("t4a closed connection without responding");
    }
    let header: Value = serde_json::from_str(line.trim())?;

    if header.get("ok") != Some(&Value::Bool(true)) {
        let err = header["error"].as_str().unwrap_or("screenshot failed");
        bail!("{err}");
    }

    let len = header["len"]
        .as_u64()
        .context("missing len in screenshot response")? as usize;
    let mut png = vec![0u8; len];
    reader.read_exact(&mut png).await?;

    Ok(png)
}

pub fn validate_event_type(event_type: &str) -> Result<()> {
    if VALID_EVENTS.contains(&event_type) {
        Ok(())
    } else {
        bail!("unknown event type: {event_type} (valid: {})", VALID_EVENTS.join(", "))
    }
}

pub async fn wait_for_event(
    terminal_id: &str,
    event_type: &str,
    timeout_ms: u64,
) -> Result<Value> {
    let mut stream = UnixStream::connect(socket_path())
        .await
        .context("connect to t4a socket")?;
    let req = serde_json::json!({
        "cmd": "events",
        "terminal": terminal_id
    });
    let mut buf = serde_json::to_vec(&req)?;
    buf.push(b'\n');
    stream.write_all(&buf).await?;

    let mut reader = BufReader::new(stream);
    let result = timeout(Duration::from_millis(timeout_ms), async {
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                bail!("connection closed");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(ev) = serde_json::from_str::<Value>(trimmed) {
                if ev.get("event").and_then(|e| e.as_str()) == Some(event_type) {
                    return Ok(ev);
                }
            }
        }
    })
    .await
    .with_context(|| format!("timeout waiting for {event_type} after {timeout_ms}ms"))??;

    Ok(result)
}
