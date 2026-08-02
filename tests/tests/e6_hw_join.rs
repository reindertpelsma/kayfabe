//! # ★★★ E6 — **THE ACCEPTANCE, ON REAL HARDWARE**: `CeEvidence::copied()`, driven by a
//! guest's own pushbuffer through the production core.
//!
//! `docs/design/execution_plane_increments.md`, the E6 row:
//!
//! > the join: guest CE copy → `plan_doorbell` → `Worker::execute` →
//! > `HostRmBackend::ce_copy_outcome`.
//! > **acceptance that could fail:** `CeEvidence::copied()` — R17's predicate.
//!
//! # ★★★ What is driven, and by what — read this before reading a green
//!
//! Rung **R17** (`kayfabe-rm-ladder`) already proved a real copy engine moves bytes: it
//! hand-builds two buffers, hand-builds one `LAUNCH_DMA`, and reads the destination back.
//! **This file changes exactly one thing and nothing else**: the copy is not hand-built.
//! It is *recovered from a guest's GPFIFO ring* by the production core —
//! `read_pushbuffer` translating the entry's VA through the channel's address table,
//! `decode_run` accumulating five GA10x method runs into one `CeLaunchDma`, `partition_ce`
//! splitting it by representability, `plan_ce` planning it, `Worker::execute` running it —
//! and only then does `HostRmBackend::ce_copy_outcome` see a copy engine.
//!
//! The predicate is **not re-derived**. It is [`CeEvidence::copied()`] itself, built from
//! the destination read back through an independent second mapping *and* from the engine's
//! own release semaphore, which the join's own [`CeWitness`] recorded.
//!
//! # ⊘ THE THREE THINGS THIS DOES NOT CLAIM, stated first
//!
//! 1. ⊘ **The guest is not a booted guest.** It is a guest's *bytes* and a guest's
//!    *declarations*: a real GA10x GPFIFO entry (`kayfabe_abi::submit::gp_entry`) naming
//!    real GA10x CE method runs, applied to a `Gpu` populated by a real `RmEvent` chain of
//!    NVIDIA class ids. A booted guest cannot reach here — its `RmInitAdapter` dies at
//!    `_memmgrMemUtilsScrubInitScheduleChannel` before it ever rings — and saying otherwise
//!    would be the shape `only_live_boots_are_proof` forbids. §10 of the design doc states
//!    the three named things that still separate this from a live boot.
//! 2. ⊘ **The isolate is IN-PROCESS, not sandboxed.** A [`CeWitness`] cannot cross a
//!    process boundary, and neither can a CPU mapping of the destination — so the evidence
//!    R17 defines is only *observable* in-process. That the same verb chain works through
//!    the real sandboxed child is R10/R11/R16's measurement, not this one's, and the two
//!    are cited separately.
//! 3. ⊘ **Guest RAM is a `MockVmm`.** The method words live in a `BTreeMap`, because no
//!    hypervisor is running. That is honest for what is under test: the *core* reads those
//!    bytes, the *GPU* never does. What the GPU touches — the operands — is real host
//!    memory at real host GPU virtual addresses.
//!
//! # The gate
//!
//! Every test prints `GPU-GATE: RAN <name>` or `GPU-GATE: SKIPPED <name> — …` to stderr in
//! **both** arms, so a run that measured nothing says so rather than looking like a pass.
//! ⊘ A skip asserts **nothing** and substitutes **nothing**.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::sync::Arc;

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::{ChanId, ProcId};
use kayfabe_isolate::{
    CancelHandle, CeExecutor, Isolate, IsolateFactory, IsolateId, IsolateRefusal, Worker, WorkerId,
};
use kayfabe_isolate_host::rm::{CeEvidence, CeWitness, HostRmBackend, RmConnection};
use kayfabe_linux_raw::DevDir;
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_vmm::Vmm;

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0x5c00_0000);
const PDB: Pdb = Pdb(0x4E60_0000);

/// The guest's two operands, and they are **guest** numbers: `#102` address identity maps
/// each published range into the host VAS at the identical address, which is the only
/// reason a pushbuffer carrying guest VAs can be executed at all.
const SRC_VA: GpuVa = GpuVa(0x0000_0002_0000_0000);
const DST_VA: GpuVa = GpuVa(0x0000_0002_0010_0000);
const COPY_LEN: u64 = 4096;
const WORDS: u64 = COPY_LEN / 4;

