//! ★★★ **E10c — the SHELL's CPU copy-engine executor** (`execution_plane_increments.md`
//! §14, the CPU branch of the CE decision tree).
//!
//! A [`CeExecutor::Ours`] sub-copy names *fabricated* space no real engine can be pointed
//! at — the emulated framebuffer, or a physical-mode operand in guest RAM. The isolate
//! **cannot** run such a copy and refuses it (`kayfabe_isolate_host::rm::ce_copy` →
//! `NOT_ON_THIS_RUNG`): it is a separate sandboxed process that deliberately holds neither
//! the emulated framebuffer nor guest RAM. That is the security posture working, not a gap.
//! So the copy is executed **here, in the shell**, against the two memory planes the shell
//! owns — the [`FbStore`] and the [`Vmm`] — with the store chosen by the E10b residency
//! answer ([`CpuPlane`]) each operand carries, never by the address value.
//!
//! ⊘ **Why this is SAFE code, not a `*_unsafe.rs` file** (`l1_os_shell.md` §4.2.1.1). This
//! module computes guest-physical and framebuffer-physical addresses by offset arithmetic,
//! but it never dereferences a raw pointer and never holds a `&[u8]` over live guest memory:
//! every access is a **bounded copy into an owned buffer through the [`Vmm`]/[`FbStore`]
//! trait, which re-validates the address against bounds the audited raw layer owns.** That is
//! precisely §4.2.1.1's sanctioned pattern — *"the first response to soundness-critical safe
//! code is to make the raw side re-validate, not to rename the file"* — so the unsound
//! surface stays in the audited crates (`kayfabe-linux-raw`/`kayfabe-qemu-raw`, the real
//! `Vmm`), and this executor is ordinary safe code on top of it (the §4.1 containment gate).
//!
//! ★ **This is forwarding, not forgery** (`mode2_forwarding_model.md`: *correctness =
//! observable end-states only*): a different executor producing the TRUE end-state is
//! forwarding. What is forbidden — and what this module never does — is signalling a copy
//! that did not move the bytes, or landing them where the guest cannot read them. The bytes
//! move first; E10d signals only after.

use kayfabe_abi::submit;
use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::{Backing, CeCompletion, CeSemStructure, CpuOperand, CpuPlane, PlaneAddr};
use kayfabe_device::{FbRefused, FbStore, FbWriter};
use kayfabe_fwd::{COMPLETION_VECTOR, CeSpan, FwdFault};
use kayfabe_isolate::{CeExecutor, CeSource};
use kayfabe_vmm::{Vmm, VmmError};

/// The bounded staging-buffer size for one copy step. A guest controls a copy's length, so
/// the executor never allocates it whole — it streams the copy through a fixed buffer, so a
/// hostile multi-gigabyte request costs a fixed 64 KiB of host memory, not its own size
/// (boundary-1 posture, the same reason [`kayfabe_fwd::read_pushbuffer`] clamps).
const CHUNK: usize = 64 * 1024;

/// Map a framebuffer-store refusal to the named CPU-CE fault.
fn fb_fault(e: FbRefused) -> FwdFault {
    FwdFault::CpuCeFb {
        phys: e.phys,
        why: e.why,
    }
}

/// Map a guest-RAM refusal to the named fault — the SAME split
/// [`kayfabe_fwd`]'s own `guest_read` makes, so a device-aimed operand and an unbacked one
/// stay distinguishable one layer over (`testing_doctrine.md` §2 rule 3).
fn ram_fault(e: VmmError) -> FwdFault {
    match e {
        VmmError::NonRamGpa { gpa } => FwdFault::NonRamGpa { gpa },
        VmmError::BadGpa { gpa } => FwdFault::GpaRead { gpa },
        _ => FwdFault::GpaRead {
            gpa: u64::MAX, // no address the port named; the variant carries none
        },
    }
}

