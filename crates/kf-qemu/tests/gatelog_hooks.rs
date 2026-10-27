//! ★ DIAGNOSTIC ONLY — `KF3_DIAG_GATELOG` hook sites (source-level, GPU-free).
//!
//! Each hook of `kf_mem::gatelog` must exist the expected number of times, sit behind
//! `gatelog::enabled()` (so the default path does nothing), and run only on the thread the log
//! allows: the channel's relay thread (its own turn), the act thread (birth, free) and the VA
//! thread (applies, dumps) -- never a vCPU or the register drainer. The behaviour and bounds of the
//! recorder itself are pinned in `crates/kf-mem/tests/gatelog.rs`.

const USERPUMP: &str = include_str!("../src/userpump.rs");
const USERRELAY: &str = include_str!("../src/userrelay.rs");
const CHAN: &str = include_str!("../src/chan.rs");
const MEM: &str = include_str!("../src/mem.rs");
const DEVICE: &str = include_str!("../src/device.rs");
const VASMGR: &str = include_str!("../../kf-mem/src/vasmgr.rs");

/// Every occurrence of `needle` in `src`, with the 600 bytes before it.
fn sites<'a>(src: &'a str, needle: &str) -> Vec<(usize, &'a str)> {
    src.match_indices(needle)
        .map(|(at, _)| {
            let mut from = at.saturating_sub(600);
            while !src.is_char_boundary(from) {
                from -= 1;
            }
            (at, &src[from..at])
        })
        .collect()
}

/// `needle` occurs exactly `n` times and each is behind `gatelog::enabled()`.
fn gated(src: &str, needle: &str, n: usize) {
    let s = sites(src, needle);
    assert_eq!(
        s.len(),
        n,
        "`{needle}` must occur {n} time(s), found {}",
        s.len()
    );
    for (_, before) in s {
        assert!(
            before.contains("gatelog::enabled()"),
            "`{needle}` is not behind `gatelog::enabled()`"
        );
    }
}

fn body_of<'a>(src: &'a str, start: &str) -> &'a str {
    let at = src
        .find(start)
        .unwrap_or_else(|| panic!("`{start}` not found"));
    let rest = &src[at + start.len()..];
    // a method of an `impl` block ends at its closing brace, indented four spaces
    let end = rest.find("\n    }\n").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn every_gatelog_hook_exists_behind_the_flag() {
    gated(USERPUMP, "h.entry_seen(slot, entry);", 1);
    gated(USERPUMP, "h.put_seen(u32::from_le_bytes(put));", 2);
    gated(USERPUMP, "h.published();", 2);
    gated(USERPUMP, "h.pass();", 1);
    gated(USERPUMP, "h.reconcile(self.core.cursor);", 1);
    gated(USERPUMP, "kf_mem::gatelog::Point::Publish);", 1);
    gated(USERPUMP, "kf_mem::gatelog::Point::Put);", 1);
    assert_eq!(USERPUMP.matches("self.gatelog_published(r);").count(), 2);
    assert_eq!(USERPUMP.matches("self.gatelog_hold();").count(), 1);
    assert!(DEVICE.contains("(kf_mem::vahist::enabled() || kf_mem::gatelog::enabled())"));
    gated(USERRELAY, "rec.chan_birth(", 1);
    gated(CHAN, "rec.chan_free(", 1);
    gated(DEVICE, "rec.dump(\"xid\"", 1);
    gated(DEVICE, "rec.dump_shutdown(", 1);
    // vasmgr: the ticket is made only behind the flag, and ended only when there is one
    let s = sites(VASMGR, "global().begin_apply(");
    assert_eq!(s.len(), 1);
    assert!(s[0].1.contains("crate::gatelog::enabled()"));
    let s = sites(VASMGR, "global().end_apply(tk)");
    assert_eq!(s.len(), 1);
    assert!(s[0].1.contains("if let Some(tk) = gl_ticket"));
}

