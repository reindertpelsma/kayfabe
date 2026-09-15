//! # w747 — does `NV01_MEMORY_LIST_OBJECT` **alias** its parent's pages, or **copy** them?
//!
//! `docs/design/list_object_alias_probe.md` carries the pre-registered predictions; this is
//! the measurement. It is a **bare-metal** rung: a real GPU, no KVM, no guest, one process
//! that owns both of the RM clients it uses.
//!
//! ## Why the question is worth a box
//!
//! Leg B of the USERD design rests on one property. If a slice **aliases**, the scratchpad
//! can mint a handle naming one 4 KiB page of the single reserved store, the isolate names
//! that as `hUserdMemory[0]`, and **the channel is born on the isolate** — preserving its
//! privilege and `ProcessID` (constraints 26 and 30). If it **copies**, the guest rings a
//! USERD hardware never reads, and — this is the part that makes it a box rather than a
//! reading — **every counter we have would show green**: USERD is referenced by *physical
//! address* (`kernel_channel_gm107.c:328`, 4 KiB-attributed at `:689`) and never through
//! PT\*/PD\*, so nothing in our mapping plane would notice.
//!
//! ## ★★★ The discriminator is a write AFTER creation, and only that
//!
//! ⊘ Reading the parent's value through the slice immediately after creating it proves
//! **nothing**: that result is identical for an alias and for a copy-at-creation. The rung
//! therefore seeds, creates, and only then writes **through the parent** and asks the slice.
//!
//! ## What makes a green result mean anything
//!
//! 1. **The control arm.** A slice over a *different* page must not see the pattern — and it
//!    must be **positively identified** by its own sentinel rather than merely differing.
//!    ⚠ w744's own rung shipped a control whose "untouched" VA had been mapped by the probe
//!    itself; a control that reads zeros because its view is dead is indistinguishable from
//!    one that is genuinely isolated.
//! 2. ★★★ **The grader must be able to say COPY.** [`grade`] is fed a deliberately stale
//!    snapshot and an unrelated object, and must call both **COPY**. A test that can only
//!    print ALIAS is not a test.

use crate::rm::{HostRmBackend, RmConnection, ViewAccess};
use kayfabe_arch::ids::GpuId;
use kayfabe_isolate::{HostHandle, IsolateId, RmBackend, RmError};
use kayfabe_linux_raw::{
    Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion, release_fence,
};
use std::sync::Arc;

/// RM's page granularity — what a `LIST_OBJECT` page *number* counts in.
const PAGE: u64 = 0x1000;
/// The parent: four RM pages, so "a different page" is expressible and page 0 is not special.
const PARENT_BYTES: u64 = 4 * PAGE;
/// The page the slice names.
const N: u64 = 2;
/// The page the **control** slice names. ⊘ Must differ from [`N`]; asserted, not assumed.
const M: u64 = 0;

/// Seeded into page [`N`] before the slice exists.
const PAT_A: u32 = 0x747A_0001;
/// ★ Written through the **parent** after the slice exists. Seeing this is ALIAS.
const PAT_B: u32 = 0x747B_0002;
/// Written through the **slice**; seeing this through the parent is the reverse direction.
const PAT_C: u32 = 0x747C_0003;
/// The control page's sentinel — the value that makes the control *positively identified*.
const PAT_S: u32 = 0x7475_0004;
/// Written into a freshly re-allocated object after the parent is freed (constraint 31).
const PAT_D: u32 = 0x747D_0005;

/// Byte offset within a page that every pattern is written at. Non-zero on purpose: an
/// offset of 0 would let a probe that silently reads the wrong page's first word agree.
const OFF: u64 = 0x40;

/// What a read after the parent-side write means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Saw the value written through the parent AFTER the slice was created. One memory.
    Alias,
    /// Still the value from before. The slice holds its own bytes.
    Copy,
    /// Neither — the read is not interpretable and nothing may be concluded from it.
    Unknown,
}

impl Verdict {
    /// The word this verdict prints as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Alias => "ALIAS",
            Verdict::Copy => "COPY",
            Verdict::Unknown => "UNKNOWN",
        }
    }
}

