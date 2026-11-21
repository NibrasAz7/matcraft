//! MatCraft Jupyter kernel core — transport-agnostic.
//!
//! Implements the Jupyter wire protocol pieces that do not depend on a
//! socket library: connection-file parsing, the multipart message model
//! with HMAC-SHA256 signing, and the kernel semantics (`kernel_info`,
//! `execute`, `is_complete`, `complete`, `shutdown`) mapped onto a
//! [`matlang::Session`]. Inbound and outbound messages are plain
//! [`WireMessage`]s; a ZeroMQ adapter (the canonical Jupyter transport)
//! plugs in later by ferrying these frames. This keeps the crate pure
//! Rust — `zmq` needs libzmq, which is a native-only dependency decision.
//!
//! Wire format reference (public spec):
//! `<https://jupyter-client.readthedocs.io/en/stable/messaging.html>`
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod conn;
mod kernel;
mod wire;

pub use conn::ConnectionInfo;
pub use kernel::Kernel;
pub use wire::{hmac_sign, Message, WireMessage, DELIM};
