//! The terminal shell for prtrack: turning a decided board into text.
//!
//! # Why this is a library at all
//!
//! It is a library target inside the binary's own crate, so that the renderer
//! can be exercised from `tests/` and so that unused-yet items are part of an
//! API rather than dead code. It is emphatically **not** a reusable library:
//! everything here assumes a terminal, which is exactly what `prtrack-core`
//! forbids and why the two are separate crates. Nothing in this crate may be
//! depended upon by the macOS app or the hosted backend; they consume
//! `prtrack-core` and render for themselves.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod cli;
pub mod color;
pub mod exit;
pub mod json;
pub mod notify;
pub mod render;
pub mod screen;
pub mod state;
pub mod update;
pub mod upgrade;
pub mod watch;
pub mod width;
