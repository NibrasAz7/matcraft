//! MatCraft language server — a minimal LSP endpoint for `.m` files.
//!
//! Speaks LSP 3.17 over `Content-Length`-framed stdio: full-document sync
//! with parse diagnostics, builtin/keyword/document-identifier completion,
//! builtin hover (signature + one-line doc) and `function` document
//! symbols. Diagnostics come from [`matlang::parser::parse_lines`], the
//! same front end the interpreter uses, so squiggles match `run` errors.
//!
//! Positions follow the spec (UTF-16 code units); conversion helpers bound
//! every index against the document text.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;
use std::io::{BufRead, Write};

use serde_json::{json, Value};

use matlang::ast::Stmt;

const PARSE_ERROR: i64 = -32700;
const METHOD_NOT_FOUND: i64 = -32601;

/// M-code keywords offered by completion. Mirrors the editor's list.
const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "else",
    "elseif",
    "end",
    "enumeration",
    "events",
    "for",
    "function",
    "global",
    "if",
    "methods",
    "otherwise",
    "persistent",
    "properties",
    "return",
    "switch",
    "try",
    "while",
];

/// The language server: open documents plus lifecycle state.
#[derive(Default)]
pub struct Server {
    docs: BTreeMap<String, String>,
    shutdown: bool,
}

impl Server {
    pub fn new() -> Self {
        Server::default()
    }