/// Where the ring's method words are scripted in (simulated) guest RAM, and the GPU VA the
/// GPFIFO entry names them by. The two differ deliberately — an identity fixture could not
/// tell a translating read from an untranslated one (§8.2.3).
const RING_GPA: u64 = 0x1_0000_0000;
const RING_VA: GpuVa = GpuVa(0x0000_0080_0000_0000);

/// The source's ramp and the destination's sentinel. The sentinel is **not** the ramp's
/// first word, so "the copy happened" and "the destination already looked like that" are
/// distinguishable outcomes.
const PATTERN: u32 = 0xC0FF_EE00;
const SENTINEL: u32 = 0x5EED_5EED;

// =====================================================================================
// The gate
// =====================================================================================

fn gate(test: &str) -> Option<Arc<RmConnection>> {
    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            eprintln!("GPU-GATE: SKIPPED {test} — /dev could not be opened ({e:?})");
            return None;
        }
    };
    match RmConnection::open(&dev, GPU, kayfabe_chips::pinned_host_classes()) {
        Ok(c) => {
            eprintln!("GPU-GATE: RAN {test}");
            Some(Arc::new(c))
        }
        Err(e) => {
            eprintln!(
                "GPU-GATE: SKIPPED {test} — no NVIDIA RM connection on this box ({e}). \
                 This test asserts NOTHING here and nothing is substituted for it."
            );
            None
        }
    }
}

// =====================================================================================
// The in-process isolate — one worker over a real `HostRmBackend`
// =====================================================================================

/// ⊘ **Not the shipped plane, and it is not pretending to be.** The shipped plane is
/// `kayfabe_isolate_host::HostIsolateFactory`, which forks a capability-less sandboxed
/// child; R10/R11/R16 measure that the same verb chain survives it. This factory exists so
/// the destination's bytes and the engine's semaphore are *observable* — both die at the
/// process boundary — and for no other reason.
struct InProcessIsolates {
    conn: Arc<RmConnection>,
    witness: Arc<CeWitness>,
}

struct InProcessIsolate {
    id: IsolateId,
    idle: Vec<Worker>,
    out: Vec<WorkerId>,
    retired: bool,
}

impl IsolateFactory for InProcessIsolates {
    fn spawn(&mut self, id: IsolateId) -> Box<dyn Isolate> {
        let backend = HostRmBackend::new(
            id,
            Arc::clone(&self.conn),
            Arc::new(kayfabe_isolate_host::ChildExports::new()),
        )
        .with_ce_witness(Arc::clone(&self.witness));
        Box::new(InProcessIsolate {
            id,
            idle: vec![Worker::new(id, WorkerId(0), Box::new(backend))],
            out: Vec::new(),
            retired: false,
        })
    }
}

impl Isolate for InProcessIsolate {
    fn id(&self) -> IsolateId {
        self.id
    }
    fn pool_size(&self) -> usize {
        1
    }
    fn idle_workers(&self) -> usize {
        self.idle.len()
    }
    fn checkout(&mut self) -> Option<Worker> {
        if self.retired {
            return None;
        }
        let w = self.idle.pop()?;
        self.out.push(w.id());
        Some(w)
    }
    fn checkin(&mut self, worker: Worker) {
        self.out.retain(|&w| w != worker.id());
        self.idle.push(worker);
    }
    fn checked_out(&self) -> Vec<WorkerId> {
        self.out.clone()
    }
    fn cancel_handle(&self, _worker: WorkerId) -> Option<CancelHandle> {
        None
    }
    fn worker_died(&mut self, _worker: WorkerId) -> bool {
        false
    }
    fn in_flight(&self) -> usize {
        self.out.len()
    }
    fn retire(&mut self) {
        self.retired = true;
    }
    fn is_retired(&self) -> bool {
        self.retired
    }
    fn refusal(&self) -> Option<IsolateRefusal<'_>> {
        None
    }
}

// =====================================================================================
// The guest's declarations and the guest's ring
// =====================================================================================