#[test]
fn the_apply_hooks_bracket_the_host_apply() {
    let b = VASMGR.find("global().begin_apply(").expect("begin");
    let a = VASMGR
        .find("apply_entry(&space.target, &e.runs, &cfg)")
        .expect("apply");
    let e = VASMGR.find("global().end_apply(tk)").expect("end");
    assert!(
        b < a && a < e,
        "the held evaluation is made BEFORE the host sees any run"
    );
    assert!(
        VASMGR[a..e].matches("apply_entry(").count() == 1,
        "one apply between the hooks"
    );
}

#[test]
fn gatelog_runs_only_on_the_va_thread_in_device_rs() {
    let va = DEVICE.find("pub fn va_loop(&self)").expect("va_loop");
    let next = DEVICE[va + 1..]
        .find("\n    pub fn ")
        .map_or(DEVICE.len(), |i| va + 1 + i);
    for (at, _) in DEVICE.match_indices("gatelog::") {
        assert!(
            at > va && at < next,
            "a `gatelog::` use outside `va_loop` (offset {at}): no vCPU or drainer path may call in"
        );
    }
    assert!(DEVICE.contains("gatelog::global()"));
}

#[test]
fn the_xid_dump_shares_vahists_trigger_and_the_shutdown_dump_follows_the_loop() {
    let x = DEVICE.find("rec.dump(\"xid\"").expect("xid dump");
    let poll = DEVICE[..x]
        .rfind("poll_trigger(path")
        .expect("poll_trigger before the dump");
    assert!(
        x - poll < 3000,
        "the dump is in the arm that answers the polled trigger"
    );
    assert!(
        DEVICE[poll..x].contains("Some(Ok(t)) =>"),
        "only a trigger that was actually written dumps"
    );
    assert!(
        DEVICE[..poll].contains("kf_mem::gatelog::enabled()"),
        "the poll honours our flag"
    );
    // the shutdown dump sits right after the closing brace of the `while !self.stop` loop
    let tail = [
        "            }\n        }\n",
        "        if self.latency_active {\n",
        "            kf_host::latency::dump_shutdown();\n",
        "        }\n",
        "        // \u{2298} DIAGNOSTIC ONLY (`KF3_DIAG_GATELOG=1`, default off): ",
        "the walk-gate log's summary at",
    ]
    .concat();
    assert!(
        DEVICE.contains(&tail),
        "the shutdown dump must follow the loop, not run inside it"
    );
}

#[test]
fn the_look_at_gp_put_is_made_only_when_the_gate_holds() {
    // the walk_busy gate: the hold (and the look) only in the branch that returns `true`
    let w = body_of(USERPUMP, "fn walk_busy(&mut self) -> bool {");
    let hold = w.find("self.gatelog_hold();").expect("hold in walk_busy");
    let ret = w.find("return true;").expect("return true");
    let pass = w.find("h.pass();").expect("pass");
    assert!(
        hold < ret && ret < pass,
        "hold before `return true`, pass only on the open path"
    );
    // the one extra USERD GP_PUT read lives in `gatelog_hold`, which is off without the flag
    let h = body_of(USERPUMP, "fn gatelog_hold(&mut self) {");
    assert!(h.contains("if !kf_mem::gatelog::enabled() {\n            return;"));
    assert!(h.contains("kf_abi::submit::USERD_GP_PUT"));
    assert!(h.contains("h.put_seen("), "the look feeds the log");
    assert!(
        h.contains(".is_ok()"),
        "a failed look is ignored, never an error"
    );
    // the look publishes nothing and decides nothing
    for forbidden in ["publish", "Failure", "?;"] {
        assert!(
            !h.contains(forbidden),
            "gatelog_hold must not contain `{forbidden}`"
        );
    }
}

#[test]
fn the_publication_gate_hook_sits_in_the_busy_branch_before_the_retry() {
    let g = sites(USERPUMP, "kf_mem::gatelog::Point::Publish);")[0].0;
    let busy = USERPUMP[..g]
        .rfind("if !me.inbox.user_source_ready(r.vas_key, r.idx, &r.walk_hold) {")
        .expect("busy branch");
    let retry = USERPUMP[g..]
        .find("return Err(Failure::Retry);")
        .expect("retry");
    assert!(
        g - busy < 900 && retry < 120,
        "the hook is inside the branch that returns Retry"
    );
}