/// ★★★ **The plane a residency answer permits us to touch — or a named refusal.**
///
/// # ⚠ The backing check is the point, and it is load-bearing today
///
/// A CPU copy assumes its operands hold still for its duration. [`Backing::Stable`] says
/// they do — the emulated framebuffer is ours outright and guest RAM is held by the
/// hypervisor for the life of the machine. [`Backing::HostOwned`] says they do **not**: the
/// managed-memory case, where the backing is a host `cudaMallocManaged` allocation and host
/// UVM may migrate it below the guest's addressing (`mode2_uvm_residency.md`, DECIDED
/// 2026-06-04, measured at host parity in the C artifact).
///
/// ⊘ **Refusing it is not a claim the case is impossible** — the C ran it. It is a claim
/// that *this port has built no interlock*, and copying from a page that may move mid-copy
/// is a silent wrong-bytes generator. When the interlock lands, the fix is here and the
/// executor's shape is unchanged, which is the whole reason the fact is a **field** rather
/// than a plane variant.
///
/// ★ Note the plane itself is unchanged for a host-owned backing: managed memory is still
/// guest RAM read through the `Vmm`. Place and ownership are independent, which is why they
/// are two fields.
fn touchable(op: CpuOperand) -> Result<CpuPlane, FwdFault> {
    match op.residency.backing {
        Backing::Stable => Ok(op.residency.plane),
        Backing::HostOwned => Err(FwdFault::CeUnstableBacking { addr: op.addr.0 }),
    }
}

/// Read `buf.len()` bytes from `plane` at [`PlaneAddr`] `addr` into `buf`.
///
/// ⊘ **`addr` is a [`PlaneAddr`], never a `u64`,** and that signature is load-bearing rather
/// than decorative: the whole of §14.14's REFUTED 4 is this function having been reachable
/// with a GPU virtual address, which for a physical operand is the same number and for a
/// virtual one is a different page.
fn read_plane(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    plane: CpuPlane,
    addr: PlaneAddr,
    buf: &mut [u8],
) -> Result<(), FwdFault> {
    match plane {
        CpuPlane::Fb => fb.read(addr.0, buf).map_err(fb_fault),
        CpuPlane::GuestRam => vmm.gpa_read(addr.0, buf).map_err(ram_fault),
    }
}

/// Write `bytes` to `plane` at [`PlaneAddr`] `addr`. See [`read_plane`] for why the type of
/// `addr` is the point.
fn write_plane(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    plane: CpuPlane,
    addr: PlaneAddr,
    bytes: &[u8],
) -> Result<(), FwdFault> {
    match plane {
        // ★ §16.16 — the shell's CPU copy executor NAMES ITSELF. Without this the executor's
        // pages were indistinguishable from a window's in the first-writer census, and
        // "which window put the ring's neighbours there" is the whole question the census
        // is for. See `kayfabe_device::FbWriter`.
        CpuPlane::Fb => fb
            .write_tagged(addr.0, bytes, FbWriter::Executor)
            .map_err(fb_fault),
        CpuPlane::GuestRam => vmm.gpa_write(addr.0, bytes).map_err(ram_fault),
    }
}