    /// Read framed LSP messages until `exit` or EOF.
    pub fn serve<R: BufRead, W: Write>(&mut self, mut input: R, mut output: W) -> std::io::Result<()> {
        while let Some(body) = read_message(&mut input)? {
            let msg: Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(_) => {
                    write_message(&mut output, &error(Value::Null, PARSE_ERROR, "parse error"))?;
                    continue;
                }
            };
            for reply in self.handle(&msg) {
                write_message(&mut output, &reply)?;
            }
            if self.shutdown && msg.get("method").and_then(Value::as_str) == Some("exit") {
                return Ok(());
            }
            if msg.get("method").and_then(Value::as_str) == Some("exit") {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Dispatch one message; returns responses and notifications to send.
    pub fn handle(&mut self, msg: &Value) -> Vec<Value> {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            ("initialize", Some(id)) => vec![response(id, initialize_result())],
            ("shutdown", Some(id)) => {
                self.shutdown = true;
                vec![response(id, Value::Null)]
            }
            ("textDocument/completion", Some(id)) => {
                vec![response(id, self.completions(&params))]
            }
            ("textDocument/hover", Some(id)) => vec![response(id, self.hover(&params))],
            ("textDocument/documentSymbol", Some(id)) => {
                vec![response(id, self.document_symbols(&params))]
            }
            ("textDocument/didOpen", _) | ("textDocument/didChange", _) => self.on_change(&params),
            ("textDocument/didClose", _) => {
                let uri = doc_uri(&params);
                self.docs.remove(&uri);
                vec![publish_diagnostics(&uri, &[])]
            }
            ("exit", _) | ("initialized", _) | ("$/cancelRequest", _) | ("$/setTrace", _) => Vec::new(),
            (_, Some(id)) => vec![error(id, METHOD_NOT_FOUND, format!("method not found: {method}"))],
            _ => Vec::new(),
        }
    }

    /// Store the new full text of a document and publish diagnostics.
    fn on_change(&mut self, params: &Value) -> Vec<Value> {
        let uri = doc_uri(params);
        let text = params
            .get("textDocument")
            .and_then(|t| t.get("text"))
            .and_then(Value::as_str)
            .or_else(|| {
                // didChange: take the last content change (full sync).
                params.get("contentChanges").and_then(|c| c.as_array()).and_then(|a| a.last()).and_then(|c| c.get("text")).and_then(Value::as_str)
            })
            .unwrap_or("")
            .to_string();
        self.docs.insert(uri.clone(), text);
        let diags = self.docs.get(&uri).map(|t| diagnostics(t)).unwrap_or_default();
        vec![publish_diagnostics(&uri, &diags)]
    }

    fn completions(&self, params: &Value) -> Value {
        let mut items = Vec::new();
        for (name, display, doc) in matlang::signatures::SIGNATURES {
            items.push(json!({
                "label": name,
                "kind": 3, // Function
                "detail": display,
                "documentation": {"kind": "markdown", "value": doc},
            }));
        }
        let covered: Vec<&str> = matlang::signatures::SIGNATURES.iter().map(|(n, _, _)| *n).collect();
        for name in matlang::BUILTIN_NAMES {
            if covered.iter().any(|c| c.eq_ignore_ascii_case(name)) {
                continue;
            }
            items.push(json!({"label": name, "kind": 3, "detail": "builtin"}));
        }
        for kw in KEYWORDS {
            items.push(json!({"label": kw, "kind": 14}));
        }
        // Identifiers in the open document (variables, local functions).
        if let Some(text) = self.docs.get(&doc_uri(params)) {
            for ident in identifiers(text) {
                items.push(json!({"label": ident, "kind": 6}));
            }
        }
        json!({"isIncomplete": false, "items": items})
    }

    fn hover(&self, params: &Value) -> Value {
        let Some(word) = word_at_position(self.docs.get(&doc_uri(params)), params) else {
            return Value::Null;
        };
        let Some((display, doc)) = matlang::signatures::builtin_signature(&word) else {
            return Value::Null;
        };
        json!({"contents": {"kind": "markdown", "value": format!("```matlab\n{display}\n```\n\n{doc}")}})
    }

    fn document_symbols(&self, params: &Value) -> Value {
        let Some(text) = self.docs.get(&doc_uri(params)) else {
            return Value::Null;
        };
        let Ok(stmts) = matlang::parser::parse_lines(text) else {
            return Value::Null;
        };
        let last_line = text.lines().count().saturating_sub(1) as u32;
        let mut out = Vec::new();
        for (idx, (line, stmt)) in stmts.iter().enumerate() {
            let Stmt::Function { name, params, outputs, .. } = stmt else { continue };
            let end_line = stmts.get(idx + 1).map(|(l, _)| (*l as u32).saturating_sub(1)).unwrap_or(last_line);
            let sig =
                format!("{}{}({})", if outputs.is_empty() { String::new() } else { format!("[{}] = ", outputs.join(", ")) }, name, params.join(", "));
            let start = (*line as u32).saturating_sub(1);
            out.push(json!({
                "name": name,
                "detail": sig,
                "kind": 12, // Function
                "range": {"start": {"line": start, "character": 0},
                          "end": {"line": end_line, "character": 0}},
                "selectionRange": {"start": {"line": start, "character": 0},
                                   "end": {"line": start, "character": 0}},
            }));
        }
        json!(out)
    }
}

fn doc_uri(params: &Value) -> String {
    params.get("textDocument").and_then(|t| t.get("uri")).and_then(Value::as_str).unwrap_or("").to_string()
}

fn initialize_result() -> Value {
    json!({
        "capabilities": {
            "positionEncoding": "utf-16",
            "textDocumentSync": {"openClose": true, "change": 1},
            "completionProvider": {"triggerCharacters": ["("]},
            "hoverProvider": true,
            "documentSymbolProvider": true
        },
        "serverInfo": {"name": "matcraft-lsp", "version": env!("CARGO_PKG_VERSION")}
    })
}

/// Parse diagnostics for a document — one error per failure, mapped onto
/// the reported 1-based line (clamped into the file).
fn diagnostics(text: &str) -> Vec<Value> {
    match matlang::parser::parse_lines(text) {
        Ok(_) => Vec::new(),
        Err(e) => {
            let line = (e.line as u32).saturating_sub(1);
            let col = text.lines().nth(line as usize).map(|l| l.encode_utf16().count() as u32).unwrap_or(0);
            vec![json!({
                "range": {"start": {"line": line, "character": 0},
                          "end": {"line": line, "character": col}},
                "severity": 1,
                "source": "matlang",
                "message": e.message,
            })]
        }
    }
}

fn publish_diagnostics(uri: &str, diags: &[Value]) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {"uri": uri, "diagnostics": diags}
    })
}