#[test]
fn walk_busy_why_mirrors_walk_busy() {
    let wb = body_of(MEM, "pub fn walk_busy(&self) -> bool {");
    let why = body_of(MEM, "pub fn walk_busy_why(&self) -> u8 {");
    for cond in [
        "self.walk_busy.load(Ordering::Acquire)",
        "!self.all_settled()",
        "self.splits.lock()",
        "self.split_tokens.lock()",
    ] {
        assert!(
            wb.contains(cond),
            "walk_busy lost `{cond}`: update the gate log"
        );
        assert!(why.contains(cond), "walk_busy_why does not name `{cond}`");
    }
    for bit in [
        "WHY_VA_BUSY",
        "WHY_UNSETTLED",
        "WHY_SPLIT_QUEUED",
        "WHY_SPLIT_RUNNING",
    ] {
        assert!(why.contains(bit), "{bit}");
    }
    assert!(
        !why.contains("store("),
        "a diagnostic reads, it never writes the gate"
    );
}

#[test]
fn birth_and_free_hooks_run_after_the_locks_are_released() {
    // birth: on the act thread, after the vahist birth hook's own preconditions (the quota attach)
    let (at, before) = &sites(USERRELAY, "rec.chan_birth(")[0];
    assert!(before.contains("kf_mem::gatelog::enabled()"));
    assert!(
        USERRELAY[*at..]
            .find("Windows USER ring chan")
            .is_some_and(|d| d < 900)
    );
    // free: after the twin was released, never before `release_twin`
    let (at, _) = &sites(CHAN, "rec.chan_free(")[0];
    let rel = CHAN[..*at]
        .rfind("me.release_twin(c, t.tsg, t.ctx_share, t.chan)")
        .expect("release_twin");
    assert!(*at - rel < 2500);
}

#[test]
fn the_relay_hooks_sit_where_the_events_happen() {
    // GP_PUT seen: after the USERD read in `gp_put`
    let g = body_of(USERPUMP, "fn gp_put(&mut self) -> Turn<Option<u32>> {");
    let read = g.find("USER_RING_PUT_READ").expect("the read");
    let hook = g.find("h.put_seen(").expect("the hook");
    assert!(read < hook, "the log sees the value the relay read");
    // entry loaded: in `load_dword`, in the branch that snapshots the entry (`!hi`)
    let l = body_of(
        USERPUMP,
        "fn load_dword(&mut self, slot: u32, hi: bool) -> Turn<u32> {",
    );
    let snap = l.find("self.aux.source = Some(source);").expect("snapshot");
    let hook = l.find("h.entry_seen(slot, entry);").expect("the hook");
    let tail = l.find("Ok(if hi {").expect("the return");
    assert!(snap < hook && hook < tail);
    // entry published: after the guarded publication succeeded, in `publish_entry`
    let p = body_of(USERPUMP, "fn publish_entry(");
    let ok = p.find("*self.source_admitted = true;").expect("admitted");
    let hook = p.find("h.published();").expect("the hook");
    assert!(ok < hook, "never before the publication");
    // segment publications: both sites call the helper after their `publish(..)?`
    assert_eq!(USERPUMP.matches("self.gatelog_published(r);").count(), 2);
    for at in USERPUMP
        .match_indices("self.gatelog_published(r);")
        .map(|(i, _)| i)
    {
        let before = &USERPUMP[at.saturating_sub(400)..at];
        assert!(
            before.contains(")?;"),
            "after the publication that can fail"
        );
    }
    let helper = body_of(
        USERPUMP,
        "fn gatelog_published(&mut self, r: &userrelay::Record) {",
    );
    assert!(helper.contains("gatelog::enabled()") && helper.contains("h.published();"));
    // the cursor resync: after the turn body, before the lifecycle notices
    let t = body_of(
        USERPUMP,
        "fn turn(&mut self, me: &ChanPlane, r: &mut userrelay::Record) -> Turn<bool> {",
    );
    let body = t.find("self.turn_body(me, r)").expect("turn_body");
    let sync = t.find("h.reconcile(self.core.cursor);").expect("reconcile");
    let notices = t.find("self.report_lifecycle_notices();").expect("notices");
    assert!(body < sync && sync < notices);
}

