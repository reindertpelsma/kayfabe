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
    CancelHandle, CeExecutor, Isolate, IsolateFactory, IsolateId, IsolateRefusal, RmBackend,
    Worker, WorkerId,
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
/// Arm 2's operands. Their own addresses, so arm 1's published ranges keep theirs — two
/// mappings at one VA in one address space is the `ALREADY-MAPPED` collision, not a test.
const PROBE_SRC_VA: GpuVa = GpuVa(0x0000_0002_0100_0000);
const PROBE_DST_VA: GpuVa = GpuVa(0x0000_0002_0110_0000);
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

/// ⚠ The marker goes **straight to the `stderr` descriptor**, not through `eprintln!`.
/// `libtest` captures a test thread's `print!`/`eprint!` and flushes it only when the test
/// **fails**, so a gate line written that way is invisible on exactly the runs that need
/// counting — `libtest_capture_swallows_thread_output`, and the reason `kvm_gate::report`
/// is written the same way. `[measured]` 2026-08-03, suite run at rev `a1cdfdd` on the
/// bench: `grep -c "GPU-GATE: RAN"` over a full `cargo test --workspace` was **0** while
/// the test passed against a real GA106.
fn gate_line(line: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{line}");
}

fn gate(test: &str) -> Option<Arc<RmConnection>> {
    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            gate_line(&format!(
                "GPU-GATE: SKIPPED {test} — /dev could not be opened ({e:?})"
            ));
            return None;
        }
    };
    match RmConnection::open(&dev, GPU, kayfabe_chips::pinned_host_classes()) {
        Ok(c) => {
            gate_line(&format!("GPU-GATE: RAN {test}"));
            Some(Arc::new(c))
        }
        Err(e) => {
            gate_line(&format!(
                "GPU-GATE: SKIPPED {test} — no NVIDIA RM connection on this box ({e}). \
                 This test asserts NOTHING here and nothing is substituted for it."
            ));
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
    fn spawn(&self, id: IsolateId) -> Box<dyn Isolate> {
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

/// ★★★ **THE E6 ACCEPTANCE, in two arms — and the second arm exists because the first
/// arm's destination is UNREADABLE BY DESIGN.**
///
/// # Arm 1 — the PRODUCT path: published guest ranges, `HostBacked`, executed
///
/// Two ranges published through `SharedDevice::publish_backing` at the **guest's own**
/// addresses (`#102` identity), a guest ring naming them, and the join. The operands
/// partition to `Representability::HostBacked`, the plan chooses a real engine, and the
/// engine's **own release semaphore** — the fourth conjunct of [`CeEvidence::copied()`],
/// and the one no CPU read can produce — is observed through the join's [`CeWitness`].
///
/// ⊘ **What arm 1 cannot show is the bytes.** `[measured]` here, 2026-08-03: a published
/// backing is minted by `RmBackend::alloc_sysmem`, which passes
/// `NVOS02_FLAGS_MAPPING_NO_MAP`, and `NV_ESC_RM_MAP_MEMORY` on it is refused
/// `NV_ERR_INVALID_ARGUMENT` (`0x1F`). That flag is a **documented product decision**
/// (*"right for a data buffer the GPU alone touches"*), so the object is opaque to the CPU
/// in both directions — the sentinel cannot be written and the answer cannot be read.
/// Relaxing it to make this test work would be changing the product to fit its instrument.
///
/// # Arm 2 — the BYTES: the same join over operands a CPU can see
///
/// Two **device-local** buffers ([`HostRmBackend::alloc_probe_local`], the class R17 itself
/// uses), mapped by hand into the **same host VAS the channel's `Vas` already holds**, at
/// their own guest addresses. The same `submit_ring` call, the same decode, the same plan,
/// the same `Worker::execute`. Then the destination is read back through a freshly opened,
/// independent mapping and [`CeEvidence::copied()`] is evaluated **in full**.
///
/// ⊘ Its operands are `Representability::Untracked` rather than `HostBacked`, because
/// nothing published them into the core's table — and `Untracked` forwards, by design
/// (*"MISS = FAULT is about resolving, and we are not resolving it"*). So arm 2 says
/// nothing about the address plane; arm 1 does. Neither substitutes for the other, which
/// is why both run and both are asserted.
///
/// ⚠ **One test function, not two.** GPU work on this bench is strictly serial and libtest
/// runs test functions in parallel threads; two `#[test]`s here would put two submissions
/// on one GPU at once.
///
/// ★ Every failure prints the whole [`CeEvidence`], because *which* conjunct failed is the
/// diagnosis: bytes that never changed is a different fault from bytes that changed with no
/// semaphore, which is a different fault from a truncated copy.
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

    let mut vmm = kayfabe_mocks::MockVmm::new();
    let mut probe = HostRmBackend::new(
        IsolateId::new(0xE6, GPU),
        Arc::clone(&conn),
        Arc::new(kayfabe_isolate_host::ChildExports::new()),
    );

    // =================================================================================
    // ARM 1 — the product path
    // =================================================================================
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

    // ⊘ The refusal that shapes this whole file, asserted rather than narrated — a
    // published backing is NOT CPU-mappable. `[measured]` 2026-08-03 on the RTX 3060 bench
    // at rev `5ac789d`, run `e6_hw_join`, where the first draft of this file died here.
    let opaque = probe.fill_words(dst.memory, COPY_LEN, SENTINEL, 0);
    assert!(
        opaque.is_err(),
        "★ a published backing must stay opaque to the CPU — `alloc_sysmem` passes \
         NVOS02_FLAGS_MAPPING_NO_MAP on purpose, and this assertion is what makes arm 2 \
         necessary rather than optional. Got {opaque:?}"
    );

    submit_guest_ring(
        &dev,
        &mut vmm,
        pid,
        cid,
        (SRC_VA, DST_VA),
        "arm 1",
        |parsed| {
            assert_eq!(
                parsed.ce_spans[0].dst_kind,
                kayfabe_fwd::Representability::HostBacked,
                "★ the published destination resolves as HOST-BACKED — this is the arm that \
             says the address plane did its job"
            );
        },
    );
    let (submit1, payload1) = witness
        .latest()
        .expect("★ arm 1 reached RmBackend::ce_copy, so a submission was observed");
    assert!(
        submit1.landed(payload1),
        "★★★ ARM 1 FAILED: the guest's copy over its own PUBLISHED ranges did not retire \
         on the host GPU. {submit1:?} want payload {payload1:#010x} — GP_GET not meeting \
         GP_PUT means the entry was never fetched; a payload mismatch means it was fetched \
         and the methods did not release."
    );

    // =================================================================================
    // ARM 2 — the same join, over operands a CPU can see
    // =================================================================================
    let host_vas = dev
        .with_proc(pid, |p| p.vases[&(GPU, PDB)].host_vas)
        .expect("live")
        .expect("arm 1's publish materialized the channel's host VAS");
    let p_src = probe
        .alloc_probe_local(COPY_LEN)
        .expect("a CPU-mappable source");
    let p_dst = probe
        .alloc_probe_local(COPY_LEN)
        .expect("a CPU-mappable destination");
    probe
        .map_gpu_va(host_vas, p_src, COPY_LEN, PROBE_SRC_VA)
        .expect("the probe source maps into the CHANNEL'S OWN host VAS at its guest VA");
    probe
        .map_gpu_va(host_vas, p_dst, COPY_LEN, PROBE_DST_VA)
        .expect("the probe destination maps into the same host VAS at its guest VA");

    probe
        .fill_words(p_src, COPY_LEN, PATTERN, 1)
        .expect("the source is filled with a ramp");
    probe
        .fill_words(p_dst, COPY_LEN, SENTINEL, 0)
        .expect("the destination is filled with the sentinel");
    let before = probe
        .read_words_independently(p_dst, COPY_LEN, &[0])
        .expect("the destination reads back before the copy")[0];
    assert_eq!(
        before, SENTINEL,
        "★ non-vacuity: the destination provably did not already hold the answer"
    );

    submit_guest_ring(
        &dev,
        &mut vmm,
        pid,
        cid,
        (PROBE_SRC_VA, PROBE_DST_VA),
        "arm 2",
        |_| {},
    );

    // ---- ★★★ THE PREDICATE. Not a re-derivation of R17's — R17's.
    let (submit, payload) = witness
        .latest()
        .expect("★ arm 2 reached RmBackend::ce_copy, so a submission was observed");
    let after = probe
        .read_words_independently(p_dst, COPY_LEN, &[0, (WORDS - 1) * 4])
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
        // ⊘ INHERITED RED, fixed 2026-08-13 (w291). `CeEvidence` gained these two fields at
        // `7c63733` (w289) and this test was never updated, so `cargo check --all-targets`
        // has been failing on master's path ever since while `--workspace --lib` stayed
        // green. ⚠ A red gate nobody owns becomes a gate nobody reads.
        // ★ The VAs the probe actually mapped, not placeholders: these are what the fixture
        // asked `map_gpu_va` for, so a future failure prints the addresses it was about.
        src_va: PROBE_SRC_VA.0,
        dst_va: PROBE_DST_VA.0,
    };
    // ★★★★★ **THE FOUR FACTS, EACH ASSERTED ON ITS OWN — w296, and the gap it closes was
    // measured on a real GA106 one plane over.**
    //
    // ⊘⊘ This arm accepted on [`CeEvidence::copied`] alone until 2026-08-14. That predicate
    // is **three** of the four facts the bar states — bytes moved ∧ destination correct ∧
    // semaphore == declared — and it **never compares the cursors**. `[measured 2026-08-13,
    // boot `w283c_client`, real GA106]` the rm-ladder client printed *"engine semaphore
    // 0x00000001 … **GP_GET 0 caught GP_PUT 1**"* and returned `R33_RC=0`: `0` did not catch
    // `1`, the word *"caught"* was template text, and `copied()` was the predicate behind it.
    // `rmladder.rs` was corrected to [`CeEvidence::met_the_whole_bar`] (`:2608, :2615`) and
    // **this file, the only real-GPU test in the suite, was not** — the correction reached
    // the diagnostic and stopped one file short of the gate.
    //
    // ★ Asserted SEPARATELY and before the conjunction, so a failure names WHICH fact went:
    // a truncated copy, a copy that never retired and an entry that was never fetched are
    // three different investigations, and one `false` from a four-way `&&` starts none of
    // them. ⊘ The conjunction is still asserted afterwards — `met_the_whole_bar()` is the
    // production predicate, and a test that only checked the parts could drift from it.
    assert_ne!(
        evidence.before, evidence.expect_after,
        "★ FACT 1 — NON-VACUITY: the destination already held the answer, so nothing below \
         can distinguish a copy from a no-op. {evidence:?}"
    );
    assert_eq!(
        (evidence.after, evidence.after_last),
        (evidence.expect_after, evidence.expect_after_last),
        "★ FACT 2 — THE BYTES: first word and last, through an INDEPENDENT mapping. `after` \
         unchanged means the engine moved nothing; `after` right with `after_last` wrong \
         means the copy was TRUNCATED. {evidence:?}"
    );
    assert_eq!(
        evidence.submit.semaphore, evidence.payload,
        "★ FACT 3 — THE RELEASE: the engine's own report semaphore does not carry the \
         declared payload, so the methods never retired — whatever the bytes say. \
         {evidence:?}"
    );
    assert!(
        evidence.cursor_caught_up(),
        "★★★ FACT 4 — THE CURSOR, and it is the one this assertion existed for. \
         `GP_GET {} != GP_PUT {}`: the GPU's host unit did not consume the entry we \
         published. ⊘ On a native arm the two agree, which is exactly why an acceptance \
         built on `copied()` looked green for a year and was blind precisely where it \
         mattered. {evidence:?}",
        evidence.submit.gp_get,
        evidence.submit.gp_put,
    );
    assert!(
        evidence.met_the_whole_bar(),
        "★★★ E6 ACCEPTANCE FAILED on the CONJUNCTION while every part passed — which means \
         `met_the_whole_bar` and the four assertions above have drifted apart and one of \
         them is no longer the bar. {evidence:?}"
    );
    gate_line(&format!(
        "GPU-GATE: E6 ACCEPTANCE CeEvidence::met_the_whole_bar() == true (all FOUR facts, \
         cursors included) — {evidence:?}"
    ));
}

/// One guest ring, scripted, bound and submitted through the L1 shell — the whole of
/// `submit_ring`. `extra` gets the parse's outcome so each arm can assert what is
/// specifically ITS claim.
fn submit_guest_ring(
    dev: &SharedDevice,
    vmm: &mut kayfabe_mocks::MockVmm,
    pid: ProcId,
    cid: ChanId,
    // ⊘ Grouped rather than two parameters: a copy has two ends and they are one fact.
    (src, dst): (GpuVa, GpuVa),
    arm: &str,
    extra: impl FnOnce(&kayfabe_fwd::PushbufferOutcome),
) {
    let (ring, method_bytes) = ga10x_ring(vmm, &ce_runs(src.0, dst.0, COPY_LEN as u32));
    dev.with_proc_mut(pid, |p| {
        let chan = p.channels.get(&cid).expect("the channel");
        let key = (chan.gpu, chan.vas_pdb.expect("it declares a VAS"));
        let vas = p.vases.get_mut(&key).expect("the VAS");
        kayfabe_tests::bind_ring_in(vas, RING_VA, RING_GPA, method_bytes);
    })
    .expect("live");

    let (parsed, forwarded) = dev
        .submit_ring(vmm, pid, cid, &ring)
        .unwrap_or_else(|e| panic!("[{arm}] the join must run, got {e:?}"));
    assert_eq!(
        parsed.ce_spans.len(),
        1,
        "[{arm}] one contiguous copy over two mapped ranges is one sub-copy, {:?}",
        parsed.ce_spans
    );
    assert_eq!(
        (parsed.ce_spans[0].sub.dst, parsed.ce_spans[0].sub.len),
        (dst.0, COPY_LEN),
        "[{arm}] ★ the instruction carries the GUEST's own destination and length"
    );
    assert_eq!(
        parsed.ce_spans[0].sub.by,
        CeExecutor::HostCe,
        "[{arm}] ★ the plan chose a REAL engine"
    );
    assert_eq!((forwarded.host_ce, forwarded.ours), (1, 0), "[{arm}]");
    extra(&parsed);
}