/// ★★★ **Execute ONE `CeExecutor::Ours` sub-copy in the shell.**
///
/// The three cases, all address-driven:
/// - **fill / scrub** ([`CeSource::Constant`]) — writes a byte-pattern into the destination
///   plane, its phase taken from the **absolute destination address** (`pattern_le[a % 4]`),
///   which is what makes a split fill byte-identical to a whole one (the same rule
///   `kayfabe_mocks`'s reference `ce_apply` and the C's remap component follow). A scrub is
///   the `pattern == 0` case.
/// - **copy** ([`CeSource::Address`]) — streams bytes from the source plane to the
///   destination plane through a bounded buffer.
///
/// # Errors
/// - [`FwdFault::CpuCeStraddle`] if a plane this sub-copy must touch is `None` — a straddle
///   the shell cannot span (one end is real device memory or untracked). Refused, never
///   guessed into a store.
/// - [`FwdFault::CpuCeFb`] / [`FwdFault::NonRamGpa`] / [`FwdFault::GpaRead`] if a store
///   refuses an access.
///
/// # Panics
/// Debug-asserts the sub-copy is `Ours`; a `HostCe` sub-copy is the isolate's and must never
/// reach here (the divert in `SharedDevice::forward_ce` is what keeps that true).
pub fn execute_ours(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    span: &CeSpan,
) -> Result<(), FwdFault> {
    debug_assert_eq!(
        span.sub.by,
        CeExecutor::Ours,
        "the CPU executor runs only Ours sub-copies; HostCe is the isolate's"
    );
    let dst = span.sub.dst;
    let len = span.sub.len;
    // ★★★ The destination **place** must exist for anything to be written — and it is the
    // place, not `sub.dst`, that says where. `sub.dst` is a GPU virtual address (the `HostCe`
    // arm submits it to a host VAS); this executor reaches bytes in *our* stores and has no
    // MMU, so it uses the address the partition resolved (§14.14 REFUTED 4).
    let dst_op = span
        .dst_place
        .ok_or(FwdFault::CpuCeStraddle { dst, dst_end: true })?;
    let dst_plane = touchable(dst_op)?;

    match span.sub.src {
        CeSource::Constant(pattern) => {
            let p = pattern.to_le_bytes();
            let mut off: u64 = 0;
            let mut chunk = vec![0u8; CHUNK.min(usize::try_from(len).unwrap_or(CHUNK))];
            while off < len {
                let take = (len - off).min(chunk.len() as u64) as usize;
                // Phase every byte by its ABSOLUTE address in the destination PLANE, not by
                // its offset within this chunk — a fill cut at an unaligned address must
                // still land the same bytes, which is what makes a split fill byte-identical
                // to a whole one.
                //
                // ★ Phasing by the plane address rather than by the VA is the same answer
                // wherever the engine's own is defined: a mapping is page-granular, so a VA
                // and its backing agree on the low bits, and `Ga10xPushbuffer::remap_fill`
                // refuses a fill whose destination is not element-aligned in the first place
                // (§14.14's sixth refusal). Where they could differ — a test binding with an
                // arbitrary `phys` — the plane address is the one the guest will read back.
                for (i, b) in chunk[..take].iter_mut().enumerate() {
                    let a = dst_op.addr.offset(off + i as u64);
                    *b = p[(a.0 % 4) as usize];
                }
                write_plane(fb, vmm, dst_plane, dst_op.addr.offset(off), &chunk[..take])?;
                off += take as u64;
            }
            Ok(())
        }
        CeSource::Address(_) => {
            // A real copy needs BOTH ends reachable; a missing source place is the same
            // straddle, named on the source end.
            let src_op = span.src_place.ok_or(FwdFault::CpuCeStraddle {
                dst,
                dst_end: false,
            })?;
            let src_plane = touchable(src_op)?;
            let mut off: u64 = 0;
            let mut chunk = vec![0u8; CHUNK.min(usize::try_from(len).unwrap_or(CHUNK))];
            while off < len {
                let take = (len - off).min(chunk.len() as u64) as usize;
                read_plane(
                    fb,
                    vmm,
                    src_plane,
                    src_op.addr.offset(off),
                    &mut chunk[..take],
                )?;
                write_plane(fb, vmm, dst_plane, dst_op.addr.offset(off), &chunk[..take])?;
                off += take as u64;
            }
            Ok(())
        }
    }
}

/// Execute every `Ours` sub-copy of `spans` in submission order, skipping `HostCe` ones (the
/// isolate's). Returns the count actually run.
///
/// ★ **Submission order is preserved** because a copy engine's within-request ordering is
/// what the guest's own semaphore release depends on. This runs the shell's share; the
/// caller ([`crate::device::SharedDevice::forward_ce`]) interleaves the isolate's `HostCe`
/// share and places the E10d completion signal only after **all** of a request's bytes —
/// both shares — are in place.
///
/// # Errors
/// The first sub-copy that refuses stops the run and propagates, exactly as the isolate's
/// [`kayfabe_isolate::VerbPlan::CeSplit`] loop stops on the first refusal: a later sub-copy
/// must not assume an earlier one landed.
pub fn execute_ours_spans(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    spans: &[CeSpan],
) -> Result<usize, FwdFault> {
    let mut ran = 0usize;
    for span in spans {
        if span.sub.by == CeExecutor::Ours {
            execute_ours(fb, vmm, span)?;
            ran += 1;
        }
    }
    Ok(ran)
}

