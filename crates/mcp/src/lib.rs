//! MatCraft MCP server.
//!
//! JSON-RPC 2.0 over newline-delimited stdio, implementing the MCP
//! lifecycle plus `tools` and `resources`. Tools map onto engine commands:
//! `execute` reaches every command by id, `eval`/`run_script` are
//! conveniences, and `matcraft://workspace` exposes the variable table as a
//! resource.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use matcraft_engine::{EngineError, Session};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED: &[&str] = &[PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

const INSTRUCTIONS: &str = "MatCraft is a numerical computing environment (MATLAB-class, M-code \
compatible). Everything is a matrix; scalars are 1x1. Fastest path: `eval` with M-code like \
`A = [2 1; 1 3]; x = A \\ [5; 5]`, or `execute` with any command id (list_commands shows ids and \
params). Read matcraft://workspace for the variable table; plot* builtins create figures, \
figure.export writes SVG.";

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}
fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// Tool result content: one text block carrying the JSON payload.
fn tool_result(payload: Value) -> Value {
    json!({"content": [{"type": "text", "text": payload.to_string()}]})
}

/// A tool call that failed at the command level (still a successful
/// JSON-RPC reply — MCP convention is `isError` content).
fn tool_error(message: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": message.into()}], "isError": true})
}

/// The MCP server: owns a headless engine session.
pub struct Server {
    session: Session,
    initialized: bool,
}

impl Server {
    pub fn new() -> Self {
        Server { session: Session::new(), initialized: false }
    }

    /// Deterministic session (tests / reproducible tool calls).
    pub fn with_seed(seed: u64) -> Self {
        Server { session: Session::with_seed(seed), initialized: false }
    }

