//! The verdict of one arm and the exit-code contract the suites read.
//!
//! The suites parse *only the exit code* for pass and fail (`bare_metal_suite.sh`: rc 124/137 is a
//! crash, any other non-zero is a fail; `fast_suite.sh` reads the guest's client rc). The legacy
//! client additionally prints `RUNG_<arm>=NOTRUN`, which the guest suite scores as its own word. The
//! new client keeps those and gives `NOTRUN` its own exit code, 3, which `bare_metal_suite.sh` already
//! uses for "a harness precondition refused" (V3_RAW_CLIENT.md §4.2.5, §5.4). Mapping 3 to `NOTRUN` in
//! the suites is step S6 and is not done here.

/// The arm passed.
pub const EXIT_PASS: u8 = 0;
/// The arm ran and a check failed.
pub const EXIT_FAIL: u8 = 1;
/// The command line was refused (a value out of bounds, an unknown flag). Nothing ran.
pub const EXIT_USAGE: u8 = 2;
/// The arm could not run, or ran on a workload that did not qualify: it decides nothing.
pub const EXIT_NOTRUN: u8 = 3;
/// The watchdog expired (`timeout(1)` uses 124 for the same thing, and every suite scores it CRASH).
pub const EXIT_WATCHDOG: u8 = 124;

/// Why an arm did not run. Each is a stable kebab-case word the suites and the compare step may grep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRunReason {
    /// No RM device node could be opened (no GPU, no permission, a sandbox without `/dev`).
    NoRmDevice,
    /// `/proc/self/pagemap` shows page frames as zero: the caller lacks `CAP_SYS_ADMIN`.
    PagemapPfnHidden,
    /// `/proc/self/pagemap` or `/proc/meminfo` could not be read.
    ProcUnreadable,
    /// Transparent huge page backing could not be read, so "none" cannot be claimed.
    ThpUnmeasurable,
    /// The memory the workload needs does not fit the memory available or the cap.
    TooLittleMemory,
    /// The workload did not reach the requested fragmentation (or got huge pages, or lost pages).
    NotQualified,
    /// The host has no copy-engine class the oracle can use.
    NoCe,
    /// The GMMU small page, the kernel page and the requested leaf disagree.
    GrainMismatch,
    /// The host reported no guest-allocatable VA band, or one too small for the workload.
    NoGuestVaBand,
    /// A required host verb is missing (named in the detail).
    VerbMissing,
    /// The host answered, and refused by name: a driver tag with no row, an architecture with no
    /// family, a bring-up rung that failed. The detail carries the host's own words.
    HostRefused,
}

impl NotRunReason {
    /// The word printed after `reason=`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            NotRunReason::NoRmDevice => "no-rm-device",
            NotRunReason::PagemapPfnHidden => "pagemap-pfn-hidden",
            NotRunReason::ProcUnreadable => "proc-unreadable",
            NotRunReason::ThpUnmeasurable => "thp-unmeasurable",
            NotRunReason::TooLittleMemory => "too-little-memory",
            NotRunReason::NotQualified => "notqualified",
            NotRunReason::NoCe => "no-ce",
            NotRunReason::GrainMismatch => "grain-mismatch",
            NotRunReason::NoGuestVaBand => "no-guest-va-band",
            NotRunReason::VerbMissing => "verb-missing",
            NotRunReason::HostRefused => "host-refused",
        }
    }
}

/// One arm's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every check that could run passed, and at least one ran.
    Pass,
    /// A check failed; the text names which.
    Fail(String),
    /// Decides nothing.
    NotRun(NotRunReason, String),
}

