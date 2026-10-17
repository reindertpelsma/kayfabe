# Extra mutants of the exhaustive model (form A); same tuple format as lockstep_mutants.py.
# (name, file, old, new, suites that MUST kill it)
B = "crates/kf-broker/src/"
D = "crates/kf-disp/src/"
MUTANTS = [
    ("a: NO_CHANGE ignores a ready frame", B + "ticks.rs",
     "            nothing_to_send\n                && self.ticks_since_open >= 1",
     "            true\n                && self.ticks_since_open >= 1", "a"),
    ("a: request spent twice (spend does not take)", B + "ticks.rs",
     "        let r = self.open.take();\n        if r.is_some() {\n            self.c.frames_on_credit += 1;",
     "        let r = self.open;\n        if r.is_some() {\n            self.c.frames_on_credit += 1;", "a"),
    ("a: vblank number skips (+2)", D + "pace.rs",
     "        self.seqs[h] += 1;", "        self.seqs[h] += 2;", "a"),
    ("a: mailbox ignores the copy in flight", D + "pace.rs",
     "        self.dirty && self.credit && !copy_in_flight", "        self.dirty && self.credit", "a"),
    ("a: mailbox keeps its credit after a copy", D + "pace.rs",
     "        self.dirty = false;\n        self.credit = false;\n        self.copies += 1;",
     "        self.dirty = false;\n        self.copies += 1;", "a"),
    ("a: silence is 500 ms", D + "tick.rs",
     "pub const BROKER_SILENCE_NS: u64 = 50_000_000;", "pub const BROKER_SILENCE_NS: u64 = 500_000_000;", "a"),
    ("a: probation never grows (always 1 gap)", D + "tick.rs",
     "    (1u32 << fallbacks.saturating_sub(1).min(3)).min(MAX_PROBATION_GAPS)", "    1", "a"),
    ("a: NO_CHANGE not paced (answer gap 0)", B + "ticks.rs",
     ".is_none_or(|t| now_ms >= t + self.answer_gap_ms())", ".is_none_or(|_| true)", "a"),
]
