//! ★★★★★ **The completion OBSERVER — the guest's own declared completion, watched at the
//! address the guest itself named, and never written by us.**
//!
//! # What this is for
//!
//! `[measured 2026-08-10, boot `w218_cb6adcc_grfull`, `docs/design/gpu_promote_ctx.md`
//! §16.79]` `cuCtxCreate` submits one 86-method pushbuffer on a `GrCompute` channel and
//! then spins **in userspace** (`state=Rl`, `RIP` in `[vdso]` at `clock_gettime`). The last
//! method of that stream is `SET_REPORT_SEMAPHORE_A/B/C/D` naming GPU VA `0x2_0440fff0`,
//! payload `1`, `AWAKEN_ENABLE = 0`, `STRUCTURE_SIZE = FOUR_WORDS`.
//!
//! ★★★ **`AWAKEN_ENABLE = 0` is the whole design constraint.** The guest is asking for **no
//! interrupt**. The completion it waits for is *a value appearing at an address*, not an
//! event — so there is nothing for a notification plane to deliver on this one, and the only
//! honest thing a VMM can do at this rung is **watch that address and say what it holds**.
//!
//! # ⊘ THREE THINGS THIS DELIBERATELY DOES NOT DO
//!
//! 1. ⊘ **It never writes the semaphore.** The payload is a **literal immediate in the
//!    guest's own bytes** — invented by guest software, not derivable from the work — so an
//!    executor that runs those bytes is right by construction and anything that re-encodes
//!    them is right only by luck. Writing `1` here without running the work is exactly the
//!    credit-shortcut the C artifact named and rejected: *"the shortcut fakes the completion
//!    without running the work = the oracle's dead end (green poll, no matmul)"*
//!    (`C: docs/design/mode2_cuctxcreate_resume.md` §0.7). This module has **no writer** and
//!    no `gpa_write` in it; grep it.
//! 2. ⊘ **It never raises an interrupt.** `AWAKEN_ENABLE = 0`; a raise here would be an
//!    unattributable vector into an ISR that was not asked for.
//! 3. ⊘ **It resolves nothing.** The address is resolved **once**, by the caller, on the
//!    thread that already holds the locks a resolution needs, and handed here as a
//!    [`Site`]. A watch whose site is [`Site::Unresolved`] stays unresolved and says so by
//!    name; it is never retried against a second resolver, because two projections of one
//!    fact that disagree is this campaign's own most expensive failure class.
//!
//! # Where each half runs, and why the split is the point
//!
//! | phase | thread | what it may touch |
//! |---|---|---|
//! | **declare** — decode the operand, resolve the VA once, register | the **vCPU**, inside the locks it already holds for the ring read | everything it already had |
//! | **observe** — read the word, compare, verdict | the **reactor** thread | its own `Vmm` handle and this module's one leaf mutex. **No ranked lock, no device lock, no address table.** |
//!
//! ★ That is the same plan/execute split `verb_op` uses one layer down, and it is what keeps
//! the vCPU path from gaining a tenth blocking site: declaring is a `BTreeMap` insert under a
//! leaf mutex and nothing else.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use kayfabe_arch::ids::GpuVa;
use kayfabe_core::{ChanId, ProcId};

/// `NVC7C0_SET_OBJECT` — method address `0x0000`, one argument: the class bound to the
/// header's subchannel. `[src] ogkm-580: src/common/sdk/nvidia/inc/class/clc7c0.h:73`.
pub const SET_OBJECT: u32 = 0x0000;

/// `NVC7C0_SET_REPORT_SEMAPHORE_A` — `0x1b00`, the first of a four-word run.
/// `[src] ogkm-580: clc7c0.h:732`.
pub const SET_REPORT_SEMAPHORE_A: u32 = 0x1b00;

/// `AMPERE_COMPUTE_B`, the class `cuCtxCreate` binds and the only one this decoder answers
/// for. `[src] ogkm-580: clc7c0.h:63` (`NVC7C0_COMPUTE`). `[measured 2026-08-10, boot
/// `w218_cb6adcc_grfull`]` the bound class of subchannel 1 in `cuCtxCreate`'s pushbuffer.
pub const AMPERE_COMPUTE_B: u32 = 0xc7c0;

