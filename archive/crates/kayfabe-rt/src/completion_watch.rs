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
                    // `SET_REPORT_SEMAPHORE_A_OFFSET_UPPER` is `7:0`. ⚠ That width is THIS
                    // method's, not the class's — see [`COMPUTE_ADDRESS_OPERANDS`] for the
                    // boot a single constant mask would have mis-reported.
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

// =========================================================================================
// ★★★★★ THE ADDRESS CENSUS — every 64-bit operand the GR pushbuffer names, not just the one
// the guest polls
// =========================================================================================

/// ★★★ **Every `_A`/`_B` 64-bit address operand `AMPERE_COMPUTE_B` defines**, derived from
/// the class header rather than hand-listed: an `_A` method at offset `o` whose `_B` sits at
/// `o + 4`. `[derived 2026-08-10 from ogkm-580: clc7c0.h]` — seventeen of them, and the
/// derivation is the point, because a hand-written list is a list of the ones somebody
/// happened to see.
///
/// ⚠⚠ **`_A` is the HIGH word, `_B` the LOW word, and the WIDTH OF `_A` IS NOT CONSTANT.**
/// The third column is the top bit of `_A`'s address field, read off the header per row:
/// `SET_REPORT_SEMAPHORE_A` and `SET_VALID_SPAN_OVERFLOW_AREA_A` are `7:0`, but
/// `SET_TEX_HEADER_POOL_A`, `SET_TEX_SAMPLER_POOL_A` and `SET_SHADER_SHARED_MEMORY_WINDOW_A`
/// are `16:0`, and `SEND_PCAS_A` / `SET_INLINE_QMD_ADDRESS_A` are `31:0`.
///
/// ★ `[measured, and it bit me before the boot did]` this decoder was first written with the
/// single `0xff` mask [`decode_report_semaphore`] uses — correct for the one method that
/// decoder answers for — and `the_census_names_every_address_the_measured_pushbuffer_dereferences`
/// went red on the `cuCtxCreate` stream: `SET_TEX_HEADER_POOL`'s measured `A = 0x00000100`
/// masked to **zero**, reporting VA `0x0` for an operand actually at `0x100_00000000`.
/// ⊘ Three of the five operands the guest names would have been reported at the wrong
/// address — and the wrong address here is `0`, which reads as *"the guest named nothing"*.
/// **The generalisation of a correct decoder is not a correct decoder**, which is why the
/// width is a column and not a constant.
pub const COMPUTE_ADDRESS_OPERANDS: &[(u32, &str, u32)] = &[
    (0x0104, "SET_NOTIFY", 7),
    (0x0130, "SET_GLOBAL_RENDER_ENABLE", 7),
    (0x01dc, "SET_I2M_SEMAPHORE", 7),
    (0x0200, "SET_VALID_SPAN_OVERFLOW_AREA", 7),
    (0x0214, "SET_QMD_VIRTUALIZATION_BASE", 7),
    (0x02a0, "SET_SHADER_SHARED_MEMORY_WINDOW", 16),
    (0x02b4, "SEND_PCAS", 31),
    (0x02e4, "SET_SHADER_LOCAL_MEMORY_NON_THROTTLED", 7),
    (0x0318, "SET_INLINE_QMD_ADDRESS", 31),
    (0x0550, "SET_MME_MEM_ADDRESS", 16),
    (0x0790, "SET_SHADER_LOCAL_MEMORY", 16),
    (0x07b0, "SET_SHADER_LOCAL_MEMORY_WINDOW", 16),
    (0x1550, "SET_RENDER_ENABLE", 7),
    (0x155c, "SET_TEX_SAMPLER_POOL", 16),
    (0x1574, "SET_TEX_HEADER_POOL", 16),
    (0x1b00, "SET_REPORT_SEMAPHORE", 7),
    (0x25f8, "SET_TRAP_HANDLER", 16),
];

