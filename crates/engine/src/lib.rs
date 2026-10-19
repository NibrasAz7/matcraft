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

#[cfg(not(target_arch = "wasm32"))]
pub mod ai;
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
/// driven through commands. A SimCraft session is linked lazily — the
/// first `sim.*`/`model.*` command creates it (see [`Session::sim`]).
pub struct Session {
    lang: matlang::Session,
    /// Linked SimCraft session (the `sim('model.simcraft')` compute side);
    /// `None` until a sim command runs so plain sessions pay nothing.
    sim: Option<simcraft_engine::Session>,
    /// AI assistant state (provider config, workers, last results) — the
    /// `ai.*` command family. Absent on wasm, where craft-ai doesn't build.
    #[cfg(not(target_arch = "wasm32"))]
    ai: ai::Ai,
}

impl Session {
    pub fn new() -> Self {
        Session {
            lang: matlang::Session::new(),
            sim: None,
            #[cfg(not(target_arch = "wasm32"))]
            ai: ai::Ai::default(),
        }
    }

    /// Deterministic RNG for reproducible runs and tests.
    pub fn with_seed(seed: u64) -> Self {
        Session {
            lang: matlang::Session::with_seed(seed),
            sim: None,
            #[cfg(not(target_arch = "wasm32"))]
            ai: ai::Ai::default(),
        }
    }

    /// The language session (workspace, figures, search path).
    pub fn lang(&self) -> &matlang::Session {
        &self.lang
    }

    pub fn lang_mut(&mut self) -> &mut matlang::Session {
        &mut self.lang
    }

    /// The linked SimCraft session, created on first use. Its model, block
    /// library and `sim.run` all live there; this session's matlang
    /// workspace is the To Workspace target (via `simcraft_engine::publish`).
    pub fn sim(&mut self) -> &mut simcraft_engine::Session {
        self.sim.get_or_insert_with(simcraft_engine::Session::new)
    }

    /// The AI assistant state (native only — `ai.*` commands and the panel
    /// don't exist on wasm).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn ai(&self) -> &ai::Ai {
        &self.ai
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn ai_mut(&mut self) -> &mut ai::Ai {
        &mut self.ai
    }

    /// `ui.state`'s `ai` section — the same payload `ai.status` returns.
    /// `Null` on wasm (mirroring the sibling apps' pattern).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn ai_status(&self) -> Json {
        self.ai.status_json()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn ai_status(&self) -> Json {
        Json::Null
    }

    /// Apply AI worker results; `ai.ask`'s `var` injection writes into the
    /// workspace. Call once per frame — returns true when anything landed.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn ai_drain(&mut self) -> bool {
        self.ai.drain(&mut self.lang)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn ai_drain(&mut self) -> bool {
        false
    }

    /// True while an AI job is running — the UI repaints on a timer so
    /// progress narration keeps flowing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn ai_busy(&self) -> bool {
        self.ai.busy.is_some()
    }

    /// What survives a restart — `MatLayout`'s `ai` field (see ui-egui).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn ai_persist(&self) -> Json {
        self.ai.persist_json()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn ai_persist(&self) -> Json {
        Json::Null
    }

    /// Restore `ai_persist` output; malformed values are ignored.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn ai_restore(&mut self, v: &Json) {
        self.ai.restore(v);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn ai_restore(&mut self, _v: &Json) {}

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
