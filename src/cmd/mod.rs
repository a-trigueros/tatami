//! Commands: one module per user-facing command, orchestration only.
//!
//! A command module owns its rendering as a pure function so fast tests can
//! pin the display contract without invoking jj; the jj data itself is proven
//! by the adapter's integration tests.

pub mod alias;
pub mod complete;
pub mod copy_ignored;
pub mod create;
pub mod list;
pub mod merge;
pub mod rebase;
pub mod remove;
pub mod shell;
pub mod step;
pub mod switch;

use crate::jj::{Jj, JjError};

/// Marker jj puts in its error when the working copy was left behind by
/// rewrites made elsewhere. shortcut: string-matched on stderr — revalidate
/// on jj upgrades.
const STALE_MARKER: &str = "The working copy is stale";

/// Runs `operation`, and on a stale-working-copy error heals once with
/// `jj workspace update-stale` (jj's own suggested, loss-less remedy) and
/// retries. Every command's first jj call goes through this — the failure
/// mode appeared empirically with `jj workspace list` after a rebase
/// rewrote the working-copy commit (revised: it does fail, contrary to the
/// tree-identical staleness probed in M1).
pub(crate) fn with_stale_recovery<T>(
    jj: &dyn Jj,
    mut operation: impl FnMut(&dyn Jj) -> Result<T, JjError>,
) -> Result<T, JjError> {
    match operation(jj) {
        Err(JjError::CommandFailed { stderr, .. }) if stderr.contains(STALE_MARKER) => {
            jj.workspace_update_stale()?;
            operation(jj)
        }
        other => other,
    }
}