// =====================================================================================
// ★★★ E10d — THE COMPLETION WRITE-BACK TAIL. There is NONE today: the finishPayload
// semaphore the guest polls is written nowhere (`execution_plane_increments.md` §14.4(3),
// §14.5 E10d). This is the `sem_releases` consumer.
// =====================================================================================

/// ★★★ **One finishPayload release, with its address already RESOLVED to a plane
/// address** — the output of [`resolve_releases`] and the input of
/// [`write_resolved_completion`].
///
/// # ⊘ Why the split exists, and it is a borrow fact rather than a style one
///
/// `write_completion` used to resolve and write in one pass, which is right when the
/// resolver and the destination store are different objects. On the CeUtils path they are
/// **the same object**: the resolver walks the guest's page tables out of the emulated
/// framebuffer, and the executor writes the guest's bytes into that same framebuffer
/// (`execution_plane_increments.md` §14.13 — the walling channel's page directories are
/// vidmem). One `&mut` cannot be held twice, so the resolution is a **phase** and its
/// result is a value.
///
/// ★ The ordering the split must not lose is *resolve → write → signal*, and it does not:
/// [`write_resolved_completion`] still writes every payload before raising anything, and
/// raises nothing at all on a refusal.
///
/// # ★★★★★ `#[non_exhaustive]`, and it is the SINGLE-WRITER RULE made a type fact
///
/// `[measured 2026-08-10, boot `w226b_534e1b3_cup2`, and the static census in
/// `completion_observer.md` §8]` there is exactly **one** function in this tree that writes
/// a completion semaphore ([`write_resolved_completion`]) and **zero** writers to the page
/// `cuCtxCreate` polls. §8.4 then stated the honest limit of that finding: the property
/// *"the release is a `ResolvedRelease` and never becomes a [`kayfabe_fwd::CeSpan`]"* was
/// **accidental — asserted nowhere, and one refactor from untrue.**
///
/// ⊘ **This is not a hypothetical.** The C artifact's `MC_SERVICE_INTERRUPTS` spin — 118
/// occurrences, two months of it read as a *missing* completion — was a **corrupted** one: a
/// lagging second writer DMA-ing stale payloads `1,2` over a live `0x1e`, which UVM's
/// 32→64-bit wrap detector read as a backwards jump (`uvm_gpu_semaphore.c:776`), wedging the
/// channel (`uvm_channel.c:205`). **The C's entire fix (M5.38, `ceb13f5`) was to DELETE THE
/// SECOND WRITER.** A backwards write is fatal on FIRST occurrence — `UVM_ASSERT_MSG_RELEASE`
/// is compiled into release builds — so this is not a property that degrades gracefully.
///
/// ⇒ `#[non_exhaustive]` means a `ResolvedRelease` **cannot be struct-expression-constructed
/// outside this crate**, so the only way to obtain one is [`resolve_releases`], which is the
/// only code that asks the guest's own page tables where the guest's own semaphore VA lands.
/// *"This address came from the guest's page tables"* stops being a convention about who
/// calls what and becomes a fact the compiler enforces. Same idiom, and for the same reason,
/// as `kayfabe_isolate::VerbPlan::Doorbell` and `DeclaredCompletion` in
/// [`crate::completion_watch`] — the payload is the **guest's** literal, so a value claiming
/// to be one that did not come from guest bytes must not be expressible.
///
/// ⚠ The fields stay `pub` **for reading**: [`crate::ceutils::CeUtilsRun::completion_at`]
/// and the boot report need them, and hiding them would buy nothing — the danger was never
/// reading a release, it was minting one. The compile-fail row that pins this is
/// `crates/kayfabe-rt/tests/ui/mint_a_release.rs`; the *census* half — that no SECOND writer
/// appears anywhere in the workspace — is `tests/tests/single_writer_census.rs`, because a
/// type cannot see a caller that never touches the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedRelease {
    /// The virtual address the guest's pushbuffer named, kept for the report.
    pub va: GpuVa,
    /// Where the release's **first** word lands. ★ This is the `#12` answer — the plane
    /// and plane address the guest's semaphore VA actually resolved to — and it is what
    /// [`crate::ceutils::CeUtilsRun::completion_at`] carries into the boot report.
    pub op: CpuOperand,
    /// The payload the release carries, already widened per `SEMAPHORE_PAYLOAD_SIZE`.
    pub payload: u64,
    /// The record this release writes — [`CeSemStructure::FourWord`] is sixteen bytes.
    pub structure: CeSemStructure,
    /// How many of the payload's bytes the guest asked for: 4 or 8.
    pub payload_bytes: u8,
    /// ★★★ **Every four-byte word this release writes, resolved SEPARATELY**: index `i`
    /// is the word at byte offset `4 * i` from [`ResolvedRelease::va`], `Some` exactly for
    /// the words the record covers.
    ///
    /// # ⊘ Why each word gets its own resolution rather than one base plus arithmetic
    ///
    /// Because a sixteen-byte record can **straddle a page boundary**, and the two halves
    /// of a straddle need not be contiguous — or even in the same plane — once the guest's
    /// own page tables have had their say. Adding 12 to a resolved plane address assumes
    /// they are, and that assumption fails silently: the bytes land in whatever page
    /// happens to follow, which is somebody else's. The four-byte granularity is the
    /// resolver's own ([`kayfabe_fwd::OperandResolver::resolve_word`]), so this asks it
    /// exactly the question it can answer, once per word.
    ///
    /// ⊘ A fixed array and not a `Vec`: the record is at most sixteen bytes by the class's
    /// own definition, and keeping this `Copy` keeps the resolve/write split a value.
    pub words: [Option<CpuOperand>; 4],
}