/// ★★★ **The one grader, used by every arm including the known-positives.**
///
/// `before` is what the slice held before the parent-side write; `after` is what it holds
/// now; `written` is what was written through the parent.
///
/// ⊘ It is a free function taking three numbers precisely so the known-positives can drive
/// it with values no GPU produced. A verdict function reachable only through the live path
/// can never be shown to be capable of the unfavourable answer.
#[must_use]
pub fn grade(before: u32, after: u32, written: u32) -> Verdict {
    if after == written {
        Verdict::Alias
    } else if after == before {
        Verdict::Copy
    } else {
        Verdict::Unknown
    }
}

/// Decode an RM status against
/// `research_clones/ogkm-580.159.04/src/common/sdk/nvidia/inc/nvstatuscodes.h`.
///
/// ⊘⊘ **This table was WRONG on its first cut and the error was the dangerous kind.** It
/// carried `0x1f` as `NV_ERR_INSUFFICIENT_PERMISSIONS` — the code this rung's own
/// pre-registration named as the refuter for the privilege prediction. `0x1f` is
/// **`NV_ERR_INVALID_ARGUMENT`**; permissions are **`0x1b`** (`nvstatuscodes.h:56`). A rung
/// that decoded its refusal with that table would have reported *"refused for permissions"*
/// about a malformed parameter block, and the finding would have been a fact about my
/// encoder. ⇒ every row below is transcribed from the header, and `unknown` says so.
///
/// ⚠ Two of the values that reach here are **NOT RM statuses at all**:
/// [`kayfabe_isolate_host::rm::NOT_ON_THIS_RUNG`](crate::rm::NOT_ON_THIS_RUNG) and
/// [`BAD_ENCODE`](crate::rm::BAD_ENCODE) are this crate's own private codes riding the same
/// `RmError::Other`. They are named, because *"the driver refused"* and *"we could not build
/// the call"* are opposite findings arriving as the same word.
#[must_use]
pub fn status_name(code: u32) -> &'static str {
    match code {
        0x0000 => "NV_OK",
        0x0002 => "NV_ERR_BUFFER_TOO_SMALL",
        0x0019 => "NV_ERR_INSERT_DUPLICATE_NAME",
        0x001a => "NV_ERR_INSUFFICIENT_RESOURCES",
        0x001b => "NV_ERR_INSUFFICIENT_PERMISSIONS",
        0x001f => "NV_ERR_INVALID_ARGUMENT",
        0x0022 => "NV_ERR_INVALID_CLASS",
        0x0023 => "NV_ERR_INVALID_CLIENT",
        0x0025 => "NV_ERR_INVALID_DATA",
        0x0026 => "NV_ERR_INVALID_DEVICE",
        0x0029 => "NV_ERR_INVALID_FLAGS",
        0x0031 => "NV_ERR_INVALID_OBJECT",
        0x0032 => "NV_ERR_INVALID_OBJECT_BUFFER",
        0x0033 => "NV_ERR_INVALID_OBJECT_HANDLE",
        0x0036 => "NV_ERR_INVALID_OBJECT_PARENT",
        0x0037 => "NV_ERR_INVALID_OFFSET",
        0x0038 => "NV_ERR_INVALID_OPERATION",
        0x003a => "NV_ERR_INVALID_PARAM_STRUCT",
        0x003b => "NV_ERR_INVALID_PARAMETER",
        0x003d => "NV_ERR_INVALID_POINTER",
        0x0040 => "NV_ERR_INVALID_STATE",
        0x0051 => "NV_ERR_NO_MEMORY",
        0x0056 => "NV_ERR_NOT_SUPPORTED",
        0x0057 => "NV_ERR_OBJECT_NOT_FOUND",
        0x0058 => "NV_ERR_OBJECT_TYPE_MISMATCH",
        0x0059 => "NV_ERR_OPERATING_SYSTEM",
        0x005b => "NV_ERR_OUT_OF_RANGE",
        0x005f => "NV_ERR_PROTECTION_FAULT",
        0x006e => "NV_ERR_RESOURCE_LOST",
        0xffff => "NV_ERR_GENERIC",
        crate::rm::NOT_ON_THIS_RUNG => {
            "⚠ NOT AN RM STATUS — kayfabe's own NOT_ON_THIS_RUNG (an encode/ioctl-build failure \
             on our side, the driver was never asked)"
        }
        crate::rm::BAD_ENCODE => {
            "⚠ NOT AN RM STATUS — kayfabe's own BAD_ENCODE (we could not build or decode the \
             parameter block; the driver's answer is not in play)"
        }
        _ => "(not in this rung's decode table — look it up in nvstatuscodes.h before quoting it)",
    }
}

