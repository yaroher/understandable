//! Static assets bundled into the binary at compile time.
//!
//! `dashboard/dist` becomes part of the `understandable` binary via
//! [`rust_embed`]. Phase 8 replaces the placeholder bundle with the
//! real React build.

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../dashboard/dist"]
pub struct Dashboard;