/// The five method runs a real `AMPERE_DMA_COPY_B` copy is — the same shape
/// `kayfabe_isolate_host::rm::ce_pushbuffer` emits and a real GA106 executed at R17,
/// encoded here as the **guest** would put them in its pushbuffer.
fn ce_runs(src: u64, dst: u64, len: u32) -> Vec<(u32, Vec<u32>)> {
    use kayfabe_abi::submit;
    let sub = 0u32;
    let hdr = |m, n| submit::method_header_inc(sub, m, n).expect("encodable");
    let flags = submit::ce::LAUNCH_TRANSFER_NON_PIPELINED
        | submit::ce::LAUNCH_FLUSH_ENABLE
        | submit::ce::LAUNCH_SRC_PITCH
        | submit::ce::LAUNCH_DST_PITCH;
    vec![
        (
            hdr(submit::SET_OBJECT, 1),
            vec![kayfabe_abi::generated::classes::AMPERE_DMA_COPY_B],
        ),
        (
            hdr(submit::ce::OFFSET_IN_UPPER, 4),
            vec![
                (src >> 32) as u32,
                src as u32,
                (dst >> 32) as u32,
                dst as u32,
            ],
        ),
        (hdr(submit::ce::LINE_LENGTH_IN, 2), vec![len, 1]),
        (hdr(submit::ce::LAUNCH_DMA, 1), vec![flags]),
    ]
}

/// Script `methods` into (simulated) guest RAM at [`RING_GPA`] and return the **one-entry
/// GA10x GPFIFO ring** naming them at [`RING_VA`].
///
/// ⊘ Built here rather than with `kayfabe_tests::script_ring_via`, because that helper
/// emits `kayfabe_mocks::MockPushbuffer`'s invented 16-byte entry and this device's arch is
/// `Ga10xArch`. The entry below is `kayfabe_abi::submit::gp_entry` — NVIDIA's own field
/// layout, the one `pushbuffer_abi_oracle.rs` differentials against the driver's macros.
fn ga10x_ring(vmm: &mut dyn Vmm, methods: &[(u32, Vec<u32>)]) -> (Vec<u8>, u64) {
    let mut bytes = Vec::new();
    for (h, args) in methods {
        bytes.extend_from_slice(&h.to_le_bytes());
        for a in args {
            bytes.extend_from_slice(&a.to_le_bytes());
        }
    }
    vmm.gpa_write(RING_GPA, &bytes)
        .expect("scripting a legitimate pushbuffer into guest RAM");
    let entry = kayfabe_abi::submit::gp_entry(RING_VA.0, bytes.len() as u64)
        .expect("the ring's VA and length are representable in a GP_ENTRY");
    (entry.to_le_bytes().to_vec(), bytes.len() as u64)
}

/// A GA10x process, declared with NVIDIA's own class ids so it materializes against
/// `Ga10xArch` — the same chain `kayfabe_tests::ga10x_process` builds.
fn declare_process(gpu: &mut Gpu) -> (ProcId, ChanId) {
    let mut s = kayfabe_tests::Scenario::new();
    kayfabe_tests::ga10x_process(&mut s, CLIENT, PDB, CLIENT.0);
    for ev in s.events {
        gpu.apply(ev).expect("the guest's declarations apply");
    }
    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB)).expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the declared channel");
    (pid, cid)
}

// =====================================================================================
// ★★★ THE ACCEPTANCE
// =====================================================================================