impl Outcome {
    /// The process exit code for this verdict.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Outcome::Pass => EXIT_PASS,
            Outcome::Fail(_) => EXIT_FAIL,
            Outcome::NotRun(..) => EXIT_NOTRUN,
        }
    }

    /// `PASS`, `FAIL` or `NOTRUN`: the value of the `RUNG_<arm>=` token.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Outcome::Pass => "PASS",
            Outcome::Fail(_) => "FAIL",
            Outcome::NotRun(..) => "NOTRUN",
        }
    }

    /// The `RUNG_<rung>=...` line, with ` reason=<word> detail=<text>` appended for a not-run.
    /// `detail` is sanitised to one token: the line stays one line and splits on whitespace.
    #[must_use]
    pub fn rung_line(&self, rung: &str) -> String {
        match self {
            Outcome::Pass => format!("RUNG_{rung}=PASS"),
            Outcome::Fail(why) => format!("RUNG_{rung}=FAIL why={}", token(why)),
            Outcome::NotRun(r, d) => {
                format!("RUNG_{rung}=NOTRUN reason={} detail={}", r.word(), token(d))
            }
        }
    }
}

/// Collapse `s` into one whitespace-free token of at most 160 bytes of printable ASCII.
#[must_use]
pub fn token(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if out.len() >= 160 {
            break;
        }
        out.push(if c.is_ascii_graphic() && c != '=' {
            c
        } else {
            '_'
        });
    }
    if out.is_empty() {
        out.push('-');
    }
    out
}

/// One printable line of at most 200 characters: control characters (newlines above all) become `_`,
/// spaces and `=` are kept. For free text after a fixed prefix, where [`token`] would lose the `key=value`
/// pairs a reader wants.
#[must_use]
pub fn oneline(s: &str) -> String {
    s.chars()
        .take(200)
        .map(|c| if c.is_control() { '_' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_the_contract() {
        assert_eq!(Outcome::Pass.exit_code(), 0);
        assert_eq!(Outcome::Fail("x".into()).exit_code(), 1);
        assert_eq!(
            Outcome::NotRun(NotRunReason::NotQualified, "x".into()).exit_code(),
            3
        );
        assert_eq!(EXIT_USAGE, 2);
        assert_eq!(EXIT_WATCHDOG, 124);
        // The legacy suites map 124 to CRASH; 2 and 3 must stay distinct from it.
        let all = [EXIT_PASS, EXIT_FAIL, EXIT_USAGE, EXIT_NOTRUN, EXIT_WATCHDOG];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn rung_lines_match_the_legacy_tokens() {
        assert_eq!(
            Outcome::Pass.rung_line("refresh_stress"),
            "RUNG_refresh_stress=PASS"
        );
        assert_eq!(
            Outcome::Fail("oracle mismatch".into()).rung_line("a"),
            "RUNG_a=FAIL why=oracle_mismatch"
        );
        assert_eq!(
            Outcome::NotRun(NotRunReason::PagemapPfnHidden, "root needed".into()).rung_line("a"),
            "RUNG_a=NOTRUN reason=pagemap-pfn-hidden detail=root_needed"
        );
    }

    #[test]
    fn reason_words_are_unique_kebab_case() {
        use NotRunReason::*;
        let all = [
            NoRmDevice,
            PagemapPfnHidden,
            ProcUnreadable,
            ThpUnmeasurable,
            TooLittleMemory,
            NotQualified,
            NoCe,
            GrainMismatch,
            NoGuestVaBand,
            VerbMissing,
            HostRefused,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for r in all {
            let w = r.word();
            assert!(
                w.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
                "{w}"
            );
            assert!(seen.insert(w), "{w} duplicated");
        }
    }

    #[test]
    fn oneline_keeps_pairs_and_removes_line_breaks() {
        assert_eq!(oneline("a=1 b=2\nRUNG_x=PASS\r"), "a=1 b=2_RUNG_x=PASS_");
        assert_eq!(oneline(&"y".repeat(500)).len(), 200);
    }

    #[test]
    fn token_is_one_bounded_printable_word() {
        assert_eq!(token(""), "-");
        assert_eq!(token("a b=c\n\u{1F600}"), "a_b_c__");
        assert_eq!(token(&"x".repeat(1000)).len(), 160);
    }
}