    /// Serve newline-delimited JSON-RPC until `input` closes.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if let Some(reply) = self.handle_line(&line) {
                output.write_all(reply.as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// One line in, optional reply out (notifications get none).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) if batch.is_empty() => Some(error(Value::Null, INVALID_REQUEST, "empty batch")),
            Ok(Value::Array(batch)) => {
                let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(msg) => self.handle(msg),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    fn handle(&mut self, msg: Value) -> Option<Value> {
        let Value::Object(o) = &msg else {
            return Some(error(Value::Null, INVALID_REQUEST, "message must be an object"));
        };
        let id = o.get("id").cloned();
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(error(id.unwrap_or(Value::Null), INVALID_REQUEST, "missing `method`"));
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        };
        Some(match self.request(method, &params) {
            Ok(r) => response(id, r),
            Err((code, m)) => error(id, code, m),
        })
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "ping" => Ok(json!({})),
            "initialize" => {
                let client_v = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED.contains(&client_v) { client_v } else { PROTOCOL_VERSION };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}, "resources": {}},
                    "serverInfo": {"name": "matcraft", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }))
            }
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "tools/call" => self.call_tool(params),
            "resources/list" => Ok(json!({"resources": [
                {"uri": "matcraft://workspace", "name": "Workspace variables",
                 "description": "Variable names, classes and sizes in the live session",
                 "mimeType": "application/json"},
                {"uri": "matcraft://commands", "name": "Command catalogue",
                 "description": "Every engine command id with params documentation",
                 "mimeType": "application/json"},
            ]})),
            "resources/read" => self.read_resource(params),
            _ => Err((METHOD_NOT_FOUND, format!("unknown method `{method}`"))),
        }
    }

    fn call_tool(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "tools/call needs `name`".to_string()))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        match name {
            "execute" => {
                let command = args.get("command").and_then(Value::as_str).ok_or((INVALID_PARAMS, "execute needs `command`".to_string()))?;
                let cmd_params = args.get("params").cloned().unwrap_or(json!({}));
                match self.session.execute(command, cmd_params) {
                    Ok(r) => Ok(tool_result(r)),
                    Err(e) => Ok(tool_error(e.to_string())),
                }
            }
            "eval" => self.call_engine_tool("eval.line", &args, "code"),
            "run_script" => self.call_engine_tool("script.run", &args, "path"),
            "list_commands" => match self.session.execute("engine.list", json!({})) {
                Ok(r) => Ok(tool_result(r)),
                Err(e) => Ok(tool_error(e.to_string())),
            },
            "workspace" => match self.session.execute("workspace.list", json!({})) {
                Ok(r) => Ok(tool_result(r)),
                Err(e) => Ok(tool_error(e.to_string())),
            },
            _ => Ok(tool_error(format!("unknown tool `{name}`"))),
        }
    }

    /// Convenience tools that wrap one engine command requiring a single
    /// named argument (`code`, `path`).
    fn call_engine_tool(&mut self, command: &str, args: &Value, arg: &str) -> Result<Value, (i64, String)> {
        let v = args.get(arg).and_then(Value::as_str).ok_or((INVALID_PARAMS, format!("{command} needs `{arg}`")))?;
        match self.session.execute(command, json!({arg: v})) {
            Ok(r) => Ok(tool_result(r)),
            Err(EngineError::Lang(e)) => Ok(tool_error(e.to_string())),
            Err(e) => Ok(tool_error(e.to_string())),
        }
    }

    fn read_resource(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let uri = params.get("uri").and_then(Value::as_str).ok_or((INVALID_PARAMS, "resources/read needs `uri`".to_string()))?;
        let result = match uri {
            "matcraft://workspace" => self.session.execute("workspace.list", json!({})),
            "matcraft://commands" => self.session.execute("engine.list", json!({})),
            _ => return Err((INVALID_PARAMS, format!("unknown resource `{uri}`"))),
        };
        match result {
            Ok(r) => Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": r.to_string()}]})),
            Err(e) => Err((INTERNAL_ERROR, e.to_string())),
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Server::new()
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "eval",
            "description": "Evaluate M-code in the session and return the echoed output (assignments without ';', disp, fprintf).",
            "inputSchema": {
                "type": "object",
                "properties": {"code": {"type": "string", "description": "M-code, e.g. \"x = mean(1:100)\""}},
                "required": ["code"],
            },
        },
        {
            "name": "run_script",
            "description": "Run a .m script file; its directory joins the function search path.",
            "inputSchema": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "Path to the .m file"}},
                "required": ["path"],
            },
        },
        {
            "name": "execute",
            "description": "Run any engine command by id with JSON params (list_commands shows the catalogue).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Command id, e.g. \"workspace.clear\""},
                    "params": {"type": "object", "description": "Command parameters"},
                },
                "required": ["command"],
            },
        },
        {
            "name": "list_commands",
            "description": "List every engine command id with its label and params documentation.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "workspace",
            "description": "List variables in the workspace: name, class, size.",
            "inputSchema": {"type": "object", "properties": {}},
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(server: &mut Server, msg: &str) -> Value {
        let out = server.handle_line(msg).unwrap_or_default();
        serde_json::from_str(&out).unwrap_or(Value::Null)
    }

    #[test]
    fn lifecycle_and_eval() {
        let mut s = Server::new();
        let init = line(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}"#);
        assert_eq!(init["result"]["serverInfo"]["name"], "matcraft");
        // Notifications get no reply.
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let r = line(&mut s, r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"eval","arguments":{"code":"x = 6 * 7"}}}"#);
        let text = r["result"]["content"][0]["text"].as_str().unwrap_or_default();
        assert!(text.contains("42"), "{text}");
    }

    #[test]
    fn execute_reaches_commands() {
        let mut s = Server::with_seed(1);
        let r = line(
            &mut s,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":{"command":"workspace.set","params":{"name":"A","value":[[1,2],[3,4]]}}}}"#,
        );
        assert!(r["result"].is_object());
        let w = line(&mut s, r#"{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"matcraft://workspace"}}"#);
        let text = w["result"]["contents"][0]["text"].as_str().unwrap_or_default();
        assert!(text.contains("\"A\""), "{text}");
    }

    #[test]
    fn bad_requests() {
        let mut s = Server::new();
        let r = line(&mut s, "not json");
        assert_eq!(r["error"]["code"], -32700);
        let r = line(&mut s, r#"{"jsonrpc":"2.0","id":9,"method":"bogus/method"}"#);
        assert_eq!(r["error"]["code"], -32601);
        let r = line(&mut s, r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"eval","arguments":{}}}"#);
        assert_eq!(r["error"]["code"], -32602);
    }

    #[test]
    fn tool_error_is_content_not_rpc_error() {
        let mut s = Server::new();
        let r = line(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"eval","arguments":{"code":"1 +"}}}"#);
        assert_eq!(r["result"]["isError"], true);
    }
}