impl ResolvedRelease {
    /// The sixteen (or four, or eight) bytes this release puts at [`ResolvedRelease::va`],
    /// as a little-endian record, with `None` for a word the release does not write.
    ///
    /// # ★★★ The four-word record, byte for byte, and the one judgement call in it
    ///
    /// `{ payload: u64, timestamp: u64 }` — the timestamp at
    /// [`submit::ce::SEM_FOUR_WORD_TIMESTAMP_OFFSET`], which is where the driver that
    /// reads the field looks for it (`uvm_push.c:478`, `timestamp += 1` on an `NvU64 *`).
    ///
    /// ⚠ **The judgement call**: when `SEMAPHORE_PAYLOAD_SIZE` is `_ONE_WORD` the guest
    /// named only 32 bits of payload, yet the record's payload slot is 64 bits wide. This
    /// **zero-extends** into the slot rather than leaving bytes `4..8` untouched. Stated
    /// as a choice because it is one:
    /// - a reader that takes the payload as a `u32` — every reader in reach — cannot tell
    ///   the two apart;
    /// - a reader that takes it as a `u64` gets the payload it asked for under
    ///   zero-extension, and gets **whatever was in that memory before** under the other
    ///   choice. Between a defined value and a stale one, this project's rule is not to
    ///   leave a byte of a record it claims to have written to chance.
    ///
    /// ⊘ It is *not* a claim about what hardware does with those four bytes — no run in
    /// this tree has observed them. It is a choice about what WE write, made where it can
    /// be changed in one place, with this paragraph as the reason it was reachable.
    #[must_use]
    pub fn record(&self, timestamp_ns: u64) -> [Option<u32>; 4] {
        let p = self.payload.to_le_bytes();
        let payload_words = [
            Some(u32::from_le_bytes([p[0], p[1], p[2], p[3]])),
            Some(u32::from_le_bytes([p[4], p[5], p[6], p[7]])),
        ];
        match self.structure {
            CeSemStructure::OneWord => [
                payload_words[0],
                if self.payload_bytes >= 8 {
                    payload_words[1]
                } else {
                    None
                },
                None,
                None,
            ],
            CeSemStructure::FourWord => {
                let t = timestamp_ns.to_le_bytes();
                [
                    payload_words[0],
                    payload_words[1],
                    Some(u32::from_le_bytes([t[0], t[1], t[2], t[3]])),
                    Some(u32::from_le_bytes([t[4], t[5], t[6], t[7]])),
                ]
            }
        }
    }
}

