//! CATEGORY C — SUSTAINED "LLM-LIKE" SOAK (the max integration without a GPU).
//!
//! A long-running virtual workload driven ENTIRELY through the mock arch/vmm/rm — the
//! full logic core under realistic sustained load, mimicking LLM token generation.
//! This is the deterministic, GPU-free analog of "run an LLM and watch it stay
//! correct for thousands of tokens" (`mode2_rust_testing_strategy.md` §9; the R5
//! never-reaped-table / L10 cross-teardown leak classes).
//!
//! Shape per "token" iteration, per inference process:
//!   * a few KV-cache-like allocations (a growing/rotating working set),
//!   * a doorbell ring (launch) on the process's GR channel,
//!   * a completion observe + poll-driven delivery (sync),
//!   * a periodic free of the oldest KV buffer (rotation).
//!
//! MULTIPLE concurrent inference processes (3) run interleaved, sustaining the #14
//! collision shape: they use the IDENTICAL guest VA base and IDENTICAL guest RM
//! handles, distinct PDBs. Invariants are asserted EVERY iteration and at the end.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::ids::GpuId;
use std::collections::BTreeSet;
use std::time::Duration;

use kayfabe_arch::ids::{GpuVa, HClient, Pdb, VChid};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{handle_doorbell, poll_completions, publish_backing, resolve};
use kayfabe_mocks::{MockArch, MockIsolateFactory, MockVmm};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

/// One virtual inference process's fixed identity + its rotating KV working set.
struct Inference {
    pdb: Pdb,
    gr_vchid: VChid,
    /// The guest VAs of live KV buffers (a bounded ring — rotation keeps it small).
    live_kv: Vec<GpuVa>,
    /// Monotonic slot counter → distinct guest VAs within this proc's own VA plane.
    next_slot: u64,
    /// Total completions this proc has had delivered (starvation check).
    delivered: u64,
}

/// The IDENTICAL guest VA base every process reuses (the #14 collision shape: two
/// procs' identical guest VAs must never collide — disjoint per-Vas backing).
const KV_VA_BASE: u64 = 0x2_0020_0000;
/// KV buffer size (64 KiB — a plausible per-token KV slice).
const KV_LEN: u64 = 0x10000;
/// How many KV buffers each process keeps live before rotating (bounded working set).
const KV_RING: usize = 8;

impl Inference {
    /// The guest VA for a given slot — IDENTICAL across processes for equal slots
    /// (that is the point: identical guest VAs, disjoint backing by construction).
    fn va_for_slot(slot: u64) -> GpuVa {
        GpuVa(KV_VA_BASE + (slot % 64) * KV_LEN)
    }
}

/// Build the soak fixture: `n` inference processes, each a compute process with
/// IDENTICAL handles + IDENTICAL VA plane, DISTINCT PDBs and vChids.
fn build(
    n: u64,
) -> (
    Guarded<Gpu>,
    MockVmm,
    Vec<Inference>,
    Vec<kayfabe_core::ProcId>,
) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    // Big window / 16-GiB arenas: a real soak must not exhaust the bump allocator
    // (no recycling yet — gpa.rs). 3 procs × ~1000 iters × 64 KiB ≈ 192 MiB each.
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x4_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

    let mut infs = Vec::new();
    let mut s = Scenario::new();
    for i in 0..n {
        // IDENTICAL handles across procs (the #14 shape); distinct PDB + vChids.
        let client = HClient(0xA000 + i as u32);
        let pdb = Pdb(0x3400_000 + i * 0x1000);
        let gr_vchid = 0x100 + (i as u16) * 2;
        let ce_vchid = 0x101 + (i as u16) * 2;
        s.compute_process(client, pdb, identical_handles(gr_vchid, ce_vchid));
        infs.push(Inference {
            pdb,
            gr_vchid: VChid(gr_vchid),
            live_kv: Vec::new(),
            next_slot: 0,
            delivered: 0,
        });
    }
    for ev in s.events {
        gpu.apply(ev).expect("fixture applies");
    }
    let pids: Vec<_> = infs
        .iter()
        .map(|inf| *gpu.spine.by_pdb.get(&(GpuId::ZERO, inf.pdb)).unwrap())
        .collect();
    (
        Guarded::new("soak_llm_like::build", gpu, rec),
        MockVmm::new(),
        infs,
        pids,
    )
}

