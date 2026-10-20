import sys,re
src,dst=sys.argv[1],sys.argv[2]
s=open(src).read()
def rep(old,new,count=1):
    global s
    assert s.count(old)>=1, old[:80]
    s=s.replace(old,new) if count==0 else s.replace(old,new,count)
# header
i=s.index('#![allow(')
s=('''//! 2026-10-20 -- **Grader groups G2 / G3 / G4 / G5 / G6 / G7 on REAL threads** (`docs/design/V3_RELAY_GRADER_20261020.md`).
//!
//! GENERATED from `tests/relay_stress.rs` at the commit named in `HARNESS_FROM` by `scripts/relay_grade/mkstress.py` (the grader
//! keeps its own copy so that it grades every lineage the same way while the harness evolves); the grader's changes:
//! * STRICT: no SOFT class (`RS_STRICT` forced), the park INFINITE (`worker::set_park_net(false)`);
//! * G2 asks ONLY the kernel: every worker thread is `S` in `ep_poll` (`/proc/self/task/<tid>/{stat,wchan}`) and a zero-timeout
//!   `epoll_wait` on the workers' own eventfds is empty; the counter / timestamp "in flight" heuristics of the source are dropped
//!   (they can mask a real strand: a signal drained by a worker that then parks keeps "signal newer than wait end" true);
//! * 8..32 channels in 1..4 address spaces (one `pt` lock per address space);
//! * G4: `force_spin` -- an always-ready eventfd in every worker's epoll set (ignored by `on_other`), so the SAME loop never
//!   blocks: the spin reference;
//! * G5: a configurable quiet window (`quiet_ms`) and the wakeups-per-doorbell count.
//! Default suite: the `g*_` tests below (about a minute). `#[ignore]`d: the full seed list (300 short + 5 x 60 s) and the
//! nightly range (`RS_FROM` / `RS_TO`).
''')+s[i:]
# strict
rep('fn failed_run(o: &Outcome) -> bool {\n    !o.violations.is_empty() || (env_u("RS_STRICT", 0) != 0 && !o.soft.is_empty())',
    'fn failed_run(o: &Outcome) -> bool {\n    !o.violations.is_empty() || !o.soft.is_empty()')
# 8..32 channels (+ synthetic hot tokens): 64 tokens, 64 declared channels
rep('    let plane = Plane::new(&vmm, 16, 15);\n    let mut caps = VmCaps::from_declared(16, 8, 8, 8);','    let plane = Plane::new(&vmm, 64, 63);\n    let mut caps = VmCaps::from_declared(64, 8, 8, 8);')
# drop tests except the pin
a=s.index('/// The default-suite smoke')
b=s.index('/// The glue mirrored in this file')
s=s[:a]+'@@GRADE_TESTS@@\n'+s[b:]
rep('fn a_source_pin_the_mirrored_glue_still_matches_production()','fn g3_stress_harness_glue_still_matches_production()')
# monitor: kernel only
a=s.index('    let in_flight = || {\n        kernel_ready()')
b=s.index('    // demand (the retry set), a carried wait')
s=s[:a]+'''    // ★ GRADER G2: the kernel's view only. A wake in flight = a ready fd in the workers' own set, or an interrupt the fake
    // hardware has queued (it WILL signal). Blocked = every worker thread asleep in `ep_poll`.
    let in_flight = || kernel_ready() || irqs.lock().map_or(true, |q| !q.is_empty());
    let blocked = || {
        let st = worker_states();
        let _ = (ws, &sh.after_waits);
        st.len() == workers && st.iter().all(|x| x.contains(":S:") && x.contains("ep_poll"))
    };
'''+s[b:]
rep('''            let report = |m: String| {
                if env_u("RS_I6_SOFT", 0) != 0 {
                    sh.soft(m)
                } else {
                    sh.violation(m)
                }
            };''','''            let report = |m: String| sh.violation(m);''')
