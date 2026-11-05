//! Every published exact branch remains its own question, even when values coincide.
use kf_rm::consttab::{self, BranchScope, DieScope};

#[test]
fn consttab_every_exact_row_selects_its_declared_branch() {
    let rows = consttab::table().unwrap();
    let mut checked = 0;
    for row in rows {
        if let (DieScope::Die(die), BranchScope::Major(branch)) = (&row.die, row.branch) {
            let hit = consttab::lookup_in(rows, &row.key, *die, branch).unwrap();
            assert_eq!(hit.branch, row.branch, "{} {die} branch {branch}", row.key);
            assert_eq!(hit, row);
            checked += 1;
        }
    }
    assert!(checked > 40);
}