/// Assert the cross-cutting invariants that must hold on EVERY iteration.
fn assert_soak_invariants(gpu: &Gpu, infs: &[Inference], pids: &[kayfabe_core::ProcId], n: u64) {
    // (1) Boundaries stay consistent: exactly `n` live procs, each still routed.
    assert_eq!(
        gpu.procs.len() as u64,
        n,
        "proc count stays bounded (no leak/dup)"
    );

    // (2) No arena collision: all live arenas pairwise disjoint.
    let ranges: Vec<_> = gpu
        .procs
        .values()
        .map(|p| p.arenas[&GpuId::ZERO].range.clone())
        .collect();
    for i in 0..ranges.len() {
        for j in (i + 1)..ranges.len() {
            assert!(
                ranges[i].end <= ranges[j].start || ranges[j].end <= ranges[i].start,
                "arena collision between live procs"
            );
        }
    }

    // (3) No backing collision across procs sharing an identical guest VA: each
    // proc resolves the SAME VA to ITS OWN distinct host backing.
    let shared = GpuVa(KV_VA_BASE);
    let mut seen_phys: BTreeSet<u64> = BTreeSet::new();
    let mut seen_hva: BTreeSet<u64> = BTreeSet::new();
    for inf in infs {
        if let Ok((bind, _)) = resolve(gpu, GpuId::ZERO, inf.pdb, shared) {
            assert!(
                seen_phys.insert(bind.phys),
                "two procs' identical VA share a GPA backing"
            );
            if let Some(hva) = bind.host_va() {
                assert!(
                    seen_hva.insert(hva),
                    "two procs' identical VA share a host VA"
                );
            }
        }
    }

    // (4) RmGraph does NOT leak: node count is exactly the fixture's (7 nodes/proc:
    // client, device, vaspace, tsg, gr-chan, ce-chan — SetPageDir isn't a node).
    let node_count = gpu.spine.rmgraph.nodes().count() as u64;
    assert_eq!(
        node_count,
        n * 6,
        "RmGraph node count bounded — no unbounded growth"
    );
    let _ = pids;
}

