//! Audit helper: capture this at the independently selected pre-change revision.
//! Never regenerate the regression fixture from an unreviewed policy candidate.
#[path = "../tests/support/capability_snapshot.rs"]
mod capability_snapshot;

fn main() {
    print!("{}", capability_snapshot::existing_policy_snapshot());
}