/// How long a declared completion is watched before the observer states what it saw.
///
/// ⚠ **A budget on OUR reporting, not on the guest.** The guest's own wait is unbounded
/// (a userspace spin); this deadline exists so that *"never observed"* is a statement made
/// at a named instant rather than an absence a reader has to infer. It is deliberately
/// shorter than `cup2`'s timeout so the verdict lands **inside** the boot log that contains
/// the doorbells it is about.
pub const OBSERVE_DEADLINE: Duration = Duration::from_secs(20);

/// ★★★ **The guest's declared completion, decoded from the guest's own method words.**
///
/// ⊘ There is **no public constructor and no `Default`**. The only way to obtain one is
/// [`decode_report_semaphore`], which reads it out of a method stream the guest wrote. That
/// is the same trick `VerbPlan::gated_doorbell` plays and it is here for the same reason: a
/// `DeclaredCompletion` that did not come from the guest's bytes must not be expressible,
/// because the entire correctness argument for passthrough is *"the payload is the guest's
/// literal"*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeclaredCompletion {
    /// The GPU VA the engine is told to write, `(A[7:0] << 32) | B[31:0]`.
    /// `[src] clc7c0.h:732-736`.
    pub va: GpuVa,
    /// The payload word, `C[31:0]`. ★ A **literal immediate**; nothing derives it.
    /// `[src] clc7c0.h:738-739`.
    pub payload: u32,
    /// `D[28:28] == FOUR_WORDS` — the engine writes a 16-byte report, not a bare dword.
    /// `[src] clc7c0.h:748-750`.
    pub four_words: bool,
    /// `D[20:20]` — whether the engine is asked to raise an awaken interrupt.
    /// `[src] clc7c0.h:745-747`. ★ Measured **0** for `cuCtxCreate`'s context init.
    pub awaken: bool,
    /// `D[1:0]` — `RELEASE` is `0`, `TRAP` is `3`. `[src] clc7c0.h:742-744`.
    pub operation: u8,
    /// The subchannel the run was written on, and the class bound to it by the preceding
    /// `SET_OBJECT`. ⊘ Carried because a method address means nothing without it: on this
    /// wire `NVC7C0_SET_CWD_REF_COUNTER = 0x0248` is `NVC7B5_SET_SEMAPHORE_PAYLOAD`.
    pub subch: u32,
    /// The class bound to [`DeclaredCompletion::subch`] when the run was seen.
    pub class_id: u32,
}

impl DeclaredCompletion {
    /// The number of bytes the engine writes at [`DeclaredCompletion::va`].
    ///
    /// ⊘ The observer reads only the **first four**, because only the first four are the
    /// payload; the other three words of a `FOUR_WORDS` report are a timestamp and are
    /// engine-written state this port models nothing about.
    #[must_use]
    pub fn report_bytes(&self) -> usize {
        if self.four_words { 16 } else { 4 }
    }
}