/// Run the soak for `iters` token iterations across `n` concurrent inference procs.
/// Returns the total completions delivered per proc (for the starvation assertion).
fn run_soak(iters: u64, n: u64) -> Vec<u64> {
    let (mut gpu, mut vmm, mut infs, pids) = build(n);
    // A well-formed doorbell token per proc's GR channel.
    let tokens: Vec<u64> = infs
        .iter()
        .map(|inf| MockArch::token_for(inf.gr_vchid))
        .collect();

    for it in 0..iters {
        // Interleave all procs each iteration (concurrent load).
        for idx in 0..n as usize {
            let pid = pids[idx];

            // --- KV-cache growth: allocate a fresh KV buffer at this proc's slot ---
            let slot = infs[idx].next_slot;
            infs[idx].next_slot += 1;
            let va = Inference::va_for_slot(slot);

            // If this exact VA is still live (ring wrap), eager-unbind first
            // (unmap eager, map lazy) — never a silent stale reuse.
            //
            // ★ §12.35: through `unpublish_and_release`, NOT a raw `table.unbind`. The
            // raw unbind abandoned this slot's host memory object and its GPU mapping
            // every single iteration — 19 992 of each by the end of the 20k-token soak,
            // on a proc that never dies, so §7.0's backstop never fires. Nothing was
            // looking: this suite's assertions are about hangs, GPA exhaustion and
            // routing.
            {
                let p = gpu.procs.get_mut(&pid).unwrap();
                if p.vases
                    .get(&(GpuId::ZERO, infs[idx].pdb))
                    .unwrap()
                    .table
                    .resolve(infs[idx].pdb, va)
                    .is_ok()
                {
                    kayfabe_tests::unpublish_and_release(p, GpuId::ZERO, infs[idx].pdb, va)
                        .expect("the ring-wrapped slot unpublishes");
                }
            }
            let p = gpu.procs.get_mut(&pid).unwrap();
            publish_backing(p, GpuId::ZERO, infs[idx].pdb, va, KV_LEN)
                .unwrap_or_else(|e| panic!("iter {it} proc {idx}: KV alloc failed: {e:?}"));
            infs[idx].live_kv.push(va);

            // --- rotate: free the oldest KV buffer once the ring is full ---
            if infs[idx].live_kv.len() > KV_RING {
                let old = infs[idx].live_kv.remove(0);
                let p = gpu.procs.get_mut(&pid).unwrap();
                // ★ §12.35 — the rotation is where the leak actually lived: the oldest KV
                // buffer left the ring every iteration and took a host object with it.
                kayfabe_tests::unpublish_and_release(p, GpuId::ZERO, infs[idx].pdb, old)
                    .expect("the rotated-out KV slot unpublishes");
            }

            // --- launch: ring this proc's GR doorbell (its own channel/isolate) ---
            let out =
                handle_doorbell(&mut gpu, GpuId::ZERO, tokens[idx], &[]).expect("launch routes");
            assert_eq!(out.proc, pid, "doorbell routes to the owning proc");

            // --- completion observed for this proc (host sema advance) ---
            gpu.procs
                .get_mut(&pid)
                .unwrap()
                .completion
                .observe(OsEventRef(it * 100 + idx as u64))
                .unwrap();
        }

        // --- sync: each proc polls (MC_SERVICE_INTERRUPTS) and its completions are
        // delivered off its OWN poll — the no-starvation property, sustained. A poll
        // composes ONE batch that MAY carry other procs' pending too (cross-proc
        // batching, no serialization); we attribute each delivered event to its owner
        // (encoded owner = ev % 100) so per-proc progress is measured exactly. ---
        vmm.advance(Duration::from_micros(1));
        for idx in 0..n as usize {
            if let Some(batch) = poll_completions(&mut gpu, &mut vmm, GpuId::ZERO, pids[idx]) {
                gpu.completions_drained(GpuId::ZERO);
                // Ack every delivered event on its OWNING proc's queue (keeps each
                // proc's awaiting_ack bounded — a leak would grow it unboundedly).
                for ev in &batch.events {
                    let owner = (ev.0 % 100) as usize;
                    infs[owner].delivered += 1;
                    gpu.procs.get_mut(&pids[owner]).unwrap().completion.ack(*ev);
                }
            }
        }

        // --- invariants EVERY iteration ---
        assert_soak_invariants(&gpu, &infs, &pids, n);
    }

    // End-state: every proc's completion queue is fully drained (no leak), and each
    // proc made forward progress every iteration (no starvation over thousands).
    for (idx, inf) in infs.iter().enumerate() {
        let q = &gpu.procs.get(&pids[idx]).unwrap().completion;
        assert!(
            !q.has_outstanding(),
            "proc {idx}: completion queue drained at end (no leak)"
        );
        assert_eq!(
            inf.delivered, iters,
            "proc {idx}: got a completion EVERY iter (no starvation)"
        );
    }
    infs.iter().map(|i| i.delivered).collect()
}

/// The fast soak: 1000 token iterations × 3 concurrent inference processes, running
/// entirely in pure logic (sub-second). Asserts every cross-cutting invariant every
/// iteration and the drained/no-starve end state.
#[test]
fn soak_1000_tokens_3_concurrent_infer_procs() {
    let delivered = run_soak(1000, 3);
    assert_eq!(
        delivered,
        vec![1000, 1000, 1000],
        "all three procs generated 1000 tokens"
    );
}

/// A single sustained process (N=1 is the same code path as N≥2) over 1000 tokens —
/// the byte-identical single-process baseline the multi-proc case must not regress.
#[test]
fn soak_1000_tokens_single_proc_baseline() {
    let delivered = run_soak(1000, 1);
    assert_eq!(delivered, vec![1000]);
}

/// The longer soak variant: 20_000 tokens × 3 procs. Proves the
/// graph/arena/completion state stays bounded over a genuinely long run — the R5
/// never-reaped-table leak class would surface here as an arena exhaustion or an
/// unbounded node count.
///
/// Formerly `#[ignore]`d as "long" — but measured it is ~0.6 s debug (the pure
/// core really is that fast), so it now just runs, always. `#[ignore]` had made
/// it invisible: nothing ran it unless someone knew to pass `--ignored`. The
/// suite's actually-slow tests are gated on `KAYFABE_SLOW=1` instead
/// (`kayfabe_tests::skip_slow!`), which skips *loudly*.
#[test]
fn soak_20000_tokens_3_procs() {
    let delivered = run_soak(20_000, 3);
    assert_eq!(delivered, vec![20_000, 20_000, 20_000]);
}
