//! The MatCraft engine.
//!
//! Every user-visible behaviour is a command with a stable id
//! (`eval.line`, `workspace.clear`, `figure.export`, …) run with JSON
//! parameters via [`Session::execute`]. The CLI, the control channel, the
//! MCP server and the Studio all reach the same commands — the ArtCraft
//! pattern shared with the sibling apps.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod cmd;

use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};

use serde_json::Value as Json;

pub use cmd::{command_specs, find_command, CommandSpec};
pub use matlang::LangError;

/// Errors commands return; they reach the user as a message, never a panic.
#[derive(Debug)]
pub enum EngineError {
    UnknownCommand(String),
    BadParams {
        cmd: String,
        msg: String,
    },
    Lang(LangError),
    /// A command panicked; the workspace was rolled back to its state
    /// before the call. This is a bug — please report it.
    Panic(String),
    Other(String),
}

impl EngineError {
    pub fn bad_params(cmd: &str, msg: impl Into<String>) -> Self {
        EngineError::BadParams { cmd: cmd.to_string(), msg: msg.into() }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::UnknownCommand(id) => write!(f, "unknown command `{id}`"),
            EngineError::BadParams { cmd, msg } => write!(f, "invalid parameters for `{cmd}`: {msg}"),
            EngineError::Lang(e) => write!(f, "{e}"),
            EngineError::Panic(cmd) => write!(f, "internal error in `{cmd}` (the workspace was kept as it was)"),
            EngineError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<LangError> for EngineError {
    fn from(e: LangError) -> Self {
        EngineError::Lang(e)
    }
}

/// A MatCraft session: the interpreter workspace plus open figures,
/// driven through commands.
pub struct Session {
    lang: matlang::Session,
}

impl Session {
    pub fn new() -> Self {
        Session { lang: matlang::Session::new() }
    }

    /// Deterministic RNG for reproducible runs and tests.
    pub fn with_seed(seed: u64) -> Self {
        Session { lang: matlang::Session::with_seed(seed) }
    }

    /// The language session (workspace, figures, search path).
    pub fn lang(&self) -> &matlang::Session {
        &self.lang
    }

    pub fn lang_mut(&mut self) -> &mut matlang::Session {
        &mut self.lang
    }

    /// Run command `id` with JSON `params`. A panic rolls the workspace
    /// back to its pre-call state and returns an error — the session
    /// stays usable.
    pub fn execute(&mut self, id: &str, params: Json) -> Result<Json, EngineError> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        let snapshot = self.lang.clone();
        match catch_unwind(AssertUnwindSafe(|| (spec.run)(self, &params))) {
            Ok(result) => result,
            Err(_) => {
                self.lang = snapshot;
                Err(EngineError::Panic(id.to_string()))
            }
        }
    }

    /// Every registered command.
    pub fn commands(&self) -> &'static [CommandSpec] {
        command_specs()
    }
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}