/// ★★★ **Decode a `SET_REPORT_SEMAPHORE` run out of a decoded method stream.**
///
/// `methods` is `(header, args)` as `kayfabe_rt::ceutils` produces it. The header's low 13
/// bits are the method address in dwords, bits 13..16 the subchannel, bits 16..29 the
/// argument count — the same arithmetic the §16.79 dump states, and the only claim made
/// about the header anywhere in this file.
///
/// # ⊘ Why it tracks `SET_OBJECT` instead of matching the address alone
///
/// The method address space is **per subchannel**. `[measured 2026-08-10, boot
/// `w218_cb6adcc_grfull`]` this very pushbuffer binds `AMPERE_COMPUTE_B` to subchannel 1 and
/// `AMPERE_DMA_COPY_B` to subchannel 4 in one stream. A decoder that matched `0x1b00`
/// without asking which class owns that subchannel would be answering about whichever class
/// happens to collide there — the single most dangerous confusion available on this wire.
///
/// Returns the **last** run in the stream: a pushbuffer may re-arm the registers, and the
/// completion the guest is waiting for is the one the engine reaches last.
#[must_use]
pub fn decode_report_semaphore(methods: &[(u32, Vec<u32>)]) -> Option<DeclaredCompletion> {
    let mut bound: [Option<u32>; 8] = [None; 8];
    let mut found: Option<DeclaredCompletion> = None;
    for (header, args) in methods {
        let addr = (header & 0x1fff) << 2;
        let subch = ((header >> 13) & 0x7) as usize;
        match addr {
            SET_OBJECT => {
                if let Some(class) = args.first() {
                    bound[subch] = Some(*class & 0xffff);
                }
            }
            SET_REPORT_SEMAPHORE_A => {
                // ⊘ The class gate. An unbound subchannel is NOT assumed to be compute:
                // "we did not see the SET_OBJECT" and "it is AMPERE_COMPUTE_B" are
                // different facts and only one of them licenses this decode.
                if bound[subch] != Some(AMPERE_COMPUTE_B) {
                    continue;
                }
                let [a, b, c, d] = *args.first_chunk::<4>()?;
                found = Some(DeclaredCompletion {
                    va: GpuVa((u64::from(a & 0xff) << 32) | u64::from(b)),
                    payload: c,
                    four_words: (d >> 28) & 1 == 0,
                    awaken: (d >> 20) & 1 == 1,
                    operation: (d & 0x3) as u8,
                    subch: subch as u32,
                    class_id: AMPERE_COMPUTE_B,
                });
            }
            _ => {}
        }
    }
    found
}

/// Where the declared VA landed when the declaring thread resolved it — **once**, under the
/// locks it already held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    /// The VA resolved to a guest-physical address the observer can read on its own thread.
    GuestRam {
        /// The guest-physical address of the payload word.
        gpa: u64,
    },
    /// The VA resolved into the emulated frame buffer.
    ///
    /// ⊘ **Not observable from the reactor thread at this rung, and said so rather than
    /// read through a second path.** The FB plane lives behind the register plane's session;
    /// reaching it off the vCPU thread is a lock-order question nobody has answered.
    Framebuffer {
        /// The resolved physical address inside the emulated FB.
        phys: u64,
    },
    /// The VA did not resolve. ⊘ **Named, never zeroed** — an unresolvable address is
    /// evidence about the address table, and decoding it to `0` would send the observer to
    /// read physical zero and report whatever is there.
    Unresolved(String),
}

/// What the observer saw. Every variant carries the address and the payload it is about, so
/// a line in a boot log is never separable from the completion it is a statement on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// ★ The declared payload appeared at the declared address. **Something satisfied the
    /// guest's wait**, and this observer read it out of guest memory rather than being told.
    Observed {
        /// The watch this is about.
        key: WatchKey,
        /// What the guest declared.
        decl: DeclaredCompletion,
        /// How long after the declaration the value appeared.
        after: Duration,
        /// How many reads it took.
        samples: u64,
    },
    /// ⊘ The deadline passed and the declared payload never appeared. The address WAS
    /// readable, so this is a statement about the completion plane, not about the observer.
    NotObserved {
        /// The watch this is about.
        key: WatchKey,
        /// What the guest declared.
        decl: DeclaredCompletion,
        /// The last value read at the address. ★ **Printed** — `0` and `garbage` are
        /// different facts, and only one of them is "nothing ever wrote here".
        last_seen: Option<u32>,
        /// How many reads were taken.
        samples: u64,
    },
    /// ⊘ The address could not be resolved, so **nothing was ever read** and no claim about
    /// the completion plane can be made from this row. It is a statement about the ADDRESS
    /// TABLE, and it names the dependency instead of faking it.
    Unobservable {
        /// The watch this is about.
        key: WatchKey,
        /// What the guest declared.
        decl: DeclaredCompletion,
        /// Why the declaring thread could not resolve the VA.
        why: String,
    },
    /// The observer's own read failed at a site that had resolved. ⚠ A defect in the
    /// observer or a region that went away; never read as "the value is not there".
    ReadRefused {
        /// The watch this is about.
        key: WatchKey,
        /// Why the read refused.
        why: String,
    },
}