/// Render an [`RmError`] with its status decoded, so no line of this rung's output carries a
/// bare number.
///
/// ⚠ `status_check` folds three RM codes into named variants before this crate ever sees
/// them — `0x1b` becomes [`RmError::InsufficientPermissions`] and `0x1a`/`0x51` both become
/// [`RmError::NoMemory`]. The *original* code is therefore printed here from the variant, or
/// the two `NoMemory` sources would be indistinguishable in the log.
fn show(e: &RmError) -> String {
    match e {
        RmError::Other(code) => format!("status {code:#06x} {}", status_name(*code)),
        RmError::InsufficientPermissions => {
            format!("status 0x001b {}", status_name(0x001b))
        }
        RmError::NoMemory => {
            "status 0x001a NV_ERR_INSUFFICIENT_RESOURCES or 0x0051 NV_ERR_NO_MEMORY (status_check \
             folds both into one variant)"
                .to_string()
        }
        RmError::BadHandle(h) => format!("BadHandle({h:?}) — OUR namespace check, not RM's"),
        other => format!("{other:?}"),
    }
}

/// One armed CPU view, kept alive together with the token that releases its BAR1 aperture.
struct View {
    token: u64,
    region: VolatileRegion,
}

impl View {
    /// Arm and map `[0, len)` of `mem`.
    fn open(rm: &mut HostRmBackend, mem: HostHandle, len: u64) -> Result<View, String> {
        let v = rm
            .export_device_view(mem, 0, len, ViewAccess::ReadWrite)
            .map_err(|e| format!("arm: {}", show(&e)))?;
        let fd = rm
            .exports()
            .lend(v.token)
            .map_err(|e| format!("lend: {e:?}"))?;
        let region = VolatileRegion::map(
            Backing::DeviceFile {
                fd: std::os::fd::AsFd::as_fd(&fd),
            },
            v.mmap_len,
            CachePolicy::WriteCombining,
            HostPageSize::query(),
        )
        .map_err(|e| format!("mmap: {e}"))?;
        Ok(View {
            token: v.token,
            region,
        })
    }

    /// Read the probe word at `page * PAGE + OFF`.
    fn get(&self, page: u64) -> u32 {
        self.region
            .load_u32(HostOffset::new(page * PAGE + OFF))
            .unwrap_or(u32::MAX)
    }

    /// Write the probe word at `page * PAGE + OFF`, then fence.
    fn put(&self, page: u64, v: u32) -> bool {
        let ok = self
            .region
            .store_u32(HostOffset::new(page * PAGE + OFF), v)
            .is_ok();
        release_fence();
        ok
    }
}

/// Print one `GET_SURFACE_PHYS_ATTR` row, or say why there is none.
fn phys(rm: &mut HostRmBackend, what: &str, mem: HostHandle, offset: u64) -> Option<u64> {
    match rm.surface_phys_attr(mem, offset) {
        Ok(a) => {
            let ap = match a.mem_aperture {
                0 => "VIDMEM",
                1 => "SYSMEM",
                _ => "(unknown aperture)",
            };
            println!(
                "info  W747 phys {what:<18} = {:#018x} aperture {} ({ap}) kind {:#x} contig {:#x}",
                a.mem_offset, a.mem_aperture, a.mem_format, a.contig_segment_size
            );
            Some(a.mem_offset)
        }
        Err(e) => {
            println!(
                "⊘     W747 phys {what:<18} = REFUSED {} — the header calls this control \
                 MODS-only; a refusal is a measurement of that claim and NOT a result about \
                 aliasing",
                show(&e)
            );
            None
        }
    }
}

