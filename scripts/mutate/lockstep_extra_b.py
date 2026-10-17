"""Extra mutants of the soak / thread harness (fork H2): each is one textual replacement; `must` names the
suites that have to kill it (see lockstep_mutants.py)."""
B = "crates/kf-broker/src/"
MUTANTS = [
    ("held_slot_overwritten", B + "slots.rs",
     "        let mut m = held(s);", "        let mut m = 0;", "b threads"),
    ("no_change_with_a_frame_waiting", B + "ticks.rs",
     "            nothing_to_send\n                && self.ticks_since_open >= 1", "            true\n                && self.ticks_since_open >= 1", "b"),
    ("request_spent_twice", B + "ticks.rs",
     "        let r = self.open.take();\n        if r.is_some() {\n            self.c.frames_on_credit += 1;",
     "        let r = self.open;\n        if r.is_some() {\n            self.c.frames_on_credit += 1;", "b threads"),
    ("legacy_credit_used_while_live", B + "ticks.rs",
     "    if negotiated && live {\n        if open.is_some() {", "    if negotiated && live && false {\n        if open.is_some() {", "b threads"),
    ("commit_label_dropped", B + "conn.rs",
     ".map_or(0, |t| t.reqs.open().unwrap_or(0));", ".map_or(0, |_| 0);", "b threads"),
    ("reserved_tick_bits_accepted", "crates/kf-broker/src/wire.rs",
     "        if word & TICK_F_RESERVED != 0 {", "        if false {", "b"),
    ("tick_seq_zero_accepted", "crates/kf-broker/src/wire.rs",
     "        if tick_seq == 0 {", "        if false {", "b"),
]
