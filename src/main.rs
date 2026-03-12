mod client;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct T4aServer {
    tool_router: ToolRouter<Self>,
}

impl T4aServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_handler]
impl ServerHandler for T4aServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("t4a: terminals for agents. Manage PTY sessions, send input, observe via screenshots and text extraction, wait for events.")
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateParams {
    #[schemars(description = "Terminal width in columns (default: 80)")]
    cols: Option<u32>,
    #[schemars(description = "Terminal height in rows (default: 24)")]
    rows: Option<u32>,
    #[schemars(description = "Command to run (default: $SHELL)")]
    command: Option<Vec<String>>,
    #[schemars(description = "Working directory")]
    cwd: Option<String>,
    #[schemars(description = "Environment variables")]
    env: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IdParam {
    #[schemars(description = "Terminal ID (e.g. t1)")]
    id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SendParams {
    #[schemars(description = "Terminal ID (e.g. t1)")]
    id: String,
    #[schemars(description = "Text to send. JSON string — \\n is enter, \\t is tab. For control chars use input_base64 instead.")]
    input: Option<String>,
    #[schemars(description = "Base64-encoded raw bytes to send (for control characters like Ctrl+C = \\x03)")]
    input_base64: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TextParams {
    #[schemars(description = "Terminal ID (e.g. t1)")]
    id: String,
    #[schemars(description = "Start line from bottom (0 = last line)")]
    start: Option<u32>,
    #[schemars(description = "End line from bottom (e.g. 5 for last 5 lines)")]
    end: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ResizeParams {
    #[schemars(description = "Terminal ID (e.g. t1)")]
    id: String,
    #[schemars(description = "New width in columns")]
    cols: u32,
    #[schemars(description = "New height in rows")]
    rows: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaitParams {
    #[schemars(description = "Terminal ID (e.g. t1)")]
    id: String,
    #[schemars(description = "Event type: command_done, idle, activity, exit, bell, title")]
    event: String,
    #[schemars(description = "Timeout in milliseconds (default: 30000)")]
    timeout_ms: Option<u64>,
}

fn text_result(text: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn error_result(msg: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![Content::text(msg)]))
}

#[tool_router]
impl T4aServer {
    #[tool(description = "Create a new terminal session. Returns terminal ID for use with other tools.")]
    async fn t4a_create(
        &self,
        Parameters(p): Parameters<CreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut req = serde_json::json!({"cmd": "create"});
        if let Some(c) = p.cols { req["cols"] = c.into(); }
        if let Some(r) = p.rows { req["rows"] = r.into(); }
        if let Some(cmd) = p.command { req["cmd_args"] = cmd.into(); }
        if let Some(cwd) = p.cwd { req["cwd"] = cwd.into(); }
        if let Some(env) = p.env { req["env"] = serde_json::to_value(env).unwrap(); }
        match client::request(&req).await {
            Ok(v) => text_result(v.to_string()),
            Err(e) => error_result(e.to_string()),
        }
    }

    #[tool(description = "List all active terminal sessions.")]
    async fn t4a_list(&self) -> Result<CallToolResult, ErrorData> {
        match client::request(&serde_json::json!({"cmd": "list"})).await {
            Ok(v) => text_result(v.to_string()),
            Err(e) => error_result(e.to_string()),
        }
    }

    #[tool(description = "Send input to a terminal. The input string uses standard JSON escapes: \\n for Enter, \\t for Tab. For raw bytes (Ctrl+C, arrow keys), use input_base64 instead.")]
    async fn t4a_send(
        &self,
        Parameters(p): Parameters<SendParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut req = serde_json::json!({"cmd": "send", "id": p.id});
        if let Some(input) = p.input {
            req["input"] = input.into();
        } else if let Some(b64) = p.input_base64 {
            req["input_base64"] = b64.into();
        } else {
            return error_result("provide either input or input_base64".into());
        }
        match client::request(&req).await {
            Ok(v) => text_result(v.to_string()),
            Err(e) => error_result(e.to_string()),
        }
    }

    #[tool(description = "Take a screenshot of the terminal viewport. Returns a low-res PNG image optimized for LLM vision (~257 tokens). Use this to glance at overall terminal state.")]
    async fn t4a_screenshot(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, ErrorData> {
        match client::screenshot(&p.id).await {
            Ok((_header, png)) => {
                let b64 = BASE64.encode(&png);
                Ok(CallToolResult::success(vec![Content::image(b64, "image/png")]))
            }
            Err(e) => error_result(e.to_string()),
        }
    }

    #[tool(description = "Read exact text from terminal lines. Lines indexed from bottom: 0 = last line. Omit start/end for all lines. Use after a screenshot to get precise text.")]
    async fn t4a_text(
        &self,
        Parameters(p): Parameters<TextParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut req = serde_json::json!({"cmd": "text", "id": p.id, "trim": true});
        if let Some(s) = p.start { req["start"] = s.into(); }
        if let Some(e) = p.end { req["end"] = e.into(); }
        match client::request(&req).await {
            Ok(v) => text_result(v.to_string()),
            Err(e) => error_result(e.to_string()),
        }
    }

    #[tool(description = "Get cursor position and visibility state.")]
    async fn t4a_cursor(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, ErrorData> {
        match client::request(&serde_json::json!({"cmd": "cursor", "id": p.id})).await {
            Ok(v) => text_result(v.to_string()),
            Err(e) => error_result(e.to_string()),
        }
    }

    #[tool(description = "Resize a terminal. Sends SIGWINCH to the child process.")]
    async fn t4a_resize(
        &self,
        Parameters(p): Parameters<ResizeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match client::request(&serde_json::json!({"cmd": "resize", "id": p.id, "cols": p.cols, "rows": p.rows})).await {
            Ok(v) => text_result(v.to_string()),
            Err(e) => error_result(e.to_string()),
        }
    }

    #[tool(description = "Kill a terminal session. Sends SIGHUP to the shell process group.")]
    async fn t4a_kill(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, ErrorData> {
        match client::request(&serde_json::json!({"cmd": "kill", "id": p.id})).await {
            Ok(v) => text_result(v.to_string()),
            Err(e) => error_result(e.to_string()),
        }
    }

    #[tool(description = "Wait for a terminal event. Blocks until the event fires or timeout. Use after sending a command to wait for completion. Events: command_done (shell finished, has exit code), idle (no output for 2s), activity (output resumed), exit (process died), bell, title.")]
    async fn t4a_wait(
        &self,
        Parameters(p): Parameters<WaitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let timeout_ms = p.timeout_ms.unwrap_or(30000);
        match client::wait_for_event(&p.id, &p.event, timeout_ms).await {
            Ok(v) => text_result(v.to_string()),
            Err(e) => error_result(e.to_string()),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    client::ensure_daemon().await;
    let server = T4aServer::new();
    let transport = rmcp::transport::io::stdio();
    let ct = server.serve(transport).await?;
    ct.waiting().await?;
    Ok(())
}
