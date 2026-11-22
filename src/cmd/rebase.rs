//! `tt rebase [cible]` — re-parent the workspace's commits onto the target
//! bookmark. Shares the integration core with `tt merge` (which additionally
//! fast-forwards the target); runs no hooks of its own.

use crate::cmd::merge;
use crate::jj::Jj;

pub fn run(
    jj: &dyn Jj,
    target: Option<&str>,
) -> Result<merge::IntegrateOutcome, merge::MergeError> {
    merge::rebase(jj, target)
}
