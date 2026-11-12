//! `iohr permission-server`: the stdio MCP server Claude Code runs as its permission prompt
//! tool (`--permission-prompt-tool mcp__iohrperm__approve`,
//! <https://code.claude.com/docs/en/cli-reference>). It decides nothing: it hands each
//! `{tool_name, input, tool_use_id}` to the broker in the `iohr ask` process over a private
//! unix socket and returns the broker's `{behavior, ...}` as the tool's text, which is the
//! shape `PermissionResult` has (<https://code.claude.com/docs/en/agent-sdk/typescript>).
//! If the broker cannot be reached the answer is a deny.
//!
//! Handshake as Claude Code 2.1 speaks it: `server/discover`, `initialize`,
//! `notifications/initialized`, `tools/list`, then `tools/call`; each line is one JSON-RPC
//! message.

use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use serde_json::{Value, json};

use super::broker::{SERVER, TOOL};

/// The protocol version answered when the client names none.
const FALLBACK_VERSION: &str = "2025-06-18";

/// Runs the server on stdin and stdout against the broker's socket until stdin closes.
pub(crate) fn run(socket: &Path) -> std::io::Result<()> {
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    serve(stdin, stdout, |call| relay(socket, call))
}

/// One request to the broker and its one answer; any failure is a deny.
fn relay(socket: &Path, call: &Value) -> Value {
    let deny = |why: &str| json!({"behavior": "deny", "message": why});
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return deny("iohr cannot be asked: the ask is not running any more");
    };
    let mut line = call.to_string();
    line.push('\n');
    if stream.write_all(line.as_bytes()).is_err() {
        return deny("iohr cannot be asked");
    }
    let mut answer = String::new();
    match std::io::BufReader::new(stream).read_line(&mut answer) {
        Ok(n) if n > 0 => serde_json::from_str(&answer).unwrap_or_else(|_| deny("no answer")),
        _ => deny("iohr gave no answer"),
    }
}

/// The server loop, generic over its input and output and the broker call so a test can
/// drive it without a process.
pub(crate) fn serve<R, W, F>(input: R, mut output: W, mut ask: F) -> std::io::Result<()>
where
    R: BufRead,
    W: Write,
    F: FnMut(&Value) -> Value,
{
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // A message with no id is a notification: no answer.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let reply = match answer(method, msg.get("params"), &mut ask) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, text)) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}})
            }
        };
        writeln!(output, "{reply}")?;
        output.flush()?;
    }
    Ok(())
}

fn answer<F: FnMut(&Value) -> Value>(
    method: &str,
    params: Option<&Value>,
    ask: &mut F,
) -> Result<Value, (i32, String)> {
    match method {
        "server/discover" | "ping" => Ok(json!({})),
        "initialize" => {
            let version = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or(FALLBACK_VERSION);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")},
            }))
        }
        "tools/list" => Ok(json!({"tools": [{
            "name": TOOL,
            "description": "Asks the person at the terminal whether a tool call may run.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool_name": {"type": "string"},
                    "input": {"type": "object"},
                    "tool_use_id": {"type": "string"},
                },
                "required": ["tool_name", "input"],
            },
        }]})),
        "tools/call" => {
            let args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = ask(&args);
            Ok(json!({"content": [{"type": "text", "text": result.to_string()}]}))
        }
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn talk(lines: &[Value], reply: &Value) -> Vec<Value> {
        let input = lines
            .iter()
            .fold(String::new(), |s, l| s + &l.to_string() + "\n");
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, |_| reply.clone()).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn the_handshake_lists_one_tool_and_a_call_returns_the_brokers_answer_as_text() {
        let allow = json!({"behavior": "allow", "updatedInput": {"command": "ls"}});
        let out = talk(
            &[
                json!({"jsonrpc": "2.0", "id": 0, "method": "server/discover"}),
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                       "params": {"protocolVersion": "2025-11-25"}}),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                    "name": "approve",
                    "arguments": {"tool_name": "Bash", "input": {"command": "ls"}}}}),
                json!({"jsonrpc": "2.0", "id": 4, "method": "nope"}),
            ],
            &allow,
        );
        // The notification gets no reply, so five answers.
        assert_eq!(out.len(), 5, "{out:?}");
        assert_eq!(out[1]["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(out[2]["result"]["tools"][0]["name"], "approve");
        let text = out[3]["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), allow);
        assert_eq!(out[4]["error"]["code"], -32601);
    }

    #[test]
    fn a_broker_that_is_gone_is_a_deny() {
        let r = relay(Path::new("/nonexistent/iohr.sock"), &json!({}));
        assert_eq!(r["behavior"], "deny");
    }
}
