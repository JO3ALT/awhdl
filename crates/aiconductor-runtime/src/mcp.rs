use crate::config::{McpServerConfig, ProjectConfig};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

const MAX_PROTOCOL_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 256 * 1024;

pub struct McpClient;

impl McpClient {
    pub async fn call_tool(
        config: &ProjectConfig,
        server: &McpServerConfig,
        tool: &str,
        arguments: Value,
    ) -> Result<Value> {
        if !server.enabled {
            bail!("MCP server is disabled");
        }
        if server.transport != "stdio" {
            bail!("unsupported MCP transport: {}", server.transport);
        }
        let command = config.resolve_path(&server.command);
        let mut child = Command::new(&command)
            .current_dir(&config.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to start MCP server: {}", command.display()))?;

        let stdin = child.stdin.take().context("MCP stdin was not piped")?;
        let stdout = child.stdout.take().context("MCP stdout was not piped")?;
        let stderr = child.stderr.take().context("MCP stderr was not piped")?;
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).take(MAX_STDERR_BYTES as u64);
            let mut data = Vec::new();
            let _ = reader.read_to_end(&mut data).await;
            String::from_utf8_lossy(&data).into_owned()
        });

        let duration = Duration::from_secs(config.runtime.loop_limits.device_timeout_sec);
        let outcome = timeout(
            duration,
            call_tool_inner(&mut child, stdin, stdout, tool, arguments),
        )
        .await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        let stderr = stderr_task.await.unwrap_or_default();

        match outcome {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                if stderr.trim().is_empty() {
                    Err(error)
                } else {
                    Err(error.context(format!("MCP stderr: {}", stderr.trim())))
                }
            }
            Err(_) => bail!(
                "MCP tool call timed out after {} seconds",
                duration.as_secs()
            ),
        }
    }
}

/// One long-lived stdio MCP connection, for servers whose state lives in the
/// process (HumanPort keeps pending requests in memory).
pub struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl McpSession {
    pub async fn start(config: &ProjectConfig, server: &McpServerConfig) -> Result<Self> {
        if !server.enabled {
            bail!("MCP server is disabled");
        }
        if server.transport != "stdio" {
            bail!("unsupported MCP transport: {}", server.transport);
        }
        let command = config.resolve_path(&server.command);
        let mut child = Command::new(&command)
            .current_dir(&config.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to start MCP server: {}", command.display()))?;
        let stdin = child.stdin.take().context("MCP stdin was not piped")?;
        let stdout = BufReader::new(child.stdout.take().context("MCP stdout was not piped")?);
        let mut session = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let initialized = session
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "aiconductor", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
            .await?;
        if let Some(error) = initialized.get("error") {
            bail!("MCP initialize failed: {error}");
        }
        send(
            &mut session.stdin,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}),
        )
        .await?;
        Ok(session)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        send(
            &mut self.stdin,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .await?;
        receive_response(&mut self.child, &mut self.stdout, id).await
    }

    pub async fn tools(&mut self) -> Result<Vec<String>> {
        let response = self.request("tools/list", json!({})).await?;
        Ok(response
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// A JSON-RPC error is returned as `Err`; the tool's text content is parsed as JSON.
    pub async fn call(&mut self, tool: &str, arguments: Value) -> Result<Value> {
        let response = self
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await?;
        if let Some(error) = response.get("error") {
            bail!("MCP tool call failed: {error}");
        }
        let result = response
            .get("result")
            .context("MCP response has no result")?;
        let text = summarize_content(result);
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    pub async fn close(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

async fn call_tool_inner(
    child: &mut Child,
    mut stdin: ChildStdin,
    stdout: ChildStdout,
    tool: &str,
    arguments: Value,
) -> Result<Value> {
    let mut stdout = BufReader::new(stdout);
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "aiconductor", "version": env!("CARGO_PKG_VERSION")}
            }
        }),
    )
    .await?;
    let initialized = receive_response(child, &mut stdout, 1).await?;
    if let Some(error) = initialized.get("error") {
        bail!("MCP initialize failed: {error}");
    }
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )
    .await?;
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}
        }),
    )
    .await?;
    let response = receive_response(child, &mut stdout, 2).await?;
    if let Some(error) = response.get("error") {
        bail!("MCP tool call failed: {error}");
    }
    let result = response
        .get("result")
        .cloned()
        .context("MCP response has no result")?;
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        bail!("MCP tool returned an error: {}", summarize_content(&result));
    }
    Ok(result)
}

async fn send(stdin: &mut ChildStdin, value: &Value) -> Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

async fn receive_response(
    child: &mut Child,
    stdout: &mut BufReader<ChildStdout>,
    expected_id: i64,
) -> Result<Value> {
    loop {
        let mut line = String::new();
        let bytes = stdout.read_line(&mut line).await?;
        if bytes == 0 {
            let status = child.try_wait()?;
            bail!("MCP server closed stdout before response; status={status:?}");
        }
        if bytes > MAX_PROTOCOL_LINE_BYTES {
            bail!("MCP protocol line exceeded size limit");
        }
        let value: Value = serde_json::from_str(line.trim_end())
            .with_context(|| format!("malformed MCP stdout: {}", truncate(&line, 500)))?;
        if value.get("id").and_then(Value::as_i64) == Some(expected_id) {
            return Ok(value);
        }
        // Notifications and responses to unrelated request IDs are not control results.
    }
}

pub fn summarize_content(result: &Value) -> String {
    let Some(content) = result.get("content").and_then(Value::as_array) else {
        return result.to_string();
    };
    let mut output = Vec::new();
    for block in content {
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            output.push(text.to_owned());
        } else {
            output.push(block.to_string());
        }
    }
    output.join("\n")
}

fn truncate(value: &str, max: usize) -> &str {
    if value.len() <= max {
        value
    } else {
        let mut end = max;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        &value[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_blocks() {
        let value = json!({"content": [{"type": "text", "text": "ok"}]});
        assert_eq!(summarize_content(&value), "ok");
    }
}