/// `NVC7C0_LOAD_MME_INSTRUCTION_RAM` — `0x0118`. `[src] ogkm-580: clc7c0.h:58`.
///
/// ★★★★★ **This is not an address and it is the most important method in the stream.** The
/// MME is the Macro Method Expander: a small programmable unit in the GR front end whose
/// **output is methods**. Guest-authored microcode loaded here can emit method words that
/// were never in the pushbuffer anybody inspected. ⇒ **No allowlist over decoded methods can
/// be sound while this method is admitted**, so the containment argument for executing a
/// guest GR pushbuffer cannot rest on method inspection at all. See
/// `docs/design/gr_execution_boundary.md` §2.
pub const LOAD_MME_INSTRUCTION_RAM: u32 = 0x0118;

/// The inclusive low-`hi_bit` mask for an `_A` word's address field.
const fn mask_to(hi_bit: u32) -> u32 {
    if hi_bit >= 31 {
        u32::MAX
    } else {
        (1u32 << (hi_bit + 1)) - 1
    }
}

/// One 64-bit address operand the guest's compute pushbuffer named.
///
/// ⊘ No public constructor and `#[non_exhaustive]`, for the same reason as
/// [`DeclaredCompletion`]: an address claiming to be one the guest named, that did not come
/// from the guest's bytes, must not be expressible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AddressOperand {
    /// The `_A` method offset, i.e. the row of [`COMPUTE_ADDRESS_OPERANDS`].
    pub method: u32,
    /// That row's name.
    pub name: &'static str,
    /// The 64-bit GPU VA, `(A[7:0] << 32) | B[31:0]`.
    pub va: GpuVa,
    /// The subchannel it was written on.
    pub subch: u32,
}