/// Allocate the slice, trying the 4 KiB page attribute first and the RM default second.
///
/// ⊘ **Both attempts are reported.** *"Do not stop at the first refusal"* — a refusal of the
/// explicit page-size attribute and a refusal of the class itself are different findings that
/// arrive as the same word if only one is tried.
fn mint_slice(
    rm: &mut HostRmBackend,
    what: &str,
    parent: HostHandle,
    page: u64,
    foreign: Option<(u32, u32)>,
) -> Option<HostHandle> {
    const ATTR_4KB: u32 = kayfabe_abi::submit::ATTR_VIDMEM_PAGE_4KB;
    for (label, attr) in [("PAGE_SIZE_4KB", ATTR_4KB), ("PAGE_SIZE_DEFAULT", 0u32)] {
        match rm.alloc_list_object_page(parent, page, foreign, attr) {
            Ok(h) => {
                println!(
                    "ok    W747 slice {what:<17} = page {page} attr {label} accepted, handle \
                     {:#010x}",
                    h.raw()
                );
                return Some(h);
            }
            Err(e) => println!(
                "⊘     W747 slice {what:<17} = page {page} attr {label} REFUSED {}",
                show(&e)
            ),
        }
    }
    None
}

/// ★★★ The rung. Returns `true` only if the verdict is ALIAS **and** every known-positive
/// held; a `false` here is a full deliverable, not a failure of the run.
///
/// # Panics
/// Never deliberately; every RM refusal is reported and the rung continues past it wherever
/// a later row is still interpretable.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn run(rm: &mut HostRmBackend, gpu: u32) -> bool {
    const { assert!(N != M, "the control must name a DIFFERENT page") };
    println!(
        "==    W747 list-object-alias = gpu {gpu}, euid {}, parent {PARENT_BYTES:#x} bytes, \
         slice page {N}, control page {M}",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  W747 the bar        = a write through the PARENT after the slice exists must be \
         visible through the SLICE (ALIAS); still reading the pre-write value is COPY and the \
         USERD design is dead however green the read-at-creation looked"
    );

    // ── KNOWN-POSITIVES FIRST. ⚠ Before any GPU work, because a grader that cannot say
    // COPY makes every line after it unreadable, and discovering that at the end means the
    // box was spent on an instrument nobody had shown could fail.
    let kp_stale = grade(PAT_A, PAT_A, PAT_B);
    let kp_alias = grade(PAT_A, PAT_B, PAT_B);
    let kp_junk = grade(PAT_A, 0xDEAD_DEAD, PAT_B);
    println!(
        "{}  W747 KP grader      = a stale snapshot grades {} · a live alias grades {} · an \
         uninterpretable read grades {} — the grader can reach all three answers",
        if kp_stale == Verdict::Copy
            && kp_alias == Verdict::Alias
            && kp_junk == Verdict::Unknown
        {
            "ok  "
        } else {
            "FAIL"
        },
        kp_stale.as_str(),
        kp_alias.as_str(),
        kp_junk.as_str()
    );
    let grader_ok =
        kp_stale == Verdict::Copy && kp_alias == Verdict::Alias && kp_junk == Verdict::Unknown;

    // ── 1. The parent, and a CPU view over the whole of it.
    let parent = match rm.alloc_vidmem(PARENT_BYTES) {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  W747 parent         = {} ⊘ the premise never existed", show(&e));
            println!("W747_VERDICT=NOTRUN");
            return false;
        }
    };
    let pv = match View::open(rm, parent, PARENT_BYTES) {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL  W747 parent view    = {e} ⊘ nothing below can run");
            println!("W747_VERDICT=NOTRUN");
            return false;
        }
    };

    // ── 2. Seed. Page N gets A; page M gets the control's sentinel S.
    if !pv.put(N, PAT_A) || !pv.put(M, PAT_S) {
        println!("FAIL  W747 seed           = a store through the parent view was refused");
        println!("W747_VERDICT=NOTRUN");
        return false;
    }
    if pv.get(N) != PAT_A || pv.get(M) != PAT_S {
        println!(
            "FAIL  W747 seed readback  = page {N} {:#010x} page {M} {:#010x} — a view that does \
             not hold its own stores decides nothing",
            pv.get(N),
            pv.get(M)
        );
        println!("W747_VERDICT=NOTRUN");
        return false;
    }
    println!(
        "ok    W747 seed           = page {N} = {PAT_A:#010x} (A), page {M} = {PAT_S:#010x} (S), \
         both read back through the parent"
    );

    // ── 3. The slice, and the control slice.
    let Some(slice) = mint_slice(rm, "L_N", parent, N, None) else {
        println!(
            "FAIL  W747 slice          = ⊘ BOTH attribute arms refused. Decode the status above: \
             0x1f is the class's RS_FLAGS_ALLOC_PRIVILEGED (root-only) and 0x56 is \
             memlistConstruct's GSP-client gate — an ENVIRONMENT result, not RM's ruling on \
             aliasing"
        );
        println!("W747_VERDICT=NOTRUN");
        return false;
    };
    let control = mint_slice(rm, "L_M (control)", parent, M, None);

    // ── 4. Read through the slice. ⊘ NECESSARY, NOT SUFFICIENT — identical for an alias and
    // for a copy-at-creation, which is the whole reason step 5 exists.
    let sv = match View::open(rm, slice, PAGE) {
        Ok(v) => v,
        Err(e) => {
            println!(
                "FAIL  W747 slice view     = {e} ⊘ the slice exists but cannot be read, so the \
                 discriminator cannot be applied"
            );
            println!("W747_VERDICT=NOTRUN");
            return false;
        }
    };
    let slice_before = sv.get(0);
    println!(
        "{}  W747 step 4         = slice reads {slice_before:#010x}, expected A {PAT_A:#010x} \
         — ⊘ necessary, NOT sufficient: a copy-at-creation gives exactly this",
        if slice_before == PAT_A { "ok  " } else { "??  " }
    );
    let cv = control.and_then(|c| match View::open(rm, c, PAGE) {
        Ok(v) => Some((c, v)),
        Err(e) => {
            println!("⊘     W747 control view   = {e}");
            None
        }
    });
    let control_before = cv.as_ref().map(|(_, v)| v.get(0));
    match control_before {
        Some(got) => println!(
            "{}  W747 control id     = control slice reads {got:#010x}, expected S \
             {PAT_S:#010x} — the control is POSITIVELY IDENTIFIED as page {M}, not merely \
             'not B'",
            if got == PAT_S { "ok  " } else { "FAIL" }
        ),
        None => println!(
            "⊘     W747 control id     = no control slice; every alias claim below is \
             UNCONTROLLED and must be read as such"
        ),
    }

    // ── 5. ★★★ THE DISCRIMINATOR. Write B through the PARENT; ask the slice.
    if !pv.put(N, PAT_B) {
        println!("FAIL  W747 step 5 write   = the parent-side store was refused");
        println!("W747_VERDICT=NOTRUN");
        return false;
    }
    release_fence();
    let slice_after = sv.get(0);
    let verdict = grade(slice_before, slice_after, PAT_B);
    println!(
        "★★★★★ W747 STEP 5 VERDICT = {} — wrote B {PAT_B:#010x} through the PARENT's page {N}; \
         the SLICE now reads {slice_after:#010x} (it read {slice_before:#010x} before)",
        verdict.as_str()
    );

    // ── 6. The reverse direction. One memory means both ways.
    let rev = if sv.put(0, PAT_C) {
        release_fence();
        let back = pv.get(N);
        let v = grade(PAT_B, back, PAT_C);
        println!(
            "{}  W747 step 6 reverse = wrote C {PAT_C:#010x} through the SLICE; the PARENT's \
             page {N} reads {back:#010x} ⇒ {}",
            if v == Verdict::Alias { "ok  " } else { "FAIL" },
            v.as_str()
        );
        v
    } else {
        println!("⊘     W747 step 6 reverse = a store through the slice view was refused");
        Verdict::Unknown
    };

    // ── 7. ⚠ THE CONTROL, after everything. It must STILL read S.
    let control_ok = match (&cv, control_before) {
        (Some((_, v)), Some(before)) => {
            let now = v.get(0);
            let held = before == PAT_S && now == PAT_S;
            println!(
                "{}  W747 step 7 control = the control slice over page {M} reads {now:#010x}; \
                 it must still be S {PAT_S:#010x} — not B, and NOT ZERO (a dead view reads \
                 zero and would look isolated)",
                if held { "ok  " } else { "FAIL" }
            );
            held
        }
        _ => false,
    };
    // ★ And the control's own alias check, so the control is not merely inert: writing a
    // fresh value into page M through the parent must reach it too.
    if let Some((_, v)) = &cv {
        let before = v.get(0);
        if pv.put(M, PAT_D) {
            release_fence();
            let now = v.get(0);
            println!(
                "{}  W747 control alias  = page {M} written {PAT_D:#010x} through the parent; \
                 the control slice reads {now:#010x} ⇒ {} — the control is a LIVE view of its \
                 own page, not an inert one that agrees by being broken",
                if grade(before, now, PAT_D) == Verdict::Alias {
                    "ok  "
                } else {
                    "??  "
                },
                grade(before, now, PAT_D).as_str()
            );
        }
    }

    // ── 8. The physical addresses, side by side. A direct equality, not an inference.
    let p_parent = phys(rm, "parent@N", parent, N * PAGE);
    let p_slice = phys(rm, "slice@0", slice, 0);
    match (p_parent, p_slice) {
        (Some(a), Some(b)) => println!(
            "{}  W747 step 8 phys    = parent page {N} at {a:#018x}, slice at {b:#018x} — {}",
            if a == b { "★★★★★" } else { "FAIL " },
            if a == b {
                "THE SAME PHYSICAL ADDRESS"
            } else {
                "DIFFERENT physical addresses; whatever the CPU views showed, these are not one page"
            }
        ),
        _ => println!(
            "⊘     W747 step 8 phys    = not both sides answered; the equality is UNMEASURED and \
             the verdict rests on the CPU views alone"
        ),
    }

    // ── 9. KNOWN-POSITIVE: an INDEPENDENT object graded by the same function.
    // ⊘ Not decoration. If `grade` were reading something other than what it claims, this
    // row is where it shows: an unrelated allocation seeded with A must grade COPY.
    let indep_ok = match rm.alloc_vidmem(PAGE) {
        Ok(other) => match View::open(rm, other, PAGE) {
            Ok(ov) => {
                ov.put(0, PAT_A);
                release_fence();
                let before = ov.get(0);
                // The parent-side write already happened; re-issue it so the timing matches.
                pv.put(N, PAT_B);
                release_fence();
                let v = grade(before, ov.get(0), PAT_B);
                println!(
                    "{}  W747 KP independent = an unrelated vidmem object seeded with A grades \
                     {} after the same parent-side write — the grader is not answering ALIAS \
                     for everything",
                    if v == Verdict::Copy { "ok  " } else { "FAIL" },
                    v.as_str()
                );
                let ok = v == Verdict::Copy;
                let _ = rm.release_device_view(ov.token);
                drop(ov);
                let _ = rm.free(other);
                ok
            }
            Err(e) => {
                println!("⊘     W747 KP independent = view refused: {e}");
                false
            }
        },
        Err(e) => {
            println!("⊘     W747 KP independent = alloc refused: {}", show(&e));
            false
        }
    };

    // ── 10. CROSS-CLIENT. Two rows, both reported verbatim whatever they say.
    cross_client(rm, gpu, parent, slice);

    // ── 11. ⚠ LIFETIME — constraint 31's mechanism, measured.
    lifetime(rm, parent, pv, slice, &sv);

    // ── Teardown of what is left. ⊘ The slice's own view is released explicitly: a leaked
    // BAR1 aperture is invisible until an unrelated arm is refused `0x51` for no reason it
    // can name (`release_device_view`'s own docs).
    let _ = rm.release_device_view(sv.token);
    if let Some((c, v)) = cv {
        let _ = rm.release_device_view(v.token);
        drop(v);
        let _ = rm.free(c);
    }

    let pass = grader_ok
        && verdict == Verdict::Alias
        && rev == Verdict::Alias
        && control_ok
        && indep_ok
        && slice_before == PAT_A;
    println!("W747_VERDICT={}", verdict.as_str());
    println!("W747_REVERSE={}", rev.as_str());
    println!("W747_CONTROL={}", if control_ok { "HELD" } else { "BROKEN" });
    println!("W747_KNOWN_POSITIVES={}", if grader_ok && indep_ok { "HELD" } else { "BROKEN" });
    println!("W747_RESULT={}", if pass { "PASS" } else { "FAIL" });
    pass
}

