//! The arm registry.
//!
//! One entry per arm: its name (the CLI flag is `--<name>`), the `RUNG_` token stem, a one-line doc, how
//! long it is expected to take natively and in the guest, and what it requires of the host. The
//! legacy client's 30 arms are **not** listed here: they run from `kayfabe-rm-ladder`, and the literal
//! `30` in the suites is untouched. The migration table (`ARMS.tsv`, with a `legacy|shadow|new` status
//! and a test that it equals the suites' default list) arrives with the facade in step S5, because
//! until a binary execs the legacy program the status column has nothing to say.

use crate::caps::Capability;

/// One arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmSpec {
    /// The name. Lower-case words joined by `-`; the flag is `--<name>`.
    pub name: &'static str,
    /// The stem of the `RUNG_<rung>=` token: the name with `_` for `-`.
    pub rung: &'static str,
    /// One line of what the arm proves.
    pub doc: &'static str,
    /// Expected seconds natively, including setup (the design's cap for a paid gate).
    pub expect_native_s: u32,
    /// Expected seconds in the guest.
    pub expect_guest_s: u32,
    /// What the arm requires of the host; checked against derived facts before it starts.
    pub requires: &'static [Capability],
}

/// The refresh stress: the Windows refresh shape recorded in V3_RAW_CLIENT.md section 4, reproduced with Linux RM verbs.
pub const REFRESH_STRESS: ArmSpec = ArmSpec {
    name: "refresh-stress",
    rung: "refresh_stress",
    doc: "fragmented guest memory pinned and mapped in 24 batches, one invalidate, oracle-checked, timed (V3_RAW_CLIENT.md section 4)",
    expect_native_s: 15,
    expect_guest_s: 60,
    requires: &[
        Capability::SmallPageMatchesLeaf,
        Capability::CopyEngine,
        Capability::GuestVaBand,
        Capability::PagemapPfn,
    ],
};

/// Every arm this binary can run itself.
pub const ARMS: &[ArmSpec] = &[REFRESH_STRESS];

/// Find an arm by name (without the leading `--`).
#[must_use]
pub fn find(name: &str) -> Option<&'static ArmSpec> {
    ARMS.iter().find(|a| a.name == name)
}

/// The `--list-arms` output, one machine-readable line per arm.
#[must_use]
pub fn list_lines() -> Vec<String> {
    ARMS.iter()
        .map(|a| {
            format!(
                "ARM name={} rung={} expect_native_s={} expect_guest_s={} requires={} doc={}",
                a.name,
                a.rung,
                a.expect_native_s,
                a.expect_guest_s,
                if a.requires.is_empty() {
                    "-".to_owned()
                } else {
                    a.requires
                        .iter()
                        .map(|c| c.word())
                        .collect::<Vec<_>>()
                        .join(",")
                },
                crate::outcome::oneline(a.doc)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_kebab_and_the_rung_stem_follows_the_name() {
        let mut seen = std::collections::BTreeSet::new();
        for a in ARMS {
            assert!(seen.insert(a.name), "{} twice", a.name);
            assert!(
                a.name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                    && !a.name.starts_with('-')
                    && !a.name.ends_with('-'),
                "{}",
                a.name
            );
            assert_eq!(a.rung, a.name.replace('-', "_"));
            assert!(!a.doc.is_empty());
            assert!(a.expect_native_s > 0 && a.expect_guest_s >= a.expect_native_s);
        }
    }

    #[test]
    fn the_stress_row_is_not_one_of_the_legacy_thirty() {
        // The legacy default list lives in scripts/fastguest/fast_suite.sh; the stress arm must not
        // collide with a name a suite already passes to the old binary.
        let suite = include_str!("../../../scripts/fastguest/fast_suite.sh");
        for a in ARMS {
            assert!(
                !suite.contains(&format!("--{} ", a.name))
                    && !suite.contains(&format!("--{})", a.name)),
                "{} is already a legacy arm",
                a.name
            );
        }
    }

    #[test]
    fn find_and_list() {
        assert_eq!(
            find("refresh-stress").map(|a| a.rung),
            Some("refresh_stress")
        );
        assert!(find("--refresh-stress").is_none());
        assert!(
            find("timer").is_none(),
            "legacy arms are not registered here"
        );
        let l = list_lines();
        assert_eq!(l.len(), ARMS.len());
        assert!(l[0].starts_with("ARM name=refresh-stress rung=refresh_stress expect_native_s=15 expect_guest_s=60 requires="));
        assert!(
            l[0].contains(
                "requires=small-page-matches-leaf,copy-engine,guest-va-band,pagemap-pfn "
            )
        );
        assert!(!l[0].contains('\n'));
    }
}