impl Verdict {
    /// A one-line rendering for a boot log. Stable prefix `COMPLETION-WATCH`.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Verdict::Observed {
                key,
                decl,
                after,
                samples,
            } => format!(
                "COMPLETION-WATCH proc={} chan={} va=0x{:x} payload=0x{:08x} → OBSERVED \
                 after={}ms samples={samples} (the value the guest declared appeared at the \
                 address the guest declared; this observer READ it, it did not write it)",
                key.proc.0,
                key.chan.0,
                decl.va.0,
                decl.payload,
                after.as_millis(),
            ),
            Verdict::NotObserved {
                key,
                decl,
                last_seen,
                samples,
            } => format!(
                "COMPLETION-WATCH proc={} chan={} va=0x{:x} payload=0x{:08x} → NOT-OBSERVED \
                 samples={samples} last_seen={} awaken={} four_words={} ⊘ the address WAS \
                 readable and the declared payload never appeared — a statement about the \
                 completion plane, not about the observer",
                key.proc.0,
                key.chan.0,
                decl.va.0,
                decl.payload,
                last_seen.map_or_else(|| "NEVER-READ".into(), |v| format!("0x{v:08x}")),
                u8::from(decl.awaken),
                u8::from(decl.four_words),
            ),
            Verdict::Unobservable { key, decl, why } => format!(
                "COMPLETION-WATCH proc={} chan={} va=0x{:x} payload=0x{:08x} → UNOBSERVABLE \
                 ({why}) ⊘ NOTHING WAS READ. This row is about the ADDRESS TABLE and says \
                 nothing about whether the work ran; the dependency is named, not faked",
                key.proc.0, key.chan.0, decl.va.0, decl.payload,
            ),
            Verdict::ReadRefused { key, why } => format!(
                "COMPLETION-WATCH proc={} chan={} → READ-REFUSED ({why}) ⚠ the observer's \
                 own read failed at a site that HAD resolved; this is about the instrument",
                key.proc.0, key.chan.0,
            ),
        }
    }
}

/// The identity of one watched completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WatchKey {
    /// The guest process that declared it.
    pub proc: ProcId,
    /// The channel the declaring submission was on.
    pub chan: ChanId,
    /// The declared VA — part of the key, because one channel may re-arm the registers at a
    /// different address and those are two completions, not one.
    pub va: u64,
}

#[derive(Debug)]
struct Watch {
    decl: DeclaredCompletion,
    site: Site,
    declared: Instant,
    deadline: Instant,
    samples: u64,
    last_seen: Option<u32>,
    reported: bool,
}

/// Counters, so "the observer ran" is a quantity rather than the absence of a line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WatchStats {
    /// ★★★ **Times the vCPU path ASKED to declare** — bumped before every gate, including
    /// every arm that then refuses.
    ///
    /// ⊘ This is the REACHABILITY quantity and it is separate from `declared` on purpose.
    /// *"The observer was never reached"* and *"the observer was reached and found nothing
    /// to declare"* are the two readings a single counter cannot separate, and the first is
    /// exactly the severance this whole rung is about.
    pub attempts: u64,
    /// Distinct completions declared.
    pub declared: u64,
    /// Declarations that named a completion already being watched.
    pub redeclared: u64,
    /// Reads the observer performed. ★ **The non-vacuity quantity**: a sweep that never
    /// read is indistinguishable from one that read and saw nothing, unless this is printed.
    pub reads: u64,
    /// Verdicts emitted.
    pub verdicts: u64,
}

/// ★★★ **THE OBSERVER'S ONE CAPABILITY** — read four bytes at a guest-physical address.
///
/// ⊘ Named as a type so that what the observer can do is a *declaration* rather than an
/// incidental parameter shape. It reads. There is no write half, no raise half and no
/// resolve half, and adding one would mean changing this line — which is exactly the review
/// this rung wants a future edit to have to pass.
pub type GuestReader<'a> = &'a mut dyn FnMut(u64, &mut [u8; 4]) -> Result<(), String>;

