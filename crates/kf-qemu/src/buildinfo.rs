//! The two lines that make a `qemu.log` self-describing (`docs/design/V3_RELAY_LIFECYCLE_LOG.md`): which build
//! wrote it, and, when the harness says so, which workload it ran. Printed once per run, at device realize; the
//! values come from the build and from the host's own environment, never from the guest.

/// `kf3: BUILD rev=<git sha|unknown> dirty=<0|1|unknown> pkg=<crate version>`.
///
/// The revision is the one `scripts/bench/build_kf3.sh` exports as `KF3_BUILD_REV` / `KF3_BUILD_DIRTY` for the
/// cargo build (no build script: a build made any other way prints `unknown`).
#[must_use]
pub fn build_line() -> String {
    build_line_from(option_env!("KF3_BUILD_REV"), option_env!("KF3_BUILD_DIRTY"))
}

pub(crate) fn build_line_from(rev: Option<&str>, dirty: Option<&str>) -> String {
    // Only a hex revision and a 0 or 1 are echoed: whatever else the environment of the build held is `unknown`.
    let rev = rev
        .filter(|r| !r.is_empty() && r.len() <= 40 && r.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or("unknown");
    let dirty = dirty
        .filter(|d| matches!(*d, "0" | "1"))
        .unwrap_or("unknown");
    format!(
        "kf3: BUILD rev={rev} dirty={dirty} pkg={}",
        env!("CARGO_PKG_VERSION")
    )
}

/// The environment variable of the optional workload label.
pub const WORKLOAD_ENV: &str = "KF3_WORKLOAD_LABEL";

/// `kf3: WORKLOAD label=<label>` when `KF3_WORKLOAD_LABEL` is set and non-empty.
#[must_use]
pub fn workload_line() -> Option<String> {
    workload_line_from(std::env::var(WORKLOAD_ENV).ok().as_deref())
}

/// The longest label echoed, in characters.
const LABEL_MAX: usize = 64;

pub(crate) fn workload_line_from(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let label: String = value
        .chars()
        .take(LABEL_MAX)
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._:/+=@-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(format!("kf3: WORKLOAD label={label}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_line_names_the_revision_and_dirtiness() {
        assert_eq!(
            build_line_from(Some("c25eb0c8"), Some("0")),
            format!(
                "kf3: BUILD rev=c25eb0c8 dirty=0 pkg={}",
                env!("CARGO_PKG_VERSION")
            )
        );
        assert!(build_line_from(Some("c25eb0c8"), Some("1")).contains(" dirty=1 "));
    }

    #[test]
    fn a_build_without_a_revision_says_unknown() {
        let l = build_line_from(None, None);
        assert!(
            l.starts_with("kf3: BUILD rev=unknown dirty=unknown pkg="),
            "{l}"
        );
        // anything that is not a hex revision / a 0 or 1 is not echoed
        let l = build_line_from(Some("v1; rm -rf /\nkf3: FORGED"), Some("maybe"));
        assert!(
            l.starts_with("kf3: BUILD rev=unknown dirty=unknown pkg="),
            "{l}"
        );
        assert!(!l.contains('\n'));
        assert!(build_line_from(Some(""), Some("")).contains("rev=unknown dirty=unknown"));
    }

    #[test]
    fn the_build_line_of_this_binary_is_one_line() {
        let l = build_line();
        assert!(l.starts_with("kf3: BUILD rev="), "{l}");
        assert!(!l.contains('\n') && l.len() < 128);
    }

    #[test]
    fn the_workload_label_is_optional_sanitized_and_capped() {
        assert_eq!(workload_line_from(None), None);
        assert_eq!(workload_line_from(Some("")), None);
        assert_eq!(workload_line_from(Some("   ")), None);
        assert_eq!(
            workload_line_from(Some("steelnomad_run5001")).as_deref(),
            Some("kf3: WORKLOAD label=steelnomad_run5001")
        );
        let hostile = workload_line_from(Some("a b\nkf3: FORGED\t=x")).unwrap();
        assert!(
            !hostile.contains('\n') && !hostile.contains('\t'),
            "{hostile:?}"
        );
        let label = hostile.strip_prefix("kf3: WORKLOAD label=").unwrap();
        assert!(
            !label.contains(' '),
            "the label is one key=value word: {hostile:?}"
        );
        let long = workload_line_from(Some(&"x".repeat(100_000))).unwrap();
        assert!(
            long.len() <= "kf3: WORKLOAD label=".len() + 64,
            "{}",
            long.len()
        );
    }
}
