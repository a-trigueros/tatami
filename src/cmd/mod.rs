//! Commands: one module per user-facing command, orchestration only.
//!
//! A command module owns its rendering as a pure function so fast tests can
//! pin the display contract without invoking jj; the jj data itself is proven
//! by the adapter's integration tests.

pub mod create;
pub mod list;
pub mod remove;
pub mod shell;
pub mod switch;