/// ★★★ **`CeEvidence::copied()` — driven by a guest's ring, through the core, onto a real
/// copy engine.**
///
/// ★ Every failure below prints the whole [`CeEvidence`], because *which* conjunct failed
/// is the diagnosis: bytes that did not change at all is a different fault from bytes that
/// changed with no semaphore, which is a different fault from a truncated copy.
#[test]
fn a_guests_ring_moves_bytes_on_the_host_gpu_and_the_guest_reads_them_back() {
    let Some(conn) =
        gate("a_guests_ring_moves_bytes_on_the_host_gpu_and_the_guest_reads_them_back")
    else {
        return;
    };
    let witness = Arc::new(CeWitness::new());

    // ---- the device, with a REAL isolate plane
    let mut gpu = Gpu::new(
        Box::new(kayfabe_chips::Ga10xArch::new()),
        Box::new(InProcessIsolates {
            conn: Arc::clone(&conn),
            witness: Arc::clone(&witness),
        }),
        GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000),
    )
    .expect("the device realizes");
    let (pid, cid) = declare_process(&mut gpu);
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));

    // ---- the guest's two ranges, published into ITS OWN address space at ITS OWN
    //      addresses. This is what makes the pushbuffer's numbers executable.
    let src = dev
        .publish_backing(GPU, PDB, SRC_VA, COPY_LEN)
        .expect("the source range publishes on the host GPU");
    let dst = dev
        .publish_backing(GPU, PDB, DST_VA, COPY_LEN)
        .expect("the destination range publishes on the host GPU");
    assert_eq!(
        (src.host_va, dst.host_va),
        (SRC_VA.0, DST_VA.0),
        "★ #102 address identity: the host MMU walks for the GUEST's number, or the \
         forwarded pushbuffer names nothing"
    );

    // ---- seed and read back, through a SECOND `HostRmBackend` on the same connection.
    //
    // ⊘ Deliberately not through the worker: the point of the read-back is that it is
    // **independent** of the mapping the value was written through, and going back through
    // the same backend would make that harder to see rather than easier. RM handles are
    // per-connection, so this probe addresses the same objects the isolate minted while
    // opening its own device node, its own mmap context and its own address for every
    // call — which is precisely `prove_ce_copy`'s "freshly opened, independent" mapping.
    let probe = HostRmBackend::new(
        IsolateId::new(0xE6, GPU),
        Arc::clone(&conn),
        Arc::new(kayfabe_isolate_host::ChildExports::new()),
    );
    probe
        .fill_words(src.memory, COPY_LEN, PATTERN, 1)
        .expect("the source is filled with a ramp");
    probe
        .fill_words(dst.memory, COPY_LEN, SENTINEL, 0)
        .expect("the destination is filled with the sentinel");

    let before = probe
        .read_words_independently(dst.memory, COPY_LEN, &[0])
        .expect("the destination reads back before the copy")[0];
    assert_eq!(
        before, SENTINEL,
        "★ non-vacuity: the destination provably did not already hold the answer"
    );

    // ---- the guest's ring: bind it, then submit it
    let mut vmm = kayfabe_mocks::MockVmm::new();
    let (ring, method_bytes) = ga10x_ring(&mut vmm, &ce_runs(SRC_VA.0, DST_VA.0, COPY_LEN as u32));
    dev.with_proc_mut(pid, |p| {
        let chan = p.channels.get(&cid).expect("the channel");
        let key = (chan.gpu, chan.vas_pdb.expect("it declares a VAS"));
        let vas = p.vases.get_mut(&key).expect("the VAS");
        kayfabe_tests::bind_ring_in(vas, RING_VA, RING_GPA, method_bytes);
    })
    .expect("live");

    let (parsed, forwarded) = dev
        .submit_ring(&mut vmm, pid, cid, &ring)
        .expect("★ the join runs: the guest's ring is read, decoded, planned and executed");

    assert_eq!(
        parsed.ce_spans.len(),
        1,
        "one contiguous copy over two published ranges is one sub-copy, {:?}",
        parsed.ce_spans
    );
    assert_eq!(
        parsed.ce_spans[0].sub.by,
        CeExecutor::HostCe,
        "★ both operands are HostBacked, so the plan chose a REAL engine"
    );
    assert_eq!((forwarded.host_ce, forwarded.ours), (1, 0));

    // ---- ★★★ THE PREDICATE. Not a re-derivation of R17's — R17's.
    let (submit, payload) = witness
        .latest()
        .expect("★ the join reached RmBackend::ce_copy, so a submission was observed");
    let after = probe
        .read_words_independently(dst.memory, COPY_LEN, &[0, (WORDS - 1) * 4])
        .expect("the destination reads back through an INDEPENDENT mapping");
    let evidence = CeEvidence {
        before,
        after: after[0],
        after_last: after[1],
        expect_after: PATTERN,
        expect_after_last: PATTERN.wrapping_add(WORDS as u32 - 1),
        bytes: COPY_LEN,
        submit,
        payload,
    };
    assert!(
        evidence.copied(),
        "★★★ E6 ACCEPTANCE FAILED. {evidence:?} — a `before` equal to `expect_after` \
         means the fixture was vacuous; `after` unchanged with the semaphore landed means \
         the engine ran a copy that moved nothing; `after` right and `after_last` wrong \
         means it was truncated; the semaphore not matching means it never retired."
    );
}