/// ★★★★★ **What a guest GR pushbuffer would make the host engine TOUCH.**
///
/// Every distinct address operand in the stream, plus the count of guest-authored MME
/// instruction dwords. Class-gated per subchannel exactly as
/// [`decode_report_semaphore`] is, and for the same measured reason: this very pushbuffer
/// binds `0xc7c0` to subchannel 1 and `0xc7b5` to subchannel 4, and `SET_TEX_HEADER_POOL`'s
/// `0x1574` is a different method entirely on the copy engine.
///
/// # ⊘ What this is FOR, stated so it is not mistaken for a step toward execution
///
/// It answers one question and refuses the neighbouring one. The question is *"if the host
/// GR engine executed these bytes, what set of addresses would it dereference, and how many
/// of them can our address table bind?"* — i.e. **how large is the gap between where we are
/// and a VA space in which running guest GR work is containable**. It is print-only: it
/// decodes, it resolves, it reports. ⊘ It does not lower anything to a host verb, it
/// produces no plan, and it makes nothing executable.
///
/// Returns the **last** value seen for each operand — a pushbuffer may re-arm a register,
/// and the binding the engine would reach is the one written last. Deduplicated by method,
/// so the answer is a set of live registers rather than a transcript.
///
/// # ★★★★★ It decodes a REGISTER FILE, not a set of runs — and the first version did not
///
/// `[measured 2026-08-10, boot `w227b_184df5f_census`]` this decoder was first written to
/// read an `_A`/`_B` pair out of **one** method run's argument list. It reported
/// `operands=2 bound=2 unbound=0` on `cuCtxCreate`'s pushbuffer. The stream actually names
/// **five** addresses, and the guest writes them **two different ways in one pushbuffer**:
///
/// ```text
/// m[070] addr=0x0200 count=3 secop=1 args=[0x2, 0x0, 0xa8000]   <- ONE run, A/B/C
/// m[079] addr=0x1574 count=1 secop=1 args=[0x00000100]          <- SEPARATE methods,
/// m[080] addr=0x1578 count=1 secop=1 args=[0x00000000]             one per half
/// ```
///
/// ⊘ **And the failure was silent in the worst possible direction.** Three operands were not
/// reported at all, so the line read `unbound=0` — which is *"every address the guest names
/// already binds"*, the most encouraging answer available. `gr_execution_boundary.md` §5 had
/// named that outcome as arm B, *"strictly better than A and the first thing to distrust"*,
/// **before** the boot; distrusting it is the only reason the defect was found in the same
/// hour. ★ A falsifier that flags its own good news is worth writing.
///
/// ⇒ So the model is the hardware's: expand each run into per-register writes (honouring
/// `SECOP` — `INCREASING` walks the method address, `NON_INCREASING` repeats it), latch each
/// half independently, and emit an operand only when **both** halves have been written. That
/// is spelling-independent by construction rather than by having seen the spellings.
///
/// ⚠ [`decode_report_semaphore`] still reads its four words out of one run, because
/// `[measured]` the guest writes `SET_REPORT_SEMAPHORE` as a single `count=4` run on every
/// boot recorded. That is a narrower claim than this function's and is left narrow
/// deliberately: it is the decoder the completion observer depends on, it is measured on
/// exactly the streams it runs against, and widening it is a change to a load-bearing path
/// to fix a problem no boot has shown. ⊘ Recorded here so the divergence is a decision.
#[must_use]
pub fn decode_address_operands(methods: &[(u32, Vec<u32>)]) -> (Vec<AddressOperand>, usize) {
    let mut bound: [Option<u32>; 8] = [None; 8];
    // ★★★ THE REGISTER FILE, not the runs: `_A` half and `_B` half latched independently,
    // keyed by the `_A` offset. See this function's docs for the boot that made this the
    // shape.
    let mut half: BTreeMap<u32, (u32, Option<u32>, Option<u32>)> = BTreeMap::new();
    let mut mme_dwords = 0usize;
    for (header, args) in methods {
        let base = (header & 0x1fff) << 2;
        let subch = ((header >> 13) & 0x7) as usize;
        // ⚠ SECOP decides whether a multi-argument run WALKS the method address or repeats
        // it: `1` is INCREASING (arg `i` lands at `base + 4i`), `3` is NON_INCREASING (every
        // arg lands at `base`). ⊘ Treating an INCREASING run as one method is how a
        // three-argument `SET_VALID_SPAN_OVERFLOW_AREA_A` run gets read as an `_A` write of
        // three words instead of `_A`, `_B` and `_C`.
        let inc = (header >> 29) != 3;
        for (i, v) in args.iter().enumerate() {
            let addr = if inc {
                base + 4 * u32::try_from(i).unwrap_or(u32::MAX)
            } else {
                base
            };
            if addr == SET_OBJECT {
                bound[subch] = Some(*v & 0xffff);
                continue;
            }
            // ⊘ The class gate, on EVERY write. An unbound subchannel is not assumed to be
            // compute — "we did not see the SET_OBJECT" and "it is AMPERE_COMPUTE_B" are
            // different facts and only the second licenses a decode.
            if bound[subch] != Some(AMPERE_COMPUTE_B) {
                continue;
            }
            if addr == LOAD_MME_INSTRUCTION_RAM {
                mme_dwords += 1;
                continue;
            }
            if let Some((m, _, hi)) = COMPUTE_ADDRESS_OPERANDS
                .iter()
                .copied()
                .find(|(m, _, _)| *m == addr)
            {
                // ⚠ THE PER-ROW MASK. See [`COMPUTE_ADDRESS_OPERANDS`] for the boot this
                // would have mis-reported with one constant mask.
                let e = half.entry(m).or_insert((subch as u32, None, None));
                e.0 = subch as u32;
                e.1 = Some(*v & mask_to(hi));
            } else if let Some((m, _, _)) = COMPUTE_ADDRESS_OPERANDS
                .iter()
                .copied()
                .find(|(m, _, _)| *m + 4 == addr)
            {
                let e = half.entry(m).or_insert((subch as u32, None, None));
                e.0 = subch as u32;
                e.2 = Some(*v);
            }
        }
    }
    let live = half
        .into_iter()
        .filter_map(|(method, (subch, a, b))| {
            // ⚠ BOTH halves or nothing. A register with only its high word written names no
            // address, and inventing a zero for the low word would put an operand in the
            // census at a VA the guest never wrote — `an_in_annotation_is_not_a_transport_fact`.
            let (a, b) = (a?, b?);
            let (_, name, _) = COMPUTE_ADDRESS_OPERANDS
                .iter()
                .copied()
                .find(|(m, _, _)| *m == method)?;
            Some(AddressOperand {
                method,
                name,
                va: GpuVa((u64::from(a) << 32) | u64::from(b)),
                subch,
            })
        })
        .collect();
    (live, mme_dwords)
}