/// ★★★ **The watch list.** One leaf mutex; no ranked lock is ever taken beneath it, and it
/// never calls into the device, the register plane or the address table.
#[derive(Debug, Default)]
pub struct WatchList {
    inner: std::sync::Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    watches: BTreeMap<WatchKey, Watch>,
    stats: WatchStats,
}

impl WatchList {
    /// An empty watch list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the declare path was entered. ★ Called BEFORE any gate, so a refusal
    /// still counts as *"the observer was reached"* — see [`WatchStats::attempts`].
    pub fn attempt(&self) {
        self.lock().stats.attempts += 1;
    }

    /// The instruments.
    #[must_use]
    pub fn stats(&self) -> WatchStats {
        self.lock().stats
    }

    /// How many completions are still being watched.
    #[must_use]
    pub fn live(&self) -> usize {
        self.lock().watches.values().filter(|w| !w.reported).count()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a completion the guest declared. **First declaration wins**: a channel rung
    /// 86 times declares the same completion 86 times, and re-arming the deadline on every
    /// doorbell would make a wall that never resolves look like one that is still young.
    ///
    /// ⊘ Takes no lock but its own, issues no verb, reads no memory. This is the whole of
    /// what the vCPU thread does.
    pub fn declare(&self, key: WatchKey, decl: DeclaredCompletion, site: Site, now: Instant) {
        let mut g = self.lock();
        if g.watches.contains_key(&key) {
            g.stats.redeclared += 1;
            return;
        }
        g.stats.declared += 1;
        g.watches.insert(
            key,
            Watch {
                decl,
                site,
                declared: now,
                deadline: now + OBSERVE_DEADLINE,
                samples: 0,
                last_seen: None,
                reported: false,
            },
        );
    }

    /// ★★★ **One observation pass.** For every live watch whose site resolved to guest RAM,
    /// `read` is asked for the payload word; the verdict is emitted the first time the value
    /// matches, or once the deadline has passed.
    ///
    /// `read` is `(gpa, &mut [u8; 4]) -> Result<(), String>` — deliberately the narrowest
    /// possible capability. ⊘ The observer is handed a **reader**. It is structurally unable
    /// to write, which is the property the whole module exists to guarantee.
    pub fn sweep(&self, now: Instant, read: GuestReader<'_>) -> Vec<Verdict> {
        // ★ The read runs INSIDE this guard, and that is deliberate: the guard is a leaf
        // mutex held only by this thread's sweeps and by `declare`'s map insert, and the
        // alternative — snapshot, read, re-lock, merge — reintroduces exactly the
        // two-projections-of-one-fact shape this campaign keeps paying for.
        let mut g = self.lock();
        let mut out = Vec::new();
        for (key, w) in &mut g.watches {
            if w.reported {
                continue;
            }
            match &w.site {
                Site::GuestRam { gpa } => {
                    let mut buf = [0u8; 4];
                    match read(*gpa, &mut buf) {
                        Ok(()) => {
                            w.samples += 1;
                            let v = u32::from_le_bytes(buf);
                            w.last_seen = Some(v);
                            if v == w.decl.payload {
                                w.reported = true;
                                out.push(Verdict::Observed {
                                    key: *key,
                                    decl: w.decl,
                                    after: now.saturating_duration_since(w.declared),
                                    samples: w.samples,
                                });
                            } else if now >= w.deadline {
                                w.reported = true;
                                out.push(Verdict::NotObserved {
                                    key: *key,
                                    decl: w.decl,
                                    last_seen: w.last_seen,
                                    samples: w.samples,
                                });
                            }
                        }
                        Err(why) => {
                            w.reported = true;
                            out.push(Verdict::ReadRefused { key: *key, why });
                        }
                    }
                }
                // ⊘ Nothing is read on either of these, and the verdict says so in its own
                // name. An `Unobservable` row must never be counted as evidence that the
                // completion did not land.
                Site::Framebuffer { phys } => {
                    if now >= w.deadline {
                        w.reported = true;
                        out.push(Verdict::Unobservable {
                            key: *key,
                            decl: w.decl,
                            why: format!(
                                "the VA resolves into the emulated framebuffer at \
                                 0x{phys:x}; the observer thread has no sanctioned path to \
                                 that plane"
                            ),
                        });
                    }
                }
                Site::Unresolved(why) => {
                    if now >= w.deadline {
                        w.reported = true;
                        out.push(Verdict::Unobservable {
                            key: *key,
                            decl: w.decl,
                            why: why.clone(),
                        });
                    }
                }
            }
        }
        g.stats.reads += out
            .iter()
            .filter(|v| matches!(v, Verdict::Observed { .. }))
            .count() as u64;
        // ⊘ Counted from the watches themselves, not from the verdicts: a sweep that read
        // ten times and emitted nothing still READ ten times, and that is the number that
        // makes "no verdict" legible.
        g.stats.reads = g.watches.values().map(|w| w.samples).sum();
        g.stats.verdicts += out.len() as u64;
        out
    }
}

kayfabe_util::assert_send_sync!(WatchList, WatchStats, DeclaredCompletion, WatchKey);

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the header word the way the wire does.
    fn hdr(addr: u32, subch: u32, count: u32) -> u32 {
        (addr >> 2) | (subch << 13) | (count << 16)
    }