/// Every use of a hook sits inside the block its own flag check opens (or after an early
/// `return` on the negated check in the same function): a hook whose check was dropped, or that
/// slid out of its `if`, is a hook that runs with the flag off.
#[test]
fn every_use_sits_in_the_block_its_flag_check_opens() {
    let uses = [
        "h.entry_seen(",
        "h.put_seen(",
        "h.published(",
        "h.pass(",
        "h.reconcile(",
        "h.gate(",
        "rec.chan_birth(",
        "rec.chan_free(",
        "rec.dump(",
        "rec.dump_shutdown(",
        "global().begin_apply(",
    ];
    let mut checked = 0;
    for (name, src) in [
        (
            "userpump",
            USERPUMP.split("#[cfg(test)]").next().unwrap_or(""),
        ),
        ("userrelay", USERRELAY),
        ("chan", CHAN),
        ("device", DEVICE),
        ("vasmgr", VASMGR),
    ] {
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !uses.iter().any(|u| line.contains(u)) || line.trim_start().starts_with("//") {
                continue;
            }
            // a definition inside kf_mem is not a use
            if line.contains("pub fn ") || line.contains("fn ") {
                continue;
            }
            let indent = |l: &str| l.len() - l.trim_start().len();
            let at = (0..i)
                .rev()
                .find(|&j| lines[j].contains("gatelog::enabled()"))
                .unwrap_or_else(|| panic!("{name}:{}: a hook with no flag check before it", i + 1));
            assert!(
                !lines[at].contains("vahist"),
                "{name}:{}: a hook must be gated by OUR flag alone, not by an either-or",
                i + 1
            );
            let k = indent(lines[at]);
            let early_return = lines[at].contains("!kf_mem::gatelog::enabled()");
            for l in &lines[at + 1..i] {
                let t = l.trim();
                if early_return {
                    assert!(
                        !t.starts_with("fn ") && !t.starts_with("pub fn "),
                        "{name}:{}: the early return is in another function",
                        i + 1
                    );
                } else {
                    assert!(
                        !(t.starts_with('}') && indent(l) <= k),
                        "{name}:{}: the block of the flag check closed before this hook",
                        i + 1
                    );
                }
            }
            checked += 1;
        }
    }
    assert!(checked >= 12, "only {checked} hook uses found");
}

const FFI: &str = include_str!("../src/ffi_unsafe.rs");

/// QEMU leaves by `exit`, which never reaches `kf3_unrealize` (run 5301: no `at stop` line, no
/// shutdown dump), so the shutdown dump is an `atexit` handler registered once at realize.
#[test]
fn the_shutdown_dump_is_an_atexit_handler_registered_at_realize_behind_the_flag() {
    assert!(
        FFI.contains("fn atexit("),
        "the libc declaration is in the unsafe file"
    );
    let h = body_of_fn(FFI, "extern \"C\" fn gatelog_atexit()");
    assert!(
        h.contains("dump_shutdown("),
        "the handler prints the shutdown dump"
    );
    assert!(
        h.contains("writeln!") && !h.contains("eprintln!"),
        "no panic out of exit"
    );
    let reg = FFI.find("atexit(gatelog_atexit)").expect("registration");
    let before = &FFI[reg.saturating_sub(700)..reg];
    assert!(
        before.contains("gatelog::enabled()"),
        "registered only with the flag"
    );
    assert!(before.contains("Once"), "registered once");
    // inside kf3_realize, on the success path (after the VA thread started)
    let realize = FFI
        .find("pub unsafe extern \"C\" fn kf3_realize(")
        .expect("realize");
    let va = FFI.find(".spawn(move || d.va_loop())").expect("va thread");
    assert!(realize < va && va < reg, "after the threads are up");
    assert!(reg - realize < 9000);
}

fn body_of_fn<'a>(src: &'a str, sig: &str) -> &'a str {
    let at = src.find(sig).unwrap_or_else(|| panic!("`{sig}` not found"));
    let rest = &src[at..];
    &rest[..rest.find("\n}\n").unwrap_or(rest.len())]
}