/// ★★★ **The page-table leaf an address falls in**, as the walk that resolved the address
/// reported it — the unit the second crossing backs.
///
/// # ⊘ Why the leaf and not the buffer
///
/// Nothing in a pushbuffer says how long a `SET_TEX_SAMPLER_POOL` is. The guest names a
/// base address and the engine reads as far as its own state says; the only extent anything
/// in this port can state from evidence is the one the guest's own page tables bind. So the
/// leaf is what gets backed, and a buffer longer than its first leaf is backed only as far
/// as that leaf goes. ⚠ **That is a real limit, not a rounding**, and it is carried in the
/// type so a reader of a census row can see the granularity rather than infer it.
///
/// The C reaches the same unit from the other direction: it coalesces adjacent leaves into
/// runs and then re-cuts the runs into 2 MiB-aligned chunks (`C: nvkvm_gpu_emul.c:8472-8477`),
/// one host object per chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbLeaf {
    /// The leaf's base guest VA.
    pub va: u64,
    /// The leaf's length — the walk's own page size for this entry, never an assumed one.
    pub len: u64,
    /// The framebuffer-physical address the leaf's base maps to.
    pub phys: u64,
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
        /// ★★★ The **leaf** this address falls in, as the SAME walk reported it. ⊘ Not a
        /// second lookup and not a rounding of `phys` by an assumed page size: the walk
        /// knows the leaf's granularity and this carries it rather than making a consumer
        /// guess one. See [`FbLeaf`].
        leaf: FbLeaf,
    },
    /// ★★★★★ **THE SECOND CROSSING, LANDED** — the framebuffer leaf carrying this
    /// address has a **real host vidmem object** mapped at the guest's own VA in the host
    /// VA space this proc's channels run in.
    ///
    /// ⊘ Read this variant for exactly what it says and nothing more:
    ///
    /// - **`Framebuffer` means NOT backed.** That is the whole informational content of
    ///   the split, and it is why the backed case is a distinct variant rather than a
    ///   field: an unarmed boot's rows must be visibly different from an armed boot's.
    /// - **The object is BLANK.** Nothing seeded it. The host GR engine could write it;
    ///   nothing has.
    /// - ⊘ **The guest's own CPU accesses at `phys` do NOT reach this object.** They
    ///   still go to the shell's fabricated aperture. Two memories, and closing that is
    ///   the CPU-view half this rung did not build.
    /// - ⊘ **Nothing executed.** A host object existing at an address is not the host
    ///   engine dereferencing it.
    HostBackedFb {
        /// The resolved physical address inside the emulated FB — unchanged, and still
        /// where the *guest's* accesses go.
        phys: u64,
        /// The leaf that was backed.
        leaf: FbLeaf,
        /// The host GPU VA the object was placed at. Equal to `leaf.va` by address
        /// identity, or it would not have been adopted.
        host_va: u64,
        /// The host memory object's raw handle, for the boot log.
        memory: u64,
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

    /// ★★★★★ **LEG 5's SOURCE — a read-only SNAPSHOT of every completion this guest has
    /// declared, so a consumer other than the observer can be given the addresses.**
    ///
    /// `[measured, w265, real GA106]` the host GPU faults `ENGINE CE3 HUBCLIENT_CE1 @
    /// 0x2_0440f000 ACCESS_TYPE_VIRT_WRITE`, eight times, and that page is **exactly** the page
    /// these declarations name: eight `SET_REPORT_SEMAPHORE` targets `0x20440ff80 …
    /// 0x20440fff0`, all `Site::GuestRam`. The addresses were already in this list; nothing
    /// could read them out of it.
    ///
    /// ⊘ **This adds a READER and no capability.** [`WatchList`] still cannot write, resolve or
    /// call the device — the whole point of [`GuestReader`] — and a snapshot of `(key, site)`
    /// pairs cannot change what any of those do. What the consumer does with an address is the
    /// consumer's to justify; see `SharedDoorbell::pin_completion_guest_ram`.
    ///
    /// ⊘ **Includes REPORTED watches.** A completion the observer has already ruled on is still
    /// a page the engine may write, and filtering on `reported` would make the source silently
    /// depend on the observer's deadline — an instrument gating a fix on its own verdict.
    #[must_use]
    pub fn declared_sites(&self) -> Vec<(WatchKey, Site)> {
        self.lock()
            .watches
            .iter()
            .map(|(k, w)| (*k, w.site.clone()))
            .collect()
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
                Site::Framebuffer { phys, .. } => {
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
                // ★★ **A BACKED leaf is no more observable from here than an unbacked
                // one**, and saying so is the point. The host object lives in the
                // isolate's handle namespace behind the worker pool; this thread has a
                // guest-RAM *reader* and nothing else. ⊘ Reporting `Observed` here
                // because a real host object exists would be the observer reporting on a
                // plane it never read — the exact substitution `Unobservable` exists to
                // refuse.
                Site::HostBackedFb { phys, host_va, .. } => {
                    if now >= w.deadline {
                        w.reported = true;
                        out.push(Verdict::Unobservable {
                            key: *key,
                            decl: w.decl,
                            why: format!(
                                "the VA resolves into the emulated framebuffer at \
                                 0x{phys:x}, whose leaf IS host-backed at host_va \
                                 0x{host_va:x} — but the observer thread has a guest-RAM \
                                 reader and no path to either plane, so nothing was read"
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

    /// Build the header word the way the wire does — `SECOP = INCREASING`, which is what
    /// `[measured]` every method in `cuCtxCreate`'s stream uses except the MME loads.
    fn hdr(addr: u32, subch: u32, count: u32) -> u32 {
        (addr >> 2) | (subch << 13) | (count << 16) | (1 << 29)
    }

    /// ★★★ `SECOP = NON_INCREASING` — every argument lands at the SAME method address.
    ///
    /// ⚠ `[measured, and the fixture was wrong before it]` the MME loads are the only
    /// non-increasing runs in `cuCtxCreate`'s pushbuffer (`hdr=0x600f2046`, `0x60182046`).
    /// This fixture originally built them with the increasing helper, and the decoder — being
    /// correct — walked the 15 arguments across fifteen consecutive registers, landing two of
    /// them on `SET_GLOBAL_RENDER_ENABLE_A/B` and inventing a sixth operand out of microcode.
    /// ⊘ **A fixture that is not the wire is a second implementation of the wire**, which is
    /// the failure this campaign has paid for more than any other — here it manufactured a
    /// *false positive* in the instrument that exists to count false negatives.
    fn hdr_ni(addr: u32, subch: u32, count: u32) -> u32 {
        (addr >> 2) | (subch << 13) | (count << 16) | (3 << 29)
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

    /// ★★★ THE MEASURED `cuCtxCreate` GR PUSHBUFFER, in the shape the address census must
    /// answer for. `[measured 2026-08-10, boot `w226b_534e1b3_cup2`, `run_w226b_*_qemu.log`
    /// lines 58-145 — 86 methods, 864 bytes, token 0x00000007]`. Only the rows that name an
    /// address or load MME microcode are reproduced; the 64 `SET_CWD_REF_COUNTER` writes and
    /// the cache invalidates are not, because the census says nothing about them.
    fn measured_ctxinit_full() -> Vec<(u32, Vec<u32>)> {
        vec![
            (hdr(SET_OBJECT, 1, 1), vec![AMPERE_COMPUTE_B]),
            // ★ SET_SHADER_SHARED_MEMORY_WINDOW_A/B as TWO SEPARATE single-argument
            // methods — the spelling the guest actually uses, and the one the first version
            // of this decoder could not see (see `decode_address_operands`).
            (hdr(0x02a0, 1, 1), vec![0x0000_7d1e]),
            (hdr(0x02a4, 1, 1), vec![0xe900_0000]),
            // SET_VALID_SPAN_OVERFLOW_AREA_A/B/C — the third word is a SIZE, not address.
            (
                hdr(0x0200, 1, 3),
                vec![0x0000_0002, 0x0000_0000, 0x000a_8000],
            ),
            // LOAD_MME_INSTRUCTION_RAM — 15 then 24 dwords of GUEST-AUTHORED MICROCODE.
            (hdr(0x0114, 1, 1), vec![0x0000_0000]),
            (
                hdr_ni(LOAD_MME_INSTRUCTION_RAM, 1, 15),
                vec![0x0014_0003; 15],
            ),
            (hdr(0x0114, 1, 1), vec![0x0000_000f]),
            (
                hdr_ni(LOAD_MME_INSTRUCTION_RAM, 1, 24),
                vec![0x0000_0003; 24],
            ),
            // ⊘ The copy engine binds subchannel 4 in the SAME stream — the collision the
            // class gate exists for, reproduced here so the census is asked about it too.
            (hdr(SET_OBJECT, 4, 1), vec![0xc7b5]),
            // SET_TEX_HEADER_POOL_A/B/C and SET_TEX_SAMPLER_POOL_A/B/C
            (
                hdr(0x1574, 1, 3),
                vec![0x0000_0100, 0x0000_0000, 0x000f_ffff],
            ),
            (
                hdr(0x155c, 1, 3),
                vec![0x0000_0100, 0x0200_0000, 0x0000_0000],
            ),
            (
                hdr(SET_REPORT_SEMAPHORE_A, 1, 4),
                vec![0x0000_0002, 0x0440_fff0, 0x0000_0001, 0x0000_0000],
            ),
        ]
    }

    #[test]
    fn the_census_names_every_address_the_measured_pushbuffer_dereferences() {
        let (ops, mme) = decode_address_operands(&measured_ctxinit_full());
        let by_name: std::collections::BTreeMap<_, _> =
            ops.iter().map(|o| (o.name, o.va.0)).collect();
        // ★★★★★ FIVE distinct 64-bit VAs, across THREE unrelated aperture bases. The
        // completion observer measured exactly ONE of them.
        assert_eq!(
            by_name.get("SET_REPORT_SEMAPHORE"),
            Some(&0x2_0440_fff0),
            "the one address `w226b` proved binds"
        );
        assert_eq!(
            by_name.get("SET_TEX_HEADER_POOL"),
            Some(&0x100_0000_0000),
            "★ a DIFFERENT aperture base — and with max index 0xfffff the engine reads 32 MiB \
             of whatever is mapped there"
        );
        assert_eq!(by_name.get("SET_TEX_SAMPLER_POOL"), Some(&0x100_0200_0000));
        assert_eq!(
            by_name.get("SET_VALID_SPAN_OVERFLOW_AREA"),
            Some(&0x2_0000_0000)
        );
        assert_eq!(
            by_name.get("SET_SHADER_SHARED_MEMORY_WINDOW"),
            Some(&0x7d1e_e900_0000),
            "★ ~137 TB — a base the guest chose, and nothing in this port has an opinion on it"
        );
        assert_eq!(ops.len(), 5, "five live address registers: {by_name:#?}");
        // ★★★★★ THE ROW THAT DECIDES THE SHAPE OF ANY ANSWER.
        assert_eq!(
            mme, 39,
            "★★★ 39 dwords of GUEST-AUTHORED MME microcode. The Macro Method Expander's \
             OUTPUT IS METHODS, so a method-level allowlist over this stream cannot be \
             sound — the VA space is the only boundary left. gr_execution_boundary.md §2."
        );
    }

    #[test]
    fn the_two_spellings_of_one_address_register_decode_identically() {
        // ★★★★★ THE DEFECT `w227b` MEASURED, pinned. The guest writes the SAME register two
        // ways in ONE pushbuffer: `SET_VALID_SPAN_OVERFLOW_AREA` as a single 3-argument
        // INCREASING run, and `SET_TEX_HEADER_POOL` as three separate 1-argument methods.
        // The first version of this decoder read only the run spelling and reported
        // `operands=2 bound=2 unbound=0` — three addresses missing, and the missing ones
        // made the answer read as "everything already binds".
        let run = vec![
            (hdr(SET_OBJECT, 1, 1), vec![AMPERE_COMPUTE_B]),
            (
                hdr(0x1574, 1, 3),
                vec![0x0000_0100, 0x0000_0000, 0x000f_ffff],
            ),
        ];
        let split = vec![
            (hdr(SET_OBJECT, 1, 1), vec![AMPERE_COMPUTE_B]),
            (hdr(0x1574, 1, 1), vec![0x0000_0100]),
            (hdr(0x1578, 1, 1), vec![0x0000_0000]),
            (hdr(0x157c, 1, 1), vec![0x000f_ffff]),
        ];
        assert_eq!(
            decode_address_operands(&run),
            decode_address_operands(&split),
            "★★★ the wire's two spellings of one register must be ONE fact. A decoder that \
             sees only one of them under-reports SILENTLY, and `unbound=0` on an \
             under-reported census is the most encouraging answer the instrument can give."
        );
        assert_eq!(decode_address_operands(&run).0.len(), 1);
        assert_eq!(
            decode_address_operands(&run).0[0].va,
            GpuVa(0x100_0000_0000)
        );
    }

    #[test]
    fn a_non_increasing_run_does_not_walk_into_the_next_register() {
        // ⚠ SECOP is load-bearing. 15 dwords of MME microcode written NON_INCREASING are 15
        // writes to 0x0118; written INCREASING they would be 15 CONSECUTIVE registers, two
        // of which are `SET_GLOBAL_RENDER_ENABLE_A/B` — i.e. microcode read as an address.
        let ni = vec![
            (hdr(SET_OBJECT, 1, 1), vec![AMPERE_COMPUTE_B]),
            (
                hdr_ni(LOAD_MME_INSTRUCTION_RAM, 1, 15),
                vec![0x0014_0003; 15],
            ),
        ];
        assert_eq!(
            decode_address_operands(&ni),
            (vec![], 15),
            "microcode is microcode, and none of it is an address"
        );
        // The mirror: the same words as an INCREASING run ARE fifteen register writes, and
        // the decoder must say so rather than special-casing the method.
        let inc = vec![
            (hdr(SET_OBJECT, 1, 1), vec![AMPERE_COMPUTE_B]),
            (hdr(LOAD_MME_INSTRUCTION_RAM, 1, 15), vec![0x0014_0003; 15]),
        ];
        let (ops, mme) = decode_address_operands(&inc);
        assert_eq!(mme, 1, "only the FIRST word landed on the MME port");
        assert!(
            ops.iter().any(|o| o.name == "SET_GLOBAL_RENDER_ENABLE"),
            "0x0118 + 4*6 = 0x0130; the walk is real and the decoder follows it: {ops:#?}"
        );
    }

    #[test]
    fn a_census_row_on_a_subchannel_bound_to_the_copy_engine_is_not_decoded() {
        // ⊘ The same collision the semaphore decoder is gated for, asked of the census:
        // `0x1574` is SET_TEX_HEADER_POOL on 0xc7c0 and something else entirely on 0xc7b5.
        let stream = vec![
            (hdr(SET_OBJECT, 4, 1), vec![0xc7b5]),
            (
                hdr(0x1574, 4, 3),
                vec![0x0000_0100, 0x0000_0000, 0x000f_ffff],
            ),
        ];
        let (ops, mme) = decode_address_operands(&stream);
        assert!(
            ops.is_empty(),
            "a copy-engine subchannel named no compute address: {ops:#?}"
        );
        assert_eq!(mme, 0);
    }

    #[test]
    fn an_unbound_subchannel_names_no_address_and_loads_no_microcode() {
        // ⊘ "we did not see the SET_OBJECT" and "it is AMPERE_COMPUTE_B" are different
        // facts, and only the second licenses the decode. An address census that guessed
        // would put a VA in the report that the guest may never have written.
        let stream = vec![
            (
                hdr(0x1574, 2, 3),
                vec![0x0000_0100, 0x0000_0000, 0x000f_ffff],
            ),
            (hdr_ni(LOAD_MME_INSTRUCTION_RAM, 2, 15), vec![0x1; 15]),
        ];
        assert_eq!(decode_address_operands(&stream), (vec![], 0));
    }

    #[test]
    fn a_half_written_address_pair_names_no_address_at_all() {
        // ⚠ `an_in_annotation_is_not_a_transport_fact`, one plane over: a run carrying only
        // the HIGH word named no address, and inventing a zero for the low word would put
        // an operand in the census at a VA the guest never wrote.
        let stream = vec![
            (hdr(SET_OBJECT, 1, 1), vec![AMPERE_COMPUTE_B]),
            (hdr(0x1574, 1, 1), vec![0x0000_0100]),
        ];
        assert_eq!(decode_address_operands(&stream).0, vec![]);
    }

    #[test]
    fn a_re_armed_address_register_reports_the_last_value() {
        // A pushbuffer may rewrite a register; the binding the engine would reach is the one
        // written last, and a census that reported both would be a transcript, not a state.
        let stream = vec![
            (hdr(SET_OBJECT, 1, 1), vec![AMPERE_COMPUTE_B]),
            (hdr(0x1574, 1, 2), vec![0x0000_0100, 0x0000_0000]),
            (hdr(0x1574, 1, 2), vec![0x0000_0007, 0xdead_0000]),
        ];
        let (ops, _) = decode_address_operands(&stream);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].va, GpuVa(0x7_dead_0000));
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

    /// ★★★★★ **LEG 5's SOURCE** — the eight `SET_REPORT_SEMAPHORE` targets `w265` measured
    /// are ONE page, and `declared_sites` is what lets a consumer see that.
    ///
    /// The addresses are the boot's own: `0x2_0440ff80 … 0x2_0440fff0` at a 16-byte stride,
    /// every one `site=GuestRam`, and the host GPU faulted `ACCESS_TYPE_VIRT_WRITE` at
    /// `0x2_0440f000` — the page they all fall in.
    #[test]
    fn the_eight_declared_semaphores_of_w265_are_one_page() {
        let list = WatchList::new();
        let t0 = Instant::now();
        // ⊘ The real addresses, in the order and at the stride the boot printed them.
        for (chan, va) in (0u32..8).map(|i| (i, 0x2_0440_fff0u64 - u64::from(i) * 0x10)) {
            let decl = DeclaredCompletion {
                va: crate::GpuVa(va),
                payload: 1,
                awaken: false,
                four_words: true,
                operation: 0,
                subch: 1,
                class_id: AMPERE_COMPUTE_B,
            };
            list.declare(
                WatchKey {
                    proc: ProcId(2),
                    chan: ChanId(chan),
                    va,
                },
                decl,
                Site::GuestRam {
                    gpa: 0x59a_0ff0 - u64::from(chan) * 0x10,
                },
                t0,
            );
        }
        let sites = list.declared_sites();
        assert_eq!(sites.len(), 8, "★ the snapshot lost a declaration");
        let pages: std::collections::BTreeSet<u64> =
            sites.iter().map(|(k, _)| k.va & !0xfffu64).collect();
        assert_eq!(
            pages.iter().copied().collect::<Vec<_>>(),
            vec![0x2_0440_f000],
            "★★★ the eight declarations are not one page — leg 5 would be eight pins, and the \
             address hardware named would not be among them"
        );
        assert!(
            sites
                .iter()
                .all(|(_, s)| matches!(s, Site::GuestRam { .. })),
            "⊘ a non-guest-RAM site here would be refused by the pin, correctly"
        );
    }

    /// ⊘ **A REPORTED watch is still a page the engine may write.** Filtering the snapshot on
    /// `reported` would make leg 5's source depend on the OBSERVER's deadline — an instrument
    /// gating a fix on its own verdict, which is the `a_diagnostic_gated_on_the_failure` shape.
    #[test]
    fn declared_sites_still_names_a_completion_the_observer_has_ruled_on() {
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
        assert!(
            matches!(v.as_slice(), [Verdict::NotObserved { .. }]),
            "got {v:?}"
        );
        assert_eq!(list.live(), 0, "the watch has been reported");
        assert_eq!(
            list.declared_sites().len(),
            1,
            "★ the reported watch vanished from the source; the page it names did not"
        );
    }
}
