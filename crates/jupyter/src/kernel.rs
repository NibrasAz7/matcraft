//! Kernel semantics: request in, reply + iopub side-band messages out.

use serde_json::{json, Value};

use matlang::Session;

use crate::wire::{Message, WireMessage};

/// Completion candidates = builtins + keywords (same lists the LSP serves).
const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "else",
    "elseif",
    "end",
    "for",
    "function",
    "global",
    "if",
    "otherwise",
    "persistent",
    "return",
    "switch",
    "try",
    "while",
];

/// A kernel: one interpreter session plus Jupyter bookkeeping.
pub struct Kernel {
    session: Session,
    session_id: String,
    execution_count: u32,
    msg_seq: u64,
    dead: bool,
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Kernel {
    pub fn new() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let tag = std::process::id();
        #[cfg(target_arch = "wasm32")]
        let tag = 0u32;
        Kernel { session: Session::new(), session_id: format!("matcraft-{tag}"), execution_count: 0, msg_seq: 0, dead: false }
    }

    /// `true` after a `shutdown_request` — the transport should exit.
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Handle one wire message; returns the replies to emit (shell reply
    /// plus any iopub side-band traffic, in send order).
    pub fn handle(&mut self, req: &WireMessage) -> Vec<WireMessage> {
        match req.message.msg_type.as_str() {
            "kernel_info_request" => vec![self.reply(req, "kernel_info_reply", kernel_info())],
            "execute_request" => self.execute(req),
            "is_complete_request" => vec![self.reply(req, "is_complete_reply", self.is_complete(req))],
            "complete_request" => vec![self.reply(req, "complete_reply", self.complete(req))],
            "comm_info_request" => {
                vec![self.reply(req, "comm_info_reply", json!({"status": "ok", "comms": {}}))]
            }
            "shutdown_request" => {
                self.dead = true;
                let restart = req.message.content.get("restart").and_then(Value::as_bool).unwrap_or(false);
                vec![self.reply(req, "shutdown_reply", json!({"status": "ok", "restart": restart}))]
            }
            _ => Vec::new(),
        }
    }

    fn next_id(&mut self) -> String {
        self.msg_seq += 1;
        format!("{}-{}", self.session_id, self.msg_seq)
    }

    fn reply(&mut self, req: &WireMessage, msg_type: &str, content: Value) -> WireMessage {
        let msg_id = self.next_id();
        WireMessage {
            idents: req.idents.clone(),
            channel: req.channel,
            message: Message { msg_id, session: self.session_id.clone(), username: "matcraft".into(), msg_type: msg_type.into(), content },
            parent_header: req.orig_header.clone(),
            orig_header: json!({}),
        }
    }

    fn iopub(&mut self, req: &WireMessage, msg_type: &str, content: Value) -> WireMessage {
        let mut m = self.reply(req, msg_type, content);
        m.channel = "iopub";
        m.idents = Vec::new();
        m
    }

    fn is_complete(&self, req: &WireMessage) -> Value {
        let code = req.message.content.get("code").and_then(Value::as_str).unwrap_or("");
        let status = match matlang::parser::parse_lines(code) {
            Ok(_) => "complete",
            Err(e) => {
                // Unclosed block/atom → more input could finish it.
                let m = &e.message;
                if m.contains("missing 'end'") || m.contains("expected 'end'") || m.contains("unterminated") || m.contains("end of input") {
                    "incomplete"
                } else {
                    "invalid"
                }
            }
        };
        json!({"status": status})
    }

    fn complete(&self, req: &WireMessage) -> Value {
        let code = req.message.content.get("code").and_then(Value::as_str).unwrap_or("");
        let cursor = req.message.content.get("cursor_pos").and_then(Value::as_u64).map(|c| (c as usize).min(code.len())).unwrap_or(code.len());
        // Identifier immediately before the cursor.
        let head = &code[..cursor];
        let start = head.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map(|i| i + 1).unwrap_or(0);
        let token = &head[start..];
        let mut matches: Vec<&str> =
            matlang::BUILTIN_NAMES.iter().chain(KEYWORDS.iter()).filter(|n| !token.is_empty() && n.starts_with(token)).copied().collect();
        matches.sort_unstable();
        matches.dedup();
        json!({
            "status": "ok",
            "matches": matches,
            "cursor_start": start,
            "cursor_end": cursor,
            "metadata": {},
        })
    }

    /// `execute_request` → busy + execute_input + result/stream + idle on
    /// iopub, then `execute_reply` on the request's channel.
    fn execute(&mut self, req: &WireMessage) -> Vec<WireMessage> {
        let code = req.message.content.get("code").and_then(Value::as_str).unwrap_or("").to_string();
        let silent = req.message.content.get("silent").and_then(Value::as_bool).unwrap_or(false);
        let store_history = req.message.content.get("store_history").and_then(Value::as_bool).unwrap_or(!silent);

        let mut out = Vec::new();
        out.push(self.iopub(req, "status", json!({"execution_state": "busy"})));
        if !silent {
            self.execution_count += 1;
            out.push(self.iopub(req, "execute_input", json!({"code": code, "execution_count": self.execution_count})));
        }
        if !store_history {
            self.execution_count = self.execution_count.max(1);
        }

        match self.session.run(&code) {
            Ok(echo) => {
                if !silent {
                    if !echo.is_empty() {
                        out.push(self.iopub(req, "stream", json!({"name": "stdout", "text": echo})));
                    }
                    out.push(self.iopub(
                        req,
                        "execute_result",
                        json!({
                            "execution_count": self.execution_count,
                            "data": {"text/plain": "ok"},
                            "metadata": {},
                        }),
                    ));
                }
                out.push(self.iopub(req, "status", json!({"execution_state": "idle"})));
                out.push(self.reply(
                    req,
                    "execute_reply",
                    json!({
                        "status": "ok",
                        "execution_count": self.execution_count,
                        "user_expressions": {},
                    }),
                ));
            }
            Err(e) => {
                if !silent {
                    out.push(self.iopub(
                        req,
                        "error",
                        json!({
                            "ename": "MError",
                            "evalue": e.to_string(),
                            "traceback": [e.to_string()],
                        }),
                    ));
                }
                out.push(self.iopub(req, "status", json!({"execution_state": "idle"})));
                out.push(self.reply(
                    req,
                    "execute_reply",
                    json!({
                        "status": "error",
                        "execution_count": self.execution_count,
                        "ename": "MError",
                        "evalue": e.to_string(),
                        "traceback": [e.to_string()],
                    }),
                ));
            }
        }
        out
    }
}

/// `kernel_info_reply` content — protocol 5.3, M-code language info.
fn kernel_info() -> Value {
    json!({
        "status": "ok",
        "protocol_version": "5.3",
        "implementation": "matcraft",
        "implementation_version": env!("CARGO_PKG_VERSION"),
        "language_info": {
            "name": "matlab",
            "version": "",
            "mimetype": "text/x-octave",
            "file_extension": ".m",
            "pygments_lexer": "matlab",
            "codemirror_mode": "octave",
        },
        "banner": "MatCraft — a clean-room MATLAB-class numerical environment",
        "help_links": [],
        "debugger": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::WireMessage;

    fn req(msg_type: &str, content: Value) -> WireMessage {
        WireMessage {
            idents: vec![b"client".to_vec()],
            channel: "shell",
            message: Message { msg_id: "req-1".into(), session: "client-s".into(), username: "u".into(), msg_type: msg_type.into(), content },
            parent_header: json!({}),
            orig_header: json!({"msg_id": "req-1", "msg_type": msg_type}),
        }
    }

    #[test]
    fn kernel_info_and_complete() {
        let mut k = Kernel::new();
        let out = k.handle(&req("kernel_info_request", json!({})));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message.msg_type, "kernel_info_reply");
        assert_eq!(out[0].message.content["language_info"]["file_extension"], ".m");
        assert_eq!(out[0].idents, vec![b"client".to_vec()]);
        assert_eq!(out[0].parent_header["msg_id"], "req-1");

        let out = k.handle(&req("complete_request", json!({"code": "zer", "cursor_pos": 3})));
        let matches = out[0].message.content["matches"].as_array().unwrap();
        assert!(matches.iter().any(|m| m == "zeros"));
        assert_eq!(out[0].message.content["cursor_start"], 0);
    }

    #[test]
    fn execute_streams_output_and_counts() {
        let mut k = Kernel::new();
        let out = k.handle(&req("execute_request", json!({"code": "x = [1 2; 3 4]", "silent": false})));
        let types: Vec<&str> = out.iter().map(|m| m.message.msg_type.as_str()).collect();
        assert_eq!(types, vec!["status", "execute_input", "stream", "execute_result", "status", "execute_reply"]);
        assert_eq!(out[0].channel, "iopub");
        assert_eq!(out[5].channel, "shell");
        assert!(out[2].message.content["text"].as_str().unwrap().contains('4'));
        assert_eq!(out[5].message.content["execution_count"], 1);

        // A second cell shares the session.
        let out = k.handle(&req("execute_request", json!({"code": "det(x)", "silent": false})));
        assert_eq!(out[5].message.content["execution_count"], 2);
        assert!(out[2].message.content["text"].as_str().unwrap().contains("-2"));

        // Errors flow through iopub `error` + an error reply, kernel lives on.
        let out = k.handle(&req("execute_request", json!({"code": "nosuch(", "silent": false})));
        let types: Vec<&str> = out.iter().map(|m| m.message.msg_type.as_str()).collect();
        assert!(types.contains(&"error"));
        assert_eq!(out.last().unwrap().message.content["status"], "error");
        assert!(!k.is_dead());
    }

    #[test]
    fn is_complete_and_shutdown() {
        let mut k = Kernel::new();
        let out = k.handle(&req("is_complete_request", json!({"code": "x = 1"})));
        assert_eq!(out[0].message.content["status"], "complete");
        let out = k.handle(&req("is_complete_request", json!({"code": "for i = 1:3"})));
        assert_eq!(out[0].message.content["status"], "incomplete");
        let out = k.handle(&req("is_complete_request", json!({"code": "x = ]["})));
        assert_eq!(out[0].message.content["status"], "invalid");

        let out = k.handle(&req("shutdown_request", json!({"restart": false})));
        assert_eq!(out[0].message.msg_type, "shutdown_reply");
        assert!(k.is_dead());
    }
}
