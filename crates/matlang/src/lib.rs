//! MatCraft language engine.
//!
//! Lexer, parser and tree-walk interpreter for the M-code subset MatCraft
//! supports today: matrices, operators, control flow, indexing and the
//! builtin function library. See `docs/language.md` for the grammar.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod ast;
pub mod interp;
pub mod lexer;
pub mod matfile;
pub mod parser;

pub use interp::{describe, fmt_num, fmt_value, Breakpoint, CellArray, PauseInfo, Session, Value, BUILTIN_NAMES, DEBUG_PAUSE_ID};
// Re-export the numeric core types callers need to inspect `Value::Cplx`
// without taking a direct `matcore` dependency.
pub use matcore::{CMatrix, CNum};

/// Error type shared by the lexer, parser and interpreter.
#[derive(Debug, Clone, PartialEq)]
pub struct LangError {
    pub message: String,
    /// 1-based line number when known.
    pub line: usize,
    /// Error identifier (`error('my:id', …)`); empty for plain errors.
    /// Surfaces as `e.identifier` inside `catch e`.
    pub id: String,
}

impl LangError {
    pub fn new(message: impl Into<String>, line: usize) -> Self {
        LangError { message: message.into(), line, id: String::new() }
    }

    pub fn with_id(id: impl Into<String>, message: impl Into<String>, line: usize) -> Self {
        LangError { message: message.into(), line, id: id.into() }
    }
}

impl std::fmt::Display for LangError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "line {}: {}", self.line, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for LangError {}

/// Parse and execute `source` in a fresh session; returns echoed output.
pub fn eval(source: &str) -> Result<String, LangError> {
    let mut session = Session::new();
    session.run(source)
}