# pt per address space
rep('    pt: Mutex<()>,\n}','    pts: Vec<Mutex<()>>,\n}\nimpl Shared {\n    /// The `pt` lock of the address space client `idx` lives in.\n    fn pt(&self, idx: u32) -> &Mutex<()> {\n        &self.pts[idx as usize % self.pts.len()]\n    }\n}')
rep('self.sh.pt.try_lock().is_err() || self.busy()','self.sh.pt(self.c.idx).try_lock().is_err() || self.busy()',0)
rep('let pt = match self.sh.pt.try_lock() {','let pt = match self.sh.pt(ht).try_lock() {')
rep('let _g = sh.pt.lock();','let _g = sh.pt(i as u32).lock();')
rep('        pt: Mutex::new(()),','        pts: (0..sc.n_as.max(1)).map(|_| Mutex::new(())).collect(),')
# scenario fields
rep('    turn_us: u64,\n}','    turn_us: u64,\n    /// GRADER: address spaces (one `pt` lock each).\n    n_as: usize,\n    /// GRADER G4: the loop never blocks.\n    force_spin: bool,\n    /// GRADER G5: the quiet window after the run (ms).\n    quiet_ms: u64,\n}')
rep('            turn_us: 0,\n        };\n        sc.apply_overrides();','            turn_us: 0,\n            n_as: 1,\n            force_spin: false,\n            quiet_ms: 30,\n        };\n        sc.apply_overrides();')
# spin fd
rep('''                let poller = Poller::create().expect("epoll");''','''                let poller = Poller::create().expect("epoll");
                let spin_fd = Notifier::create().expect("spin fd");
                if sc.force_spin {
                    let _ = spin_fd.signal();
                    poller.watch(spin_fd.as_source_fd(), SPIN_TAG).unwrap();
                }''')
rep('''                let on_other = |tag: u64| {
                    if tag == worker::TICK_TAG {''','''                let on_other = |tag: u64| {
                    if tag == worker::TICK_TAG || tag == SPIN_TAG {''')
# quiet window
rep('            && sc.synth_hot == 0\n            && !sc.holder.on\n            && settled()\n        {','            && sc.synth_hot == 0\n            && !sc.holder.on\n            && !sc.force_spin\n            && settled()\n        {\n            sh.quiet_checked.store(true, Ordering::Release);')
rep('            std::thread::sleep(Duration::from_millis(30));\n            let after','            std::thread::sleep(Duration::from_millis(sc.quiet_ms));\n            let after')
rep('    pts: Vec<Mutex<()>>,\n}\nimpl Shared {','    pts: Vec<Mutex<()>>,\n    /// GRADER G5: the quiet window was checked (the run settled).\n    quiet_checked: AtomicBool,\n}\nimpl Shared {')
rep('        pts: (0..sc.n_as','        quiet_checked: AtomicBool::new(false),\n        pts: (0..sc.n_as')
# outcome extras
rep('    per_client: Vec<String>,\n}','    per_client: Vec<String>,\n    /// GRADER: parks, doorbells rung by the guests, whether the quiet window was checked, cold-client resp (p99 us, max ms).\n    parks: u64,\n    rings: u64,\n    quiet_checked: bool,\n    max_serves_during_resp: u64,\n}')
rep('        busy_looks: g(&stats.busy_looks),\n        per_client,','        busy_looks: g(&stats.busy_looks),\n        parks: g(&ws.parks),\n        rings: reps.iter().map(|r| r.1.rings).sum(),\n        quiet_checked: sh.quiet_checked.load(Ordering::Acquire),\n        max_serves_during_resp: reps.iter().map(|r| r.1.max_serves_during_resp).max().unwrap_or(0),\n        per_client,')
rep('const DB_OFF: u32 = 0x00BB_0090;','const DB_OFF: u32 = 0x00BB_0090;\n/// GRADER G4: the always-ready eventfd of the spin reference.\nconst SPIN_TAG: u64 = worker::OTHER_TAG_BASE + 7;')
s=s.replace('@@GRADE_TESTS@@\n', open(sys.argv[3]).read())
open(dst,'w').write(s)