    /// ★ `[measured 2026-08-10, boot `w218_cb6adcc_grfull`, `gpu_promote_ctx.md` §16.79]`
    /// `cuCtxCreate`'s shape: `SET_OBJECT 0xc7c0` on subchannel 1, then the four-word run
    /// naming `0x2_0440fff0` / payload 1 / `AWAKEN_ENABLE = 0` / `FOUR_WORDS`.
    fn ctx_init_stream() -> Vec<(u32, Vec<u32>)> {
        vec![
            (hdr(SET_OBJECT, 1, 1), vec![AMPERE_COMPUTE_B]),
            (hdr(0x0248, 1, 1), vec![0x0005_403f]),
            (
                hdr(SET_REPORT_SEMAPHORE_A, 1, 4),
                vec![0x0000_0002, 0x0440_fff0, 0x0000_0001, 0x0000_0000],
            ),
        ]
    }

    #[test]
    fn the_measured_ctxinit_semaphore_decodes_to_the_measured_operand() {
        let d = decode_report_semaphore(&ctx_init_stream()).expect("the run is present");
        assert_eq!(d.va, GpuVa(0x2_0440_fff0), "§16.79's measured VA");
        assert_eq!(d.payload, 1, "§16.79's measured payload, a LITERAL");
        assert!(
            !d.awaken,
            "★ AWAKEN_ENABLE=0 — the guest wants NO interrupt"
        );
        assert!(d.four_words, "STRUCTURE_SIZE=FOUR_WORDS");
        assert_eq!(d.operation, 0, "OPERATION=RELEASE");
        assert_eq!(d.report_bytes(), 16);
    }

    #[test]
    fn a_run_on_a_subchannel_bound_to_another_class_is_not_decoded() {
        // ⊘ THE COLLISION THIS GATE EXISTS FOR. Same address, subchannel bound to the copy
        // engine instead — on that class `0x1b00` is not a report semaphore at all.
        let stream = vec![
            (hdr(SET_OBJECT, 4, 1), vec![0xc7b5]),
            (
                hdr(SET_REPORT_SEMAPHORE_A, 4, 4),
                vec![0x0000_0002, 0x0440_fff0, 0x0000_0001, 0x0000_0000],
            ),
        ];
        assert_eq!(decode_report_semaphore(&stream), None);
    }

    #[test]
    fn an_unbound_subchannel_is_not_assumed_to_be_compute() {
        let stream = vec![(
            hdr(SET_REPORT_SEMAPHORE_A, 1, 4),
            vec![0x2, 0x0440_fff0, 1, 0],
        )];
        assert_eq!(decode_report_semaphore(&stream), None);
    }