/// ★★★ **Phase 1 of the completion tail — resolve every release's VA through the operand
/// resolver, refusing by name rather than writing anywhere.**
///
/// # Errors
/// - Whatever the resolver refuses with — [`FwdFault::Address`] for a table miss (MISS =
///   FAULT: a completion aimed at nothing is not written and does not signal),
///   [`FwdFault::CeNoTable`] where there is no address space to miss in, or the walk's own
///   named fault.
/// - [`FwdFault::CePeerOperand`] if it resolves into peer memory.
pub fn resolve_releases(
    ops: &mut dyn kayfabe_fwd::OperandResolver,
    releases: &[CeCompletion],
) -> Result<Vec<ResolvedRelease>, FwdFault> {
    releases
        .iter()
        .map(|c| {
            // How many four-byte words the record covers. ⊘ From the STRUCTURE, which is
            // what the class says the engine writes, and never from the payload width —
            // those are two different fields and a four-word release with a one-word
            // payload still writes sixteen bytes.
            let words = match c.structure {
                CeSemStructure::OneWord => u64::from(c.payload_bytes).div_ceil(4),
                CeSemStructure::FourWord => submit::ce::SEM_FOUR_WORD_BYTES / 4,
            };
            let mut resolved: [Option<CpuOperand>; 4] = [None; 4];
            for (i, slot) in resolved.iter_mut().enumerate() {
                if (i as u64) < words {
                    // ⚠ MISS = FAULT on EVERY word, including the timestamp's. A record
                    // whose second half does not resolve is not written at all and does
                    // not signal — a half-written completion is worse than a refused one.
                    *slot = Some(ops.resolve_word(GpuVa(c.addr.0 + 4 * i as u64))?);
                }
            }
            Ok(ResolvedRelease {
                va: c.addr,
                op: resolved[0].ok_or(FwdFault::CeNoTable { va: c.addr })?,
                payload: c.payload,
                structure: c.structure,
                payload_bytes: c.payload_bytes,
                words: resolved,
            })
        })
        .collect()
}

/// ★★★ **Phase 2 of the completion tail — write every resolved payload, and only then
/// raise the completion interrupt.**
///
/// See [`write_completion`] for the `#12` argument this ordering is.
///
/// # Errors
/// - [`FwdFault::CeUnstableBacking`] if a release's backing is not ours to hold still.
/// - [`FwdFault::CpuCeFb`] / [`FwdFault::NonRamGpa`] / [`FwdFault::GpaRead`] on a store
///   refusal. In every error case **no interrupt is raised**.
pub fn write_resolved_completion(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    releases: &[ResolvedRelease],
    timestamp_ns: Option<u64>,
) -> Result<usize, FwdFault> {
    // ⊘ CHECK EVERY WORD OF EVERY RECORD BEFORE WRITING ANY OF THEM. A four-word release
    // whose timestamp half sits on unstable backing must not have its payload half land
    // first: the guest's waiter would be released over a record only half of which is ours
    // to have written. `touchable` is pure, so this pass cannot itself have an effect.
    for r in releases {
        for op in r.words.iter().flatten() {
            touchable(*op)?;
        }
    }
    // ★★★ **A TIMESTAMPED RELEASE WITHOUT A CLOCK IS REFUSED, NOT ZEROED.**
    //
    // `0` is a legal `PTIMER` reading, so a guest handed one cannot tell it from a real
    // sample — it just computes a nonsense elapsed time and believes it. `[measured
    // 2026-08-10, boot `s51_d502ac6_engroute`]` the semaphore page in question reads
    // `fbFIN@0x102c004=0000…0000 nz0/4096 resN-NEVER-WRITTEN`, i.e. zeros are exactly what
    // "we never wrote this" already looks like there. That is the C oracle's fifth-limit
    // failure (*"an empty capture is evidence of nothing, not evidence of emptiness"*),
    // and the standing rule
    // `kayfabe_device::NanoClock`'s own docs state for this device is **never answer a
    // free-running counter with a constant**. So the source is a precondition with a
    // name, checked before a byte moves.
    let ts = match (
        releases
            .iter()
            .any(|r| r.structure == CeSemStructure::FourWord),
        timestamp_ns,
    ) {
        (true, None) => return Err(FwdFault::CeReleaseNoClock),
        (_, t) => t.unwrap_or(0),
    };
    // 1. WRITE every record. A refusal here returns before any signal.
    for r in releases {
        let record = r.record(ts);
        for (i, op) in r.words.iter().enumerate() {
            let (Some(op), Some(word)) = (op, record[i]) else {
                continue;
            };
            write_plane(fb, vmm, touchable(*op)?, op.addr, &word.to_le_bytes())?;
        }
    }
    // 2. SIGNAL — only now, and only because every write above returned Ok. The guest's
    //    poll of `pbCpuVA` already sees the values; the interrupt wakes a blocking waiter.
    if !releases.is_empty() {
        vmm.raise_irq(COMPLETION_VECTOR).map_err(ram_fault)?;
    }
    Ok(releases.len())
}