/// The two cross-client rows: can a slice be created naming a **foreign** client's parent,
/// and can a slice be **duped** into another client?
///
/// ⊘ Both are reported with verbatim status codes whatever they say — a refusal here is a
/// finding about how far the design can reach, not a failure of the rung.
fn cross_client(rm: &mut HostRmBackend, gpu: u32, parent: HostHandle, slice: HostHandle) {
    let Ok(dev) = DevDir::open(c"/dev") else {
        println!("⊘     W747 cross-client   = /dev could not be opened; the rows never ran");
        return;
    };
    let conn_b = match RmConnection::open(&dev, GpuId(gpu), kayfabe_chips::pinned_host_classes()) {
        Ok(c) => c,
        Err(e) => {
            println!(
                "⊘     W747 cross-client   = a SECOND RM client could not be opened ({e}); the \
                 rows never ran, which is NOT a cross-client result"
            );
            return;
        }
    };
    let client_a = rm.host_client();
    let device_a = rm.host_device();
    let client_b = conn_b.client();
    println!("info  W747 hClient A/B    = {client_a:#010x} / {client_b:#010x}");
    if client_a == client_b {
        println!(
            "⊘     W747 cross-client   = the two connections returned the SAME hClient; nothing \
             below would be a cross-client statement"
        );
        return;
    }
    let mut rm_b = HostRmBackend::new(
        IsolateId::new(1, GpuId(gpu)),
        Arc::new(conn_b),
        Arc::new(crate::ChildExports::new()),
    );

    // Row 1 — client B mints a slice naming client A's object, through `hClient`/`hParent`.
    // ⚠ The parent handle belongs to A's namespace, so it is passed as a RAW value that B
    // names; `mint_slice` would refuse it as a foreign handle, and rightly.
    match rm_b.alloc_list_object_page_raw(
        parent.raw() as u32,
        N,
        Some((client_a, device_a)),
        kayfabe_abi::submit::ATTR_VIDMEM_PAGE_4KB,
    ) {
        Ok(h) => {
            println!(
                "★★★   W747 xclient mint   = client B minted a slice over client A's object \
                 (hClient={client_a:#010x} hParent={device_a:#010x}) — handle {:#010x}, \
                 status {:#06x} {}",
                h.raw(),
                0,
                status_name(0)
            );
            let _ = rm_b.free(h);
        }
        Err(e) => println!(
            "⊘     W747 xclient mint   = REFUSED {} — a slice naming a foreign client's parent \
             is not available to us",
            show(&e)
        ),
    }

    // Row 2 — the dup. `memlistCanCopy_IMPL` returns NV_TRUE, so the reading is that this
    // succeeds. ⚠ Constraint 30: a dup that SUCCEEDS says nothing about what it carries.
    match rm_b.dup_object_for_probe(client_a, slice.raw() as u32) {
        Ok(h) => {
            println!(
                "★★★   W747 xclient dup    = client B duped A's slice, handle {:#010x}, status \
                 {:#06x} {} ⚠ constraint 30: a successful dup says NOTHING about what the duped \
                 object carries",
                h.raw(),
                0,
                status_name(0)
            );
            let _ = rm_b.free(h);
        }
        Err(e) => println!("⊘     W747 xclient dup    = REFUSED {}", show(&e)),
    }
}