    #[test]
    fn the_observer_reads_and_never_writes_and_says_when_the_value_appears() {
        let list = WatchList::new();
        let decl = decode_report_semaphore(&ctx_init_stream()).expect("decodes");
        let key = WatchKey {
            proc: ProcId(1),
            chan: ChanId(7),
            va: decl.va.0,
        };
        let t0 = Instant::now();
        list.declare(key, decl, Site::GuestRam { gpa: 0x1000 }, t0);

        // Sweep 1: the memory holds zero — no verdict, and the deadline has not passed.
        let mut mem = 0u32;
        let mut read = |_g: u64, b: &mut [u8; 4]| {
            *b = mem.to_le_bytes();
            Ok(())
        };
        assert!(list.sweep(t0, &mut read).is_empty());
        assert_eq!(
            list.stats().reads,
            1,
            "★ the read HAPPENED — the quantity that makes an empty verdict list legible"
        );

        // Sweep 2: something else wrote the guest's literal. The observer READS it.
        mem = 1;
        let mut read = |_g: u64, b: &mut [u8; 4]| {
            *b = mem.to_le_bytes();
            Ok(())
        };
        let v = list.sweep(t0, &mut read);
        assert!(
            matches!(v.as_slice(), [Verdict::Observed { .. }]),
            "got {v:?}"
        );
    }

    #[test]
    fn a_deadline_that_passes_with_the_wrong_value_is_not_observed_and_prints_what_it_saw() {
        let list = WatchList::new();
        let decl = decode_report_semaphore(&ctx_init_stream()).expect("decodes");
        let key = WatchKey {
            proc: ProcId(1),
            chan: ChanId(7),
            va: decl.va.0,
        };
        let t0 = Instant::now();
        list.declare(key, decl, Site::GuestRam { gpa: 0x1000 }, t0);
        let mut read = |_g: u64, b: &mut [u8; 4]| {
            *b = 0u32.to_le_bytes();
            Ok(())
        };
        let v = list.sweep(t0 + OBSERVE_DEADLINE, &mut read);
        match v.as_slice() {
            [Verdict::NotObserved { last_seen, .. }] => {
                assert_eq!(
                    *last_seen,
                    Some(0),
                    "⊘ 0 and 'never read' are different facts"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn an_unresolved_site_is_never_read_and_its_verdict_says_so() {
        let list = WatchList::new();
        let decl = decode_report_semaphore(&ctx_init_stream()).expect("decodes");
        let key = WatchKey {
            proc: ProcId(1),
            chan: ChanId(7),
            va: decl.va.0,
        };
        let t0 = Instant::now();
        list.declare(key, decl, Site::Unresolved("RING-VA-UNBOUND".into()), t0);
        let mut reads = 0u32;
        let mut read = |_g: u64, _b: &mut [u8; 4]| {
            reads += 1;
            Ok(())
        };
        let v = list.sweep(t0 + OBSERVE_DEADLINE, &mut read);
        assert_eq!(reads, 0, "⊘ NOTHING may be read at an unresolved site");
        assert!(
            matches!(v.as_slice(), [Verdict::Unobservable { .. }]),
            "got {v:?}"
        );
        assert!(v[0].line().contains("NOTHING WAS READ"));
    }

    #[test]
    fn redeclaring_does_not_re_arm_the_deadline() {
        let list = WatchList::new();
        let decl = decode_report_semaphore(&ctx_init_stream()).expect("decodes");
        let key = WatchKey {
            proc: ProcId(1),
            chan: ChanId(7),
            va: decl.va.0,
        };
        let t0 = Instant::now();
        list.declare(key, decl, Site::GuestRam { gpa: 0x1000 }, t0);
        // 86 doorbells later, at a much later instant.
        for _ in 0..85 {
            list.declare(
                key,
                decl,
                Site::GuestRam { gpa: 0x1000 },
                t0 + OBSERVE_DEADLINE,
            );
        }
        assert_eq!(list.stats().declared, 1);
        assert_eq!(list.stats().redeclared, 85);
        let mut read = |_g: u64, b: &mut [u8; 4]| {
            *b = 0u32.to_le_bytes();
            Ok(())
        };
        // ⚠ The deadline runs from the FIRST declaration. A re-armed one would still be
        // young here and the wall would never be reported.
        let v = list.sweep(t0 + OBSERVE_DEADLINE, &mut read);
        assert!(
            matches!(v.as_slice(), [Verdict::NotObserved { .. }]),
            "got {v:?}"
        );
    }
}
