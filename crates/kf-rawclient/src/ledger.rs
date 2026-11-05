//! Named checks and the sink they print to.
//!
//! A verdict is computed from named checks, never from reaching the end of a program (the old tree's
//! "the last line is not the verdict"). Unlike `kf_harness::Ledger`, this one has a third state: a
//! check that **could not be made** is `SKIP`, printed and counted but neither a pass nor a fail. A
//! skip is never folded into a pass: the result line carries `skipped=<n>` and the check names.

use crate::outcome::oneline;

/// Where lines go. The real sink is stdout; tests collect.
pub trait Sink {
    /// Emit one line (no trailing newline).
    fn line(&mut self, s: &str);
}

/// Prints each line to stdout and flushes, so a watchdog `exit(124)` cannot lose a line.
#[derive(Debug, Default)]
pub struct StdoutSink;

impl Sink for StdoutSink {
    fn line(&mut self, s: &str) {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{s}");
        let _ = out.flush();
    }
}

/// Collects lines (tests, and the JSON record).
#[derive(Debug, Default)]
pub struct VecSink {
    /// Every line emitted, in order.
    pub lines: Vec<String>,
}

impl Sink for VecSink {
    fn line(&mut self, s: &str) {
        self.lines.push(s.to_owned());
    }
}

/// A check's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    /// Made, and held.
    Pass,
    /// Made, and did not hold.
    Fail,
    /// Could not be made (a verb is missing, not enough refreshes ran).
    Skip,
}

/// One named check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Stable lower-snake name.
    pub name: &'static str,
    /// Its state.
    pub state: CheckState,
    /// What was observed, or why it was skipped.
    pub why: String,
}

/// The most checks one run records. A run that would exceed it records an overflow failure instead of
/// growing: the ledger is bounded like everything else.
pub const MAX_CHECKS: usize = 64;

/// The run's checks.
#[derive(Debug, Default)]
pub struct Ledger {
    checks: Vec<Check>,
    overflowed: bool,
}

impl Ledger {
    /// Record and print `CHECK <name> PASS|FAIL <why>`.
    pub fn check(
        &mut self,
        sink: &mut dyn Sink,
        name: &'static str,
        pass: bool,
        why: impl Into<String>,
    ) {
        let state = if pass {
            CheckState::Pass
        } else {
            CheckState::Fail
        };
        self.push(sink, name, state, why.into());
    }

    /// Record and print `CHECK <name> SKIP <why>`.
    pub fn skip(&mut self, sink: &mut dyn Sink, name: &'static str, why: impl Into<String>) {
        self.push(sink, name, CheckState::Skip, why.into());
    }

    /// Record and print an already-built check (the gates of a pure verdict function).
    pub fn record(&mut self, sink: &mut dyn Sink, c: Check) {
        self.push(sink, c.name, c.state, c.why);
    }

    fn push(&mut self, sink: &mut dyn Sink, name: &'static str, state: CheckState, why: String) {
        if self.checks.len() >= MAX_CHECKS {
            self.overflowed = true;
            return;
        }
        let word = match state {
            CheckState::Pass => "PASS",
            CheckState::Fail => "FAIL",
            CheckState::Skip => "SKIP",
        };
        sink.line(&format!("CHECK {name} {word} {}", oneline(&why)));
        self.checks.push(Check { name, state, why });
    }

    /// The recorded checks.
    #[must_use]
    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    /// Names of the failed checks (an overflow counts as one).
    #[must_use]
    pub fn failed(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = self
            .checks
            .iter()
            .filter(|c| c.state == CheckState::Fail)
            .map(|c| c.name)
            .collect();
        if self.overflowed {
            v.push("ledger_overflow");
        }
        v
    }

    /// Names of the skipped checks.
    #[must_use]
    pub fn skipped(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|c| c.state == CheckState::Skip)
            .map(|c| c.name)
            .collect()
    }

    /// PASS only when at least one check passed and none failed. Zero passing checks is not a pass
    /// (a run that checked nothing says nothing).
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failed().is_empty() && self.checks.iter().any(|c| c.state == CheckState::Pass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_ledger_is_not_a_pass() {
        assert!(!Ledger::default().passed());
    }

    #[test]
    fn skips_are_neither_pass_nor_fail_and_stay_visible() {
        let mut s = VecSink::default();
        let mut l = Ledger::default();
        l.check(&mut s, "a", true, "ok");
        l.skip(&mut s, "b", "no probe verb");
        assert!(l.passed());
        assert_eq!(l.skipped(), vec!["b"]);
        assert_eq!(
            s.lines,
            vec!["CHECK a PASS ok", "CHECK b SKIP no probe verb"]
        );
        l.check(&mut s, "c", false, "bad");
        assert!(!l.passed());
        assert_eq!(l.failed(), vec!["c"]);
    }

    #[test]
    fn only_skips_is_not_a_pass() {
        let mut s = VecSink::default();
        let mut l = Ledger::default();
        l.skip(&mut s, "b", "x");
        assert!(!l.passed());
    }

    #[test]
    fn the_ledger_is_bounded_and_overflow_fails() {
        let mut s = VecSink::default();
        let mut l = Ledger::default();
        for _ in 0..(MAX_CHECKS + 10) {
            l.check(&mut s, "x", true, "");
        }
        assert_eq!(l.checks().len(), MAX_CHECKS);
        assert_eq!(s.lines.len(), MAX_CHECKS);
        assert!(!l.passed());
        assert_eq!(l.failed(), vec!["ledger_overflow"]);
    }
}