/// ⚠ **The owner's question (4).** With a live slice outstanding, free the parent and read
/// through the slice.
///
/// Three outcomes and the third is the dangerous one: RM refuses the free · the read faults
/// or is refused · **the slice silently serves stale physical memory**. ★ The third is why
/// constraint 31 exists, and nothing in our code would notice it.
fn lifetime(
    rm: &mut HostRmBackend,
    parent: HostHandle,
    pv: View,
    slice: HostHandle,
    sv: &View,
) {
    // The parent's CPU view goes first: a free refused because a mapping is outstanding
    // would be a fact about the mapping, not about the slice's claim on the pages.
    let _ = rm.release_device_view(pv.token);
    drop(pv);
    let before = sv.get(0);
    match rm.free(parent) {
        Ok(()) => println!(
            "★★★   W747 lifetime free  = the PARENT was freed with a live slice outstanding — \
             RM did NOT refuse"
        ),
        Err(e) => {
            println!(
                "★★★   W747 lifetime free  = RM REFUSED to free the parent: {} ⇒ the slice holds \
                 a reference and constraint 31's window may be narrower than feared",
                show(&e)
            );
            return;
        }
    }
    let after_free = sv.get(0);
    println!(
        "info  W747 lifetime read  = through the slice: {before:#010x} before the free, \
         {after_free:#010x} after"
    );

    // ★★★ The sharpest form of the question: re-allocate and write something NEW. If the
    // slice shows it, the slice is a live window onto memory its parent no longer owns —
    // which is precisely *"a page now owned by guest process B reachable through machinery
    // minted for guest process A"*.
    match rm.alloc_vidmem(PARENT_BYTES) {
        Ok(fresh) => {
            match View::open(rm, fresh, PARENT_BYTES) {
                Ok(fv) => {
                    fv.put(N, PAT_D);
                    release_fence();
                    let seen = sv.get(0);
                    if seen == PAT_D {
                        println!(
                            "⊘⊘⊘   W747 lifetime stale = THE SLICE SERVES THE NEW OWNER'S BYTES. \
                             A fresh allocation wrote {PAT_D:#010x} at page {N} and the slice — \
                             minted over a parent that no longer exists — reads {seen:#010x}. \
                             ⇒ constraint 31's window is REAL and silent: no refusal, no fault, \
                             no counter"
                        );
                    } else {
                        println!(
                            "info  W747 lifetime stale = the slice reads {seen:#010x}, not the \
                             new owner's {PAT_D:#010x}. ⊘ This does NOT prove containment — the \
                             fresh allocation may simply have landed elsewhere in the heap; it \
                             is one draw, not a bound"
                        );
                    }
                    let _ = rm.release_device_view(fv.token);
                    drop(fv);
                }
                Err(e) => println!("⊘     W747 lifetime stale = fresh view refused: {e}"),
            }
            let _ = rm.free(fresh);
        }
        Err(e) => println!("⊘     W747 lifetime stale = fresh alloc refused: {}", show(&e)),
    }
    let _ = rm.free(slice);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★★★ **The grader can say COPY, and the assertion is what proves it.** A test that
    /// only ever drives the favourable path cannot distinguish a working verdict function
    /// from `|_, _, _| Verdict::Alias`.
    #[test]
    fn the_grader_reaches_all_three_answers() {
        assert_eq!(grade(PAT_A, PAT_B, PAT_B), Verdict::Alias);
        assert_eq!(grade(PAT_A, PAT_A, PAT_B), Verdict::Copy);
        assert_eq!(grade(PAT_A, 0xDEAD_DEAD, PAT_B), Verdict::Unknown);
    }

    /// ⊘ The degenerate case that would make every run read ALIAS: if the "before" value and
    /// the written value were ever equal, a copy would be indistinguishable from an alias.
    /// The patterns are asserted distinct here rather than left to whoever edits them.
    #[test]
    fn the_patterns_cannot_make_a_copy_look_like_an_alias() {
        let all = [PAT_A, PAT_B, PAT_C, PAT_S, PAT_D, 0u32];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two patterns collide; the discriminator is degenerate");
            }
        }
        assert_ne!(N, M, "the control must name a different page");
        assert!(N * PAGE + OFF < PARENT_BYTES);
        assert!(M * PAGE + OFF < PARENT_BYTES);
        assert!(OFF < PAGE, "the probe word must be inside its own page");
    }

    /// Every status this rung can print decodes to a name, and an unknown one says so
    /// instead of guessing — the w744 lesson, held by a test.
    #[test]
    fn statuses_decode_and_unknown_ones_admit_it() {
        assert_eq!(status_name(0x0000), "NV_OK");
        // ⊘ The correction this table exists for: 0x1f is INVALID_ARGUMENT, and permissions
        // are 0x1b. Asserted in both directions so a future edit cannot quietly swap them.
        assert_eq!(status_name(0x001f), "NV_ERR_INVALID_ARGUMENT");
        assert_eq!(status_name(0x001b), "NV_ERR_INSUFFICIENT_PERMISSIONS");
        assert!(status_name(crate::rm::BAD_ENCODE).contains("NOT AN RM STATUS"));
        assert_eq!(status_name(0x0056), "NV_ERR_NOT_SUPPORTED");
        // ⊘ 0xFFFF IS in the table (NV_ERR_GENERIC) — the first draft of this assertion used
        // it as the "unknown" case and failed, which is the cheapest possible instance of
        // this rung's own lesson: a decode table you did not read is a decode table you are
        // guessing about. A genuinely absent code is used instead.
        assert_eq!(status_name(0xFFFF), "NV_ERR_GENERIC");
        assert!(status_name(0x0DED).contains("nvstatuscodes.h"));
    }
}