/// ★★★ **Write a copy-engine request's finishPayload semaphores where the guest polls
/// them, then — and only then — raise the completion interrupt.**
///
/// Each `(addr, payload)` is a `SEM_RELEASE` the guest's own pushbuffer asked for. `addr`
/// is a GPU **virtual** address in the *channel's own VAS* — `pChannel->pbGpuVA +
/// finishPayloadOffset` (`ogkm-580: channel_utils.c:838-840`) — and the guest reads the
/// same physical allocation back through its CPU mapping (`pbCpuVA`). So the payload is
/// written to whatever that VA resolves to **in this channel's table**, in the channel's own
/// aperture — never to a scratch framebuffer page the guest does not look at.
///
/// # ⊘ #12, the bug this ordering exists to avoid
///
/// The C artifact's `#12` wrote completion **data to the framebuffer while the guest polled
/// `pbCpuVA`** — a semaphore that advanced somewhere the guest never read — and it cost
/// weeks. Two disciplines here are that lesson, made structural:
/// 1. **Where** — the payload lands at the resolved physical of the guest's own semaphore
///    VA, so `pbCpuVA` sees it.
/// 2. **When** — the interrupt is raised **after every byte is in place**, and **not at
///    all** if any write refuses. A completion signal for work whose result is not yet
///    visible is exactly the forgery `mode2_forwarding_model.md` forbids.
///
/// The finishPayload is a **one-word** (4-byte) release
/// (`ogkm-580: channel_utils.c:732` `_RELEASE_SIZE_4BYTE`, `:836` `_RELEASE_ONE_WORD`), so
/// the low 32 bits are written and the adjacent `authTagBufSema` (`:` `finishPayloadOffset +
/// CHANNEL_ENGINE_SEMAPHORE_SIZE`) is never clobbered by an over-wide store.
///
/// # Errors
/// - [`FwdFault::Address`] if a semaphore VA does not resolve in the channel's table
///   (MISS = FAULT — a completion aimed at nothing is not written and does not signal).
/// - [`FwdFault::CePeerOperand`] if it resolves into peer memory.
/// - [`FwdFault::CpuCeFb`] / [`FwdFault::NonRamGpa`] / [`FwdFault::GpaRead`] on a store
///   refusal. In every error case **no interrupt is raised** — the writes that DID land are
///   the guest's own memory and are harmless, but no completion is claimed.
///
/// Returns the number of semaphores written on success.
pub fn write_completion(
    fb: &mut dyn FbStore,
    vmm: &mut dyn Vmm,
    ops: &mut dyn kayfabe_fwd::OperandResolver,
    releases: &[(GpuVa, u64)],
) -> Result<usize, FwdFault> {
    // ★ These are host-FIFO / `channel_utils` finishPayload releases, which the citation
    // above measures as one-word 4-byte writes — so they are built as exactly that, and
    // `None` for the timestamp source is not a gap: a `OneWord` record has no timestamp
    // word to fill, and `write_resolved_completion` refuses only if one asks for one.
    let completions: Vec<CeCompletion> = releases
        .iter()
        .map(|&(addr, payload)| CeCompletion {
            addr,
            payload,
            structure: CeSemStructure::OneWord,
            payload_bytes: 4,
        })
        .collect();
    let resolved = resolve_releases(ops, &completions)?;
    write_resolved_completion(fb, vmm, &resolved, None)
}