/// Identifier characters: letters, digits and `_`.
fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Distinct identifiers in a document (completion candidates).
fn identifiers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if is_ident(c) {
            cur.push(c);
        } else {
            if cur.chars().next().is_some_and(|f| f.is_alphabetic() || f == '_') && !out.contains(&cur) {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if cur.chars().next().is_some_and(|f| f.is_alphabetic() || f == '_') && !out.contains(&cur) {
        out.push(cur);
    }
    out
}

/// The identifier under an LSP position (`textDocument` + `position`).
fn word_at_position(text: Option<&String>, params: &Value) -> Option<String> {
    let text = text?;
    let line_no = params.get("position")?.get("line")?.as_u64()? as usize;
    let col16 = params.get("position")?.get("character")?.as_u64()? as usize;
    let line = text.lines().nth(line_no)?;
    // UTF-16 column → char index.
    let mut char_idx = line.chars().count();
    let mut u = 0usize;
    for (i, c) in line.chars().enumerate() {
        if u >= col16 {
            char_idx = i;
            break;
        }
        u += c.len_utf16();
    }
    let chars: Vec<char> = line.chars().collect();
    let mut start = char_idx.min(chars.len());
    let mut end = start;
    while start > 0 && chars.get(start - 1).is_some_and(|c| is_ident(*c)) {
        start -= 1;
    }
    while chars.get(end).is_some_and(|c| is_ident(*c)) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(chars[start..end].iter().collect())
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// Read one `Content-Length`-framed message; `None` at clean EOF.
fn read_message<R: BufRead>(input: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = None;
    loop {
        let mut header = String::new();
        let n = input.read_line(&mut header)?;
        if n == 0 {
            return Ok(None);
        }
        let h = header.trim();
        if h.is_empty() {
            if let Some(l) = len {
                let mut body = vec![0u8; l];
                input.read_exact(&mut body)?;
                return Ok(Some(body));
            }
            continue;
        }
        if let Some(v) = h.strip_prefix("Content-Length:") {
            len = v.trim().parse::<usize>().ok().filter(|l| *l <= 64 * 1024 * 1024);
        }
    }
}

fn write_message<W: Write>(output: &mut W, msg: &Value) -> std::io::Result<()> {
    let body = msg.to_string();
    write!(output, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(server: &mut Server, method: &str, params: Value, id: i64) -> Vec<Value> {
        server.handle(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
    }

    fn notify(server: &mut Server, method: &str, params: Value) -> Vec<Value> {
        server.handle(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn open_doc(server: &mut Server, uri: &str, text: &str) -> Vec<Value> {
        notify(server, "textDocument/didOpen", json!({"textDocument": {"uri": uri, "languageId": "matlab", "version": 1, "text": text}}))
    }

    #[test]
    fn lifecycle_and_diagnostics() {
        let mut s = Server::new();
        let r = call(&mut s, "initialize", json!({}), 1);
        assert!(r[0]["result"]["capabilities"]["hoverProvider"].as_bool().unwrap());

        // Clean doc → empty diagnostics.
        let n = open_doc(&mut s, "file:///a.m", "x = [1 2; 3 4]\ny = det(x)\n");
        assert_eq!(n[0]["method"], "textDocument/publishDiagnostics");
        assert_eq!(n[0]["params"]["diagnostics"].as_array().unwrap().len(), 0);

        // Broken doc → one error on the right line.
        let n = notify(
            &mut s,
            "textDocument/didChange",
            json!({"textDocument": {"uri": "file:///a.m"},
                   "contentChanges": [{"text": "x = [1 2\ny = ]\n"}]}),
        );
        let diags = n[0]["params"]["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1);
        assert!(diags[0]["range"]["start"]["line"].as_u64().is_some());
        assert!(diags[0]["message"].as_str().is_some());

        // Close clears diagnostics.
        let n = notify(&mut s, "textDocument/didClose", json!({"textDocument": {"uri": "file:///a.m"}}));
        assert_eq!(n[0]["params"]["diagnostics"].as_array().unwrap().len(), 0);

        let r = call(&mut s, "shutdown", Value::Null, 2);
        assert!(r[0]["result"].is_null());
    }

    #[test]
    fn completion_hover_symbols() {
        let mut s = Server::new();
        open_doc(&mut s, "file:///b.m", "function y = twice(u)\n  y = 2*u;\nend\nmyvar = eig(eye(3))\n");
        let r = call(&mut s, "textDocument/completion", json!({"textDocument": {"uri": "file:///b.m"}}), 3);
        let labels: Vec<&str> = r[0]["result"]["items"].as_array().unwrap().iter().filter_map(|i| i["label"].as_str()).collect();
        assert!(labels.contains(&"eig"));
        assert!(labels.contains(&"function"));
        assert!(labels.contains(&"myvar"));
        assert!(labels.contains(&"twice"));

        // Hover on `eye` (chars 12..14 of `myvar = eig(eye(3))` on line 3).
        let r = call(
            &mut s,
            "textDocument/hover",
            json!({"textDocument": {"uri": "file:///b.m"},
                   "position": {"line": 3, "character": 13}}),
            4,
        );
        let md = r[0]["result"]["contents"]["value"].as_str().unwrap_or("");
        assert!(md.contains("eye"), "{md}");

        // Hover on whitespace → null.
        let r = call(
            &mut s,
            "textDocument/hover",
            json!({"textDocument": {"uri": "file:///b.m"},
                   "position": {"line": 3, "character": 7}}),
            5,
        );
        assert!(r[0]["result"].is_null());

        let r = call(&mut s, "textDocument/documentSymbol", json!({"textDocument": {"uri": "file:///b.m"}}), 6);
        let syms = r[0]["result"].as_array().unwrap();
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0]["name"], "twice");
        assert_eq!(syms[0]["kind"], 12);
    }
}
