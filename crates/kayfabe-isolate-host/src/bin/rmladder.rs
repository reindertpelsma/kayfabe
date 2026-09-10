//! ★ The bring-up ladder, as a program — `host_execution_plane.md` §4.
//!
//! > *"The ladder is cheap and is written **before** the first bench run, not after — each
//! > rung naming what is attempted and what 'working' looks like, so a failure localises to
//! > a layer."*
//!
//! Every design in this project has been wrong about six times per stage against a
//! *cooperative* fixture. Meeting a real driver without a ladder means debugging several
//! failures at once with no idea which layer owns them. So this walks the rungs one at a
//! time, prints each outcome, and stops at the first one that does not work — with the rung
//! name in the message.
//!
//! It is deliberately a **separate binary from the isolate**. The isolate is a sandboxed
//! child that speaks a protocol; this is a diagnostic a human runs on a bench box, and
//! conflating the two would put a human-facing argument parser and a `println!` inside the
//! process that faces a hostile guest.
//!
//! ```text
//! $ kayfabe-rm-ladder --gpu 0
//! ```

use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa};
use kayfabe_isolate::{IsolateId, RmBackend, RmError};
use kayfabe_isolate_host::rm::{
    DeviceExportOutcome, FbViewJoin, HostRmBackend, OsDescSeed, RACE_FENCE_OFFSET, RmConnection,
    ViewCompare,
};
use kayfabe_linux_raw::DevDir;
use std::sync::Arc;

/// ★★★ R12 — **the concurrency measurement, against the real driver.**
///
/// `host_execution_plane.md` §2.0 left one question open and said only a real host could
/// answer it: does an isolate's worker pool buy **wire concurrency**, or only latency
/// isolation? Twelve tests in the suite assert the former.
///
/// A real RM verb cannot be parked on demand, so this measures **overlap** rather than
/// waiting on an edge: every verb records the interval it occupied, and we count how many
/// pairs of intervals from *different threads* intersect. Overlap is a positive fact —
/// counting zero of it across thousands of verbs is a much stronger statement than a
/// wall-clock ratio, and it cannot be explained away by a slow machine.
///
/// Two configurations, same total work:
///   - **one isolate, N workers** — N threads on ONE RM client;
///   - **N isolates, one worker each** — N threads on N RM clients.
fn concurrency(gpu: u32, threads: usize, verbs: usize) -> bool {
    use std::sync::Mutex;
    use std::time::Instant;

    /// Run `verbs` alloc/free pairs on each of `workers`, in parallel, and report how many
    /// pairs of intervals from different threads overlapped.
    fn measure(mut workers: Vec<kayfabe_isolate::Worker>, verbs: usize) -> (usize, u128) {
        let origin = Instant::now();
        let spans: Arc<Mutex<Vec<(usize, u128, u128)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();
        for (t, mut w) in workers.drain(..).enumerate() {
            let spans = Arc::clone(&spans);
            handles.push(std::thread::spawn(move || {
                for _ in 0..verbs {
                    let start = origin.elapsed().as_nanos();
                    let h = w.with_rm(
                        &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"),
                        |rm| rm.alloc_vaspace(),
                    );
                    let end = origin.elapsed().as_nanos();
                    spans.lock().expect("spans").push((t, start, end));
                    if let Ok(h) = h {
                        let _ = w.with_rm(
                            &kayfabe_util::trapwitness::OffTrap::claim(
                                "a test / adapter host verb",
                            ),
                            |rm| rm.free(h),
                        );
                    }
                }
                w
            }));
        }
        let done: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().expect("join"))
            .collect();
        drop(done);
        let spans = spans.lock().expect("spans").clone();
        let total = origin.elapsed().as_millis();
        let mut overlaps = 0;
        for (i, a) in spans.iter().enumerate() {
            for b in &spans[i + 1..] {
                if a.0 != b.0 && a.1 < b.2 && b.1 < a.2 {
                    overlaps += 1;
                }
            }
        }
        (overlaps, total)
    }

    let id = |p: u32| IsolateId::new(p, GpuId(gpu));

    // ★ (0) THE BASELINE, and without it the other two numbers cannot be read at all: one
    // worker doing ALL the work, sequentially. If (a) and (b) both match this, then no
    // amount of parallelism buys throughput and the bottleneck is device-global — which is
    // a completely different finding from "the pool does not help".
    let base_f = kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real)
        .with_pool_size(1);
    let mut base = kayfabe_isolate::IsolateFactory::spawn(&base_f, id(899));
    if base.is_retired() {
        println!("FAIL  R12 baseline         = it did not start");
        return false;
    }
    let Some(w) = base.checkout() else {
        println!("FAIL  R12 baseline         = no worker");
        return false;
    };
    let (_, t_base) = measure(vec![w], threads * verbs);

    // (a) ONE isolate, `threads` workers — one RM client.
    let f = kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real)
        .with_pool_size(threads);
    let mut one = kayfabe_isolate::IsolateFactory::spawn(&f, id(900));
    if one.is_retired() {
        println!("FAIL  R12 one-isolate      = it did not start");
        return false;
    }
    let ws: Vec<_> = (0..threads).filter_map(|_| one.checkout()).collect();
    if ws.len() != threads {
        println!("FAIL  R12 one-isolate      = only {} workers", ws.len());
        return false;
    }
    let (same_client, t_same) = measure(ws, verbs);

    // (b) `threads` isolates, one worker each — `threads` RM clients.
    let g = kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real)
        .with_pool_size(1);
    let mut many: Vec<_> = (0..threads)
        .map(|i| kayfabe_isolate::IsolateFactory::spawn(&g, id(910 + i as u32)))
        .collect();
    if many.iter().any(|i| i.is_retired()) {
        println!("FAIL  R12 many-isolates    = one did not start");
        return false;
    }
    let ws: Vec<_> = many.iter_mut().filter_map(|i| i.checkout()).collect();
    if ws.len() != threads {
        println!("FAIL  R12 many-isolates    = only {} workers", ws.len());
        return false;
    }
    let (many_clients, t_many) = measure(ws, verbs);

    let n = threads * verbs;
    println!("info  R12 {threads} threads x {verbs} verbs, alloc_vaspace + free");
    println!(
        "ok    R12 1 thread (base)  = {} verbs sequential, {t_base} ms",
        threads * verbs
    );
    println!("ok    R12 one client       = {same_client} overlapping pairs, {t_same} ms");
    println!("ok    R12 {threads} clients      = {many_clients} overlapping pairs, {t_many} ms");
    // ★ Speedup against the sequential baseline is the only reading that means anything.
    // "Overlapping intervals" counts the whole request/reply span — transport included —
    // so it can be non-zero while every ioctl is strictly serialised. Suspect the
    // instrument: the timing is the evidence, the overlap count is a hint.
    let sp = |t: u128| {
        if t == 0 {
            0.0
        } else {
            t_base as f64 / t as f64
        }
    };
    println!(
        "★     R12 SPEEDUP         = one client x{threads} workers: {:.2}x   |   {threads} clients: {:.2}x   (ideal {threads}.00x, {n} verbs)",
        sp(t_same),
        sp(t_many)
    );
    true
}

/// ★★★ R13b — **does `engineType` actually route?**
///
/// The first R13 run returned runlist 0 for a copy channel *and* for a graphics channel,
/// which is exactly what the C's proven `engineType = 0` bug looks like
/// (`dma_copy_class_alloc_params`, seam audit GR-1): wrong runlist, no error, failure three
/// steps later. Two readings fit that observation and they are opposite —
///
///   (a) the engine type is being ignored, or
///   (b) the first two copy engines really are on the graphics runlist on this part.
///
/// A single measurement cannot separate them, so this sweeps the engine type and reads the
/// runlist back out of the work-submit token. If the runlist never changes, (a). If it
/// changes with the engine type, (b) — and the runlist-0 result is a fact about the
/// hardware rather than a symptom.
///
/// It exists as a diagnostic rather than a test for the reason the whole file does: it
/// needs a GPU, and it *reports* a table rather than asserting one, because the table is
/// per-part.
fn engines(rm: &mut HostRmBackend, gpu: u32) {
    println!("info  R13b engine sweep   = NV2080_ENGINE_TYPE_COPY(i) -> runlist, on GPU {gpu}");
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..8u32 {
        let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(i) else {
            println!("info  R13b COPY({i})       = not expressible (past the macro's first arm)");
            continue;
        };
        let Ok(vas) = rm.alloc_vaspace() else {
            println!("FAIL  R13b COPY({i})       = no address space");
            return;
        };
        match rm.alloc_channel_on(vas, engine_type) {
            Ok((chan, token)) => {
                let runlist = (token >> 16) & 0xFFFF;
                println!(
                    "ok    R13b COPY({i})       = engineType {engine_type:#x} -> \
                     runlist {runlist} (token {token:#010x})"
                );
                seen.insert(runlist);
                let _ = rm.free(chan);
            }
            Err(e) => println!("info  R13b COPY({i})       = refused {e:?}"),
        }
        let _ = rm.free(vas);
    }
    // The verdict, stated as the disambiguation rather than as a pass: MORE THAN ONE
    // distinct runlist across the sweep is what rules out "the engine type is ignored".
    if seen.len() > 1 {
        println!(
            "★     R13b VERDICT        = {} DISTINCT runlists {:?} — engineType routes; \
             a copy channel on runlist 0 is this part's GRCE, not a wrong-runlist bug",
            seen.len(),
            seen
        );
    } else {
        println!(
            "★     R13b VERDICT        = every engine type produced runlist {:?} — \
             engineType is NOT routing, which is the wrong-runlist bug",
            seen
        );
    }
}

/// ★★★ R13c — **the doorbell-token census: what `(runlist, chid)` did RM put in there?**
///
/// This rung exists for increment **E3** (`execution_plane_increments.md` §2.1), whose
/// whole argument is that a wrong doorbell decode **cannot fail loudly** — we are the GSP
/// on the Mode-2 path, so a ring sent to the wrong channel has no second party to notice.
/// The two standing oracles are both blind to it: `MockArch::token_for` is the inverse of
/// the mock's own decode, and `c_rust_trace_differential.md` records that the completion
/// plane has **no** C oracle. So the expected value has to come from hardware, and — this
/// is the whole design of this rung — **from a part of hardware the token cannot have
/// leaked into**.
///
/// ## ⊘ Why R13/R13b's own `(runlist N chid M)` annotations are NOT that
///
/// They are `(token >> 16)` and `(token & 0xFFFF)` — the token restated. Printing them
/// beside the token and calling the pair an agreement is measuring nothing, and R13b's
/// verdict line leans on exactly that (it is still *sound* for what it claims — that the
/// upper field VARIES with `engineType` — and it is not sound as evidence about which
/// field is the runlist).
///
/// ## What this rung does instead
///
/// [`NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS`] takes a **`runlistId` as input** and
/// returns a bitmask of the chids allocated on it, walked out of that runlist's
/// `CHID_MGR` (`ogkm-580: kernel_fifo.c:3371-3443`). Snapshot every runlist, allocate one
/// channel, snapshot again: the bit that appeared **is** the `(runlist, chid)` RM's
/// allocator just handed out. Nothing in that path reads a work-submit token.
///
/// ★★ The before/after pair is the point, not the after-snapshot. A bitmask read once
/// cannot distinguish *"this channel is at chid 7"* from *"some channel is at chid 7"* —
/// the boolean-witness failure — and this box has an X server and a `nvidia-persistenced`
/// on it, so other channels genuinely do exist. A diff attributes the bit to **our**
/// allocation. If the diff is not exactly one bit, this rung says so and marks the sample
/// **AMBIGUOUS** rather than picking one.
///
/// ## Output contract
///
/// One `SAMPLE` line per channel, machine-readable, because a committed file is what the
/// suite can be keyed on (a gate keyed on a *word* is satisfied by writing the word):
///
/// ```text
/// SAMPLE engine_type=0x9 token=0x00000007 runlist=0 chid=7
/// ```
///
/// plus `SAMPLE-AMBIGUOUS` / `SAMPLE-REFUSED` lines that carry no `runlist=`/`chid=` and
/// are therefore unusable as evidence by construction.
fn doorbell_census(rm: &mut HostRmBackend, subdevice: kayfabe_isolate::HostHandle, gpu: u32) {
    use kayfabe_abi::submit::{
        ALLOCATED_CHANNELS_MAX, ALLOCATED_CHANNELS_PARAMS_SIZE, ENGINE_TYPE_GRAPHICS,
        FIFO_GET_INFO_PARAMS_SIZE, FIFO_INFO_INDEX_CHANNEL_GROUPS_IN_USE_PER_ENGINE,
        FIFO_INFO_INDEX_IS_PER_RUNLIST_CHANNEL_RAM_SUPPORTED,
        NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS, NV2080_CTRL_CMD_FIFO_GET_INFO,
        engine_type_copy,
    };

    /// How many runlist ids to sweep for the chid bitmask. `NV_CTRL_VF_DOORBELL_RUNLIST_ID`
    /// is 7 bits wide; an id past what this part supports answers `NV_ERR_OUT_OF_RANGE`
    /// (`ogkm-580: kernel_fifo.c:3392-3406`) — which is reported, so the sweep's end is
    /// measured and not assumed.
    const RUNLISTS: u32 = 24;

    /// One `NV2080_CTRL_CMD_FIFO_GET_INFO` entry. `engine_type` is a params-level field,
    /// so one call answers one index for one engine.
    fn fifo_info(
        rm: &mut HostRmBackend,
        subdevice: kayfabe_isolate::HostHandle,
        index: u32,
        engine_type: u32,
    ) -> Option<u32> {
        let mut p = vec![0u8; FIFO_GET_INFO_PARAMS_SIZE];
        p[0..4].copy_from_slice(&1u32.to_le_bytes()); // fifoInfoTblSize = 1
        p[4..8].copy_from_slice(&index.to_le_bytes()); // fifoInfoTbl[0].index
        let et = FIFO_GET_INFO_PARAMS_SIZE - 4;
        p[et..].copy_from_slice(&engine_type.to_le_bytes());
        rm.control(subdevice, ControlCmd(NV2080_CTRL_CMD_FIFO_GET_INFO), &mut p)
            .ok()?;
        Some(u32::from_le_bytes([p[8], p[9], p[10], p[11]])) // fifoInfoTbl[0].data
    }

    /// Read every runlist's allocated-chid bitmask. `None` for a runlist the control
    /// refused — recorded per-runlist so a refusal cannot masquerade as "no channels".
    fn snapshot(
        rm: &mut HostRmBackend,
        subdevice: kayfabe_isolate::HostHandle,
    ) -> Vec<Option<Vec<u32>>> {
        let mut out = Vec::new();
        for runlist in 0..RUNLISTS {
            let mut payload = vec![0u8; ALLOCATED_CHANNELS_PARAMS_SIZE];
            payload[..4].copy_from_slice(&runlist.to_le_bytes());
            match rm.control(
                subdevice,
                ControlCmd(NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS),
                &mut payload,
            ) {
                Ok(()) => out.push(Some(
                    payload[4..]
                        .chunks_exact(4)
                        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                        .collect(),
                )),
                Err(_) => out.push(None),
            }
        }
        out
    }

    /// Every `(runlist_probe_index, chid)` set in `after` and clear in `before`.
    fn appeared(before: &[Option<Vec<u32>>], after: &[Option<Vec<u32>>]) -> Vec<(u32, u32)> {
        let mut new = Vec::new();
        for (runlist, (b, a)) in before.iter().zip(after).enumerate() {
            let (Some(b), Some(a)) = (b, a) else { continue };
            for chid in 0..ALLOCATED_CHANNELS_MAX {
                let (w, bit) = (chid / 32, 1u32 << (chid % 32));
                let was = b.get(w).is_some_and(|v| v & bit != 0);
                let is = a.get(w).is_some_and(|v| v & bit != 0);
                if is && !was {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "runlist < RUNLISTS = 24 and chid < ALLOCATED_CHANNELS_MAX = 4096"
                    )]
                    new.push((runlist as u32, chid as u32));
                }
            }
        }
        new
    }

    println!(
        "info  R13c census        = RM's own chid manager vs the work-submit token, GPU {gpu}"
    );

    // ── The two facts that decide how to READ everything below ───────────────────────
    match fifo_info(
        rm,
        subdevice,
        FIFO_INFO_INDEX_IS_PER_RUNLIST_CHANNEL_RAM_SUPPORTED,
        ENGINE_TYPE_GRAPHICS,
    ) {
        Some(v) => println!("FACT per_runlist_channel_ram={v}"),
        None => println!("FACT per_runlist_channel_ram=refused"),
    }
    let probe = snapshot(rm, subdevice);
    let answered: Vec<usize> = probe
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.as_ref().map(|_| i))
        .collect();
    if answered.is_empty() {
        println!(
            "FAIL  R13c instrument    = every runlist refused NV2080_CTRL_CMD_FIFO_\
             GET_ALLOCATED_CHANNELS (it is PRIVILEGED — run as root); NO SAMPLES TAKEN"
        );
        return;
    }
    println!("FACT chid_namespaces={answered:?} of 0..{RUNLISTS}");

    // ★★ The instrument that would have named the runlist IDs outright, TRIED and its
    // refusal RECORDED. `NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE` (0x20801112) carries
    // `engineData[ENGINE_INFO_TYPE_RUNLIST]` per engine — exactly the number Part 2 below
    // has to approximate — and its flags are `0x5c040`, i.e. neither `PRIVILEGED` (0x4)
    // nor `NON_PRIVILEGED` (0x8), which is RM's `KERNEL_PRIVILEGED` default: refused to
    // every usermode client including root (`control.h:170-208`,
    // `g_subdevice_nvoc.c:4996`). Asking anyway costs one ioctl and converts *"we did not
    // measure the runlist id"* into *"we asked and RM refused, with this status"*.
    {
        // ★ The size IS load-bearing, and getting it wrong cost a run: a 12-byte payload
        // came back `NV_ERR_INVALID_ARGUMENT` (0x1F) from RM's paramSize check, which
        // reads exactly like a refusal and is not one. `NV2080_CTRL_FIFO_GET_DEVICE_INFO_
        // TABLE_PARAMS` is `NvU32 baseIndex`, `NvU32 numEntries`, `NvBool bMore` (padded
        // to 4), then 32 × `NV2080_CTRL_FIFO_DEVICE_ENTRY` of (16+2+2+1) `NvU32` and a
        // 16-byte name = 100 bytes: 12 + 3200.
        let mut p = vec![0u8; 12 + 32 * 100];
        let r = rm.control(subdevice, ControlCmd(0x2080_1112), &mut p);
        println!("FACT device_info_table={r:?}");
    }

    // The engine types this part will take a channel on. GR first: on this part CE0/CE1
    // are the graphics copy engines, so a sweep that skipped GR would never sample the
    // runlist they share.
    let mut engine_types: Vec<u32> = vec![ENGINE_TYPE_GRAPHICS];
    engine_types.extend((0..8u32).filter_map(engine_type_copy));

    // ── PART 1 — the token's LOW field, against RM's chid manager ────────────────────
    //
    // ★★ The channels are held SIMULTANEOUSLY, and that is the whole difference from the
    // first version of this rung: allocating and freeing one at a time returned chid 4
    // every time, so the sweep produced one chid value six times and pinned nothing. Held
    // together they take distinct chids, and a decoder that got the field WIDTH or the
    // shift wrong now has somewhere to be wrong.
    let mut held: Vec<(
        u32,
        kayfabe_isolate::HostHandle,
        kayfabe_isolate::HostHandle,
        u64,
    )> = Vec::new();
    for &engine_type in &engine_types {
        let Ok(vas) = rm.alloc_vaspace() else {
            println!("SAMPLE-REFUSED engine_type={engine_type:#x} reason=no-vaspace");
            continue;
        };
        let before = snapshot(rm, subdevice);
        match rm.alloc_channel_on(vas, engine_type) {
            Ok((chan, token)) => {
                let after = snapshot(rm, subdevice);
                match appeared(&before, &after).as_slice() {
                    [(ns, chid)] => println!(
                        "SAMPLE engine_type={engine_type:#x} token={token:#010x} \
                         chid={chid} chid_namespace={ns}"
                    ),
                    other => println!(
                        "SAMPLE-AMBIGUOUS engine_type={engine_type:#x} token={token:#010x} \
                         appeared={other:?} — the diff is not one bit, so this allocation \
                         cannot be attributed and is NOT evidence"
                    ),
                }
                held.push((engine_type, chan, vas, token));
            }
            Err(e) => {
                println!("SAMPLE-REFUSED engine_type={engine_type:#x} reason={e:?}");
                let _ = rm.free(vas);
            }
        }
    }
    for (_, chan, vas, _) in held.drain(..) {
        let _ = rm.free(chan);
        let _ = rm.free(vas);
    }

    // ── PART 2 — the token's UPPER field, against the engine→runlist PARTITION ───────
    //
    // ★★★ The runlist *ids* are not readable by an unprivileged client
    // (`NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE` is KERNEL_PRIVILEGED), so this does
    // not try to read one. It measures the **partition** instead:
    // `CHANNEL_GROUPS_IN_USE_PER_ENGINE` translates an engine type to its runlist inside
    // RM and returns that runlist's group count, so allocating ONE channel on engine X
    // raises the count for exactly the engines that share X's runlist.
    //
    // That is an equivalence relation on engines, derived with no reference to a token —
    // and the token's upper field must induce the SAME relation, or the field is not the
    // runlist. It is a weaker statement than reading the id, and it is the strongest one
    // this box can make; §4 of `doorbell_token_encoding.md` says so rather than implying
    // the id was measured.
    for &x in &engine_types {
        let counts = |rm: &mut HostRmBackend| -> Vec<Option<u32>> {
            engine_types
                .iter()
                .map(|&e| {
                    fifo_info(
                        rm,
                        subdevice,
                        FIFO_INFO_INDEX_CHANNEL_GROUPS_IN_USE_PER_ENGINE,
                        e,
                    )
                })
                .collect()
        };
        let Ok(vas) = rm.alloc_vaspace() else {
            continue;
        };
        let before = counts(rm);
        match rm.alloc_channel_on(vas, x) {
            Ok((chan, token)) => {
                let after = counts(rm);
                let members: Vec<String> = engine_types
                    .iter()
                    .zip(before.iter().zip(after.iter()))
                    .filter(|(_, (b, a))| match (b, a) {
                        (Some(b), Some(a)) => a > b,
                        _ => false,
                    })
                    .map(|(e, _)| format!("{e:#x}"))
                    .collect();
                println!(
                    "PARTITION engine_type={x:#x} token={token:#010x} members=[{}]",
                    members.join(",")
                );
                let _ = rm.free(chan);
            }
            Err(e) => println!("PARTITION-REFUSED engine_type={x:#x} reason={e:?}"),
        }
        let _ = rm.free(vas);
    }
    // ★★★ The verdict on the INSTRUMENT, printed by the instrument, because a reader who
    // sees the same member list under every engine must not have to work out whether that
    // means "one runlist" or "this control cannot see runlists". On a part with
    // `per_runlist_channel_ram=0` it is the latter, and it is structural:
    // `kfifoGetChidMgr` returns `ppChidMgr[0]` for EVERY runlist id in that configuration
    // (`ogkm-580: kernel_fifo.c:1457-1466`), so the per-engine count this rung diffs is
    // one global number. Read `partition_is_vacuous=1` as: this rung has **not measured**
    // anything at all about the token's upper field.
    println!(
        "FACT partition_is_vacuous={}",
        u8::from(
            fifo_info(
                rm,
                subdevice,
                FIFO_INFO_INDEX_IS_PER_RUNLIST_CHANNEL_RAM_SUPPORTED,
                ENGINE_TYPE_GRAPHICS,
            ) == Some(0)
        )
    );
    println!("info  R13c census        = done");
}

/// ★★★ R18 — **ask the real GPU what a control returns.**
///
/// Some numbers the emulated GSP has to answer are not derivable from anything we hold.
/// `NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE` (`0x20802a08`) is the case that
/// forced this rung: RM sizes the copy-engine **fault method buffer** from it, and a
/// plausible-but-wrong number is a buffer that real hardware writes past. The physical
/// handler is not in the open tree — `subdeviceCtrlCmdCeGetFaultMethodBufferSize_IMPL` is
/// declared in `g_subdevice_nvoc.h` and defined nowhere, because it is compiled into GSP
/// firmware — so **source cannot answer it and a guess is not allowed to.** A real part
/// can.
///
/// ## Why the buffer is seeded, and not zeroed
///
/// A control that returns `NV_OK` having written nothing is indistinguishable from one
/// that returned a legitimate zero if the buffer started at zero — and "the size is 0" is
/// exactly the answer that would send us back to the wall we started at. So every byte is
/// seeded with `0xCD` first, and the report says whether RM **touched** the buffer
/// separately from what it left there. An untouched buffer is reported as untouched.
///
/// ## What a refusal means here, and why it is still a result
///
/// A `KERNEL_PRIVILEGED` control (the RM default: `flags` carrying none of `PRIVILEGED`,
/// `NON_PRIVILEGED` or `INTERNAL` — `control.c:702`) is refused to every usermode client
/// including root. That refusal is **recorded, not worked around**: an
/// `InsufficientPermissions` printed here is the measurement, and it is a much better
/// artifact than a number nobody can source.
///
/// ⊘ `[measured]` 2026-08-01: for `0x20802a08` on an RTX 3060 that is exactly what happens,
/// so this rung did **not** supply the number in `kayfabe_abi::fmbsize` — an instrumented
/// build of the driver did. The rung is kept because the refusal is itself the recorded
/// result, and because the next control may not be kernel-privileged.
fn probe_ctrl(
    rm: &mut HostRmBackend,
    subdevice: kayfabe_isolate::HostHandle,
    specs: &[(u32, usize)],
) {
    println!(
        "info  R18 ctrl probe      = {} control(s) on the subdevice",
        specs.len()
    );
    for &(cmd, size) in specs {
        // The sentinel. `0xCD` in every byte is not a size, not a handle and not a status,
        // so it survives into the report as itself if RM never writes.
        let mut payload = vec![0xCDu8; size];
        let result = rm.control(subdevice, ControlCmd(cmd), &mut payload);
        let touched = payload.iter().any(|&b| b != 0xCD);
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        match result {
            Ok(()) if touched => {
                // Report the little-endian `NvU32` reading too — every control this rung
                // has needed so far leads with one, and the raw bytes stay printed so a
                // wider struct is still readable.
                let head = payload
                    .get(..4)
                    .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
                println!(
                    "★     R18 {cmd:#010x}    = NV_OK, {size} bytes: {hex}{}",
                    head.map_or(String::new(), |v| format!("   (u32[0] = {v} / {v:#x})"))
                );
            }
            Ok(()) => println!(
                "info  R18 {cmd:#010x}    = NV_OK but the buffer is UNTOUCHED ({size} bytes \
                 still {hex}) — an accepted call that answered nothing"
            ),
            Err(e) => println!("info  R18 {cmd:#010x}    = refused {e:?} (no value measured)"),
        }
    }
}

/// ★★★ R20 — **`NV2081_BINAPI` and its opaque control, asked of a real GA106.**
///
/// `execution_plane_increments.md` §14.26 named this rung by measurement: `0x20810108` has
/// **no oracle**. There is no row for it in the C artifact's captured control table, `cap1`
/// carries the *request* and never a reply, and `traces/real_ga106/` does not mention it —
/// and the three failures share **one** cause, because every one of those instruments was
/// produced by driving `RmInitAdapter` with `nvidia-smi`, while `0x20810108` is issued by
/// **libcuda**. ⊘ Three instruments agreeing is not corroboration when they share the defect.
///
/// Source cannot answer it either: `binapiControl_IMPL` does not interpret `pParams->cmd`
/// at all, it forwards the whole command to GSP over `NV_RM_RPC_API_CONTROL`
/// (`ogkm-580: src/nvidia/src/kernel/rmapi/binary_api.c:61-127`), and GSP-RM is in no
/// vendored tree. So the only remaining instrument is a real part.
///
/// ## ★★ Why this rung exists even though `cuda_ioctl_trace.c` already traced libcuda
///
/// The interposed trace of a real `cuInit` recorded `0x20810108` as **992 bytes in, 992
/// bytes out, `NV_OK`, and every byte zero on both sides** — because libcuda hands RM a
/// zeroed buffer. ⊘ That measurement **cannot distinguish** "GSP wrote 992 zeros" from "GSP
/// returned `NV_OK` and wrote nothing", and those are different facts with different
/// consequences for what our emulated GSP must put on the wire.
///
/// This rung separates them the only way they can be separated: **seed the buffer with
/// `0xCD` first**, exactly as R18 does and for exactly R18's reason. A buffer that comes
/// back zeroed was written. A buffer that comes back `0xCD` was not. An interposer must not
/// modify what it observes; a ladder is free to.
///
/// ⚠ The `0x2081` alloc is issued the way libcuda measurably issues it — **`paramsSize=0`
/// and a NULL params pointer**, not the 4-byte `NV2081_ALLOC_PARAMETERS`. RM's own RPC to
/// GSP then carries `paramsSize=4` because `RS_OPTIONAL(NV2081_ALLOC_PARAMETERS)`
/// (`resource_list.h:444`) declares that size for the *registered* class; the guest-side
/// wire we must answer and the client-side ioctl we must imitate are **not the same
/// number**, and mistaking one for the other is how a decoder ends up demanding a body no
/// client ever sends.
fn binapi_probe(
    rm: &mut HostRmBackend,
    subdevice: kayfabe_isolate::HostHandle,
    specs: &[(u32, usize)],
) {
    println!("info  R20 binapi probe    = alloc NV2081_BINAPI under the subdevice");

    // ⚠ Empty params. RS_OPTIONAL means a NULL is legal BY DECLARATION
    // (`resource_desc.c:76` expands it to `bParamRequired = NV_FALSE`), and it is what a
    // real libcuda sends — measured, not assumed.
    let binapi = match rm.alloc(subdevice, ClassId(0x2081), &[]) {
        Ok(h) => {
            println!(
                "★     R20 hBinApi        = {:#010x} (NV_OK, params NULL/0)",
                h.raw()
            );
            h
        }
        Err(e) => {
            // A refusal here is itself the result, and it localises: it says an
            // unprivileged-flagged class under a Subdevice was still denied to this client,
            // which would make every reply body below unobtainable rather than unknown.
            println!("info  R20 NV2081_BINAPI  = refused {e:?} — no control could be probed");
            return;
        }
    };

    for &(cmd, size) in specs {
        let mut payload = vec![0xCDu8; size];
        let result = rm.control(binapi, ControlCmd(cmd), &mut payload);
        let untouched = payload.iter().all(|&b| b == 0xCD);
        let all_zero = payload.iter().all(|&b| b == 0);
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        match result {
            Ok(()) if untouched => println!(
                "info  R20 {cmd:#010x}    = NV_OK but the buffer is UNTOUCHED ({size} bytes \
                 still 0xCD) — an accepted call that answered nothing"
            ),
            Ok(()) if all_zero => println!(
                "★     R20 {cmd:#010x}    = NV_OK and RM ZEROED all {size} bytes — the reply \
                 IS zeros, and the 0xCD seed is what proves it was written"
            ),
            Ok(()) => println!("★     R20 {cmd:#010x}    = NV_OK, {size} bytes: {hex}"),
            Err(e) => println!("info  R20 {cmd:#010x}    = refused {e:?} (no value measured)"),
        }
    }

    match rm.free(binapi) {
        Ok(()) => println!("ok    R20 free           = NV_OK"),
        Err(e) => println!("info  R20 free           = {e:?}"),
    }
}

/// ★★★ R21 — **every `GPU_GET_INFO_V2` index, asked of a real GA106, ONE AT A TIME.**
///
/// `execution_plane_increments.md` §14.28. The increment that needed this was handed a
/// table of **eleven** `(index, value)` rows read off an interposed `cuInit`, and that
/// table is at the **ioctl** boundary — which is the wrong boundary for this port by one
/// layer, and the error is not conservative.
///
/// ⊘ **Ten of those eleven never reach a GSP.** `getGpuInfos`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:88-580`)
/// answers thirty-two indices from **kernel** state and forwards only the `default:` arm,
/// marking each forwarded entry by OR-ing `INDEX_FORWARD_TO_PHYSICAL` (`0x8000_0000`,
/// `:83`, ct_asserted equal to `NV2080_CTRL_GPU_INFO_INDEX_RESERVED` = bit 31) into the
/// index word. A port that answers all eleven from the ioctl table is answering ten
/// questions it was never asked, from values the guest kernel had already written.
///
/// ## Why the sweep is one call per index
///
/// `getGpuInfos` **breaks out of its loop on the first non-`NV_OK` status** (`:566-569`) and
/// returns it for the whole call. A 70-index request therefore measures *"the first index
/// that fails"* and nothing after it. One index per call makes each answer independent, and
/// makes a refusal attributable to the index that earned it.
///
/// ## What this rung is and is not an oracle for
///
/// ★ For an index the kernel **forwards**, the value RM hands back to usermode is the value
/// GSP-RM produced — the kernel copies the RPC reply straight over its own params — so this
/// rung IS the oracle for exactly the rows this port has to serve.
/// ⊘ For an index the kernel **resolves itself**, the value here is the *guest kernel's*
/// and says nothing about what a GSP would answer. Those rows are printed with a `KERNEL`
/// mark and must not be copied into the port's table. Which set an index is in is a clean
/// read of the open switch above; it is not measured here and is not guessable from a
/// reply.
///
/// ⚠ The unused tail of the 564-byte struct is seeded `0xCD` so *"RM wrote back the whole
/// params"* separates from *"RM wrote back the entries I declared"* — R18's reason.
fn gpu_info_sweep(rm: &mut HostRmBackend, subdevice: kayfabe_isolate::HostHandle) {
    // `ogkm-580: ctrl2080gpu.h:122` — NV2080_CTRL_GPU_INFO_MAX_LIST_SIZE.
    const MAX_LIST: u32 = 0x46;
    // `4 + 8 * 0x46`, and confirmed on the wire as `size=564` in the real-GA106 cuInit
    // trace (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:42`).
    const PARAMS: usize = 4 + 8 * (MAX_LIST as usize);
    const CMD: u32 = 0x2080_0102;

    println!(
        "info  R21 gpuinfo sweep   = {MAX_LIST} indices x 1 call each, {PARAMS}-byte params, \
         tail seeded 0xCD"
    );
    println!("info  R21 legend          = idx status data   (the port serves FORWARDED rows only)");

    for index in 0..MAX_LIST {
        let mut params = vec![0xCDu8; PARAMS];
        // gpuInfoListSize = 1, then one NV2080_CTRL_GPU_INFO { index, data }.
        params[0..4].copy_from_slice(&1u32.to_le_bytes());
        params[4..8].copy_from_slice(&index.to_le_bytes());
        params[8..12].copy_from_slice(&0u32.to_le_bytes());
        let result = rm.control(subdevice, ControlCmd(CMD), &mut params);
        let echoed = u32::from_le_bytes([params[4], params[5], params[6], params[7]]);
        let data = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
        let tail_written = params[12..].iter().any(|&b| b != 0xCD);
        match result {
            Ok(()) => println!(
                "★     R21 {index:#04x}  NV_OK      data={data:#010x} ({data})  echo={echoed:#010x}\
                 {}",
                if tail_written {
                    "  tail=WRITTEN"
                } else {
                    "  tail=untouched"
                }
            ),
            Err(e) => println!("info  R21 {index:#04x}  refused    {e:?}"),
        }
    }

    // ★ The control experiment: libcuda's OWN eleven-index request, byte for byte off the
    // interposed trace, issued as one call. If this reproduces the trace's `out=` line then
    // the per-index sweep above and the capture are measuring the same machine, and any
    // disagreement between them is a fact about the *request shape* rather than about the
    // instrument.
    const LIBCUDA_INDICES: [u32; 11] = [
        0x11, 0x22, 0x27, 0x2a, 0x37, 0x3b, 0x3c, 0x3d, 0x2d, 0x3a, 0x44,
    ];
    let mut params = vec![0u8; PARAMS];
    params[0..4].copy_from_slice(&(LIBCUDA_INDICES.len() as u32).to_le_bytes());
    for (i, idx) in LIBCUDA_INDICES.iter().enumerate() {
        params[4 + 8 * i..8 + 8 * i].copy_from_slice(&idx.to_le_bytes());
    }
    match rm.control(subdevice, ControlCmd(CMD), &mut params) {
        Ok(()) => {
            let pairs: String = (0..LIBCUDA_INDICES.len())
                .map(|i| {
                    let idx = u32::from_le_bytes([
                        params[4 + 8 * i],
                        params[5 + 8 * i],
                        params[6 + 8 * i],
                        params[7 + 8 * i],
                    ]);
                    let data = u32::from_le_bytes([
                        params[8 + 8 * i],
                        params[9 + 8 * i],
                        params[10 + 8 * i],
                        params[11 + 8 * i],
                    ]);
                    format!("{idx:#x}={data} ")
                })
                .collect();
            println!("★     R21 libcuda 11     = NV_OK  {pairs}");
        }
        Err(e) => println!("FAIL  R21 libcuda 11     = refused {e:?} — the trace says NV_OK"),
    }
}

/// ★★★ R22 — **every `BUS_GET_INFO_V2` index, asked of a real GA106, ONE AT A TIME — and
/// `PCIE_GEN_INFO` asked REPEATEDLY, because the question is whether it holds still.**
///
/// `execution_plane_increments.md` §14.29 ends at a second `0x20801823` answered `0x56` by
/// this port, whose six indices are `0x0f 0x10 0x2c 0x2d 0x03 0x06`. Of those, **exactly
/// one is RPC-forwarded on a GSP client** — `0x2d` `PCIE_GEN_INFO`, the first case label of
/// `getBusInfos`'s `bSendRpc = IS_VIRTUAL(pGpu) || IS_GSP_CLIENT(pGpu)` group
/// (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus_ctrl.c:296-334`). The other five are
/// computed by the guest's own kernel and are **not this port's to write**, exactly as ten
/// of `GPU_GET_INFO_V2`'s eleven were not.
///
/// ## Why one call per index, again
///
/// `getBusInfos` forwards each entry through `kbusSendBusInfo` under
/// `NV_CHECK_OK_OR_RETURN` (`:333`), so a multi-entry request measures *"the first entry
/// that fails"* and nothing after it. One index per call makes each answer attributable.
///
/// ⚠ And note what `kbusSendBusInfo_IMPL` actually puts on the wire
/// (`ogkm-580: kern_bus.c:1065-1101`): a **fresh `NV2080_CTRL_BUS_GET_INFO_V2_PARAMS` with
/// `busInfoListSize = 1`** and the single entry copied into slot 0. So the *ioctl* the
/// guest issues is six entries and the *RPC* this port must answer is **one** — a second
/// place where the boundary the trace was read at is not the boundary the port serves.
///
/// ## ★ The question this rung exists to settle, with both predictions written down first
///
/// `[unmeasured before this run]` whether `0x2d` may be a chip-family row at all.
///
/// | hypothesis | prediction |
/// |---|---|
/// | **H1 — die constant.** `PCIE_GEN_INFO` describes the GPU. | every field is a property of GA106; the value is the same on every GA106 and never moves on one box. |
/// | **H2 — link state.** It describes the *link*, like `0x23`/`0x24` describe the *die*. | at least one field tracks the **current** negotiated speed, so the SAME part answers differently as the link trains up and down, and a value baked into a chip row is wrong on a different slot/riser/bifurcation. |
///
/// H2 predicts drift **on one box with no second part to rent**, which is why this rung
/// samples `0x2d` repeatedly and decodes it rather than printing one number. `nvidia-smi`
/// on the bench says `gen.gpumax=4 gen.max=3 gen.current=1` — three different generations
/// in one machine — so if any of the three is in the word, H1 is dead.
fn bus_info_sweep(rm: &mut HostRmBackend, subdevice: kayfabe_isolate::HostHandle) {
    // `ogkm-580: ctrl2080bus.h:341` — NV2080_CTRL_BUS_INFO_MAX_LIST_SIZE.
    const MAX_LIST: u32 = 0x34;
    // `4 + 8 * 0x34`, and confirmed on the wire as `size=420` in the real-GA106 cuInit
    // trace (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:44,46`).
    const PARAMS: usize = 4 + 8 * (MAX_LIST as usize);
    const CMD: u32 = 0x2080_1823;
    // `ogkm-580: ctrl2080bus.h:329`.
    const PCIE_GEN_INFO: u32 = 0x2d;

    /// Build a `NV2080_CTRL_BUS_GET_INFO_V2_PARAMS` from `(index, data)` pairs written
    /// verbatim — the `data` words included, so a request off the trace can be replayed
    /// with libcuda's own stale buffer contents rather than a tidied-up version of it.
    fn request(entries: &[(u32, u32)], seed: u8) -> Vec<u8> {
        let mut p = vec![seed; PARAMS];
        p[0..4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        for (i, &(idx, data)) in entries.iter().enumerate() {
            let at = 4 + 8 * i;
            p[at..at + 4].copy_from_slice(&idx.to_le_bytes());
            p[at + 4..at + 8].copy_from_slice(&data.to_le_bytes());
        }
        p
    }

    fn pairs(p: &[u8]) -> Vec<(u32, u32)> {
        let n = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize;
        (0..n.min(MAX_LIST as usize))
            .map(|i| {
                let at = 4 + 8 * i;
                (
                    u32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]]),
                    u32::from_le_bytes([p[at + 4], p[at + 5], p[at + 6], p[at + 7]]),
                )
            })
            .collect()
    }

    /// Decode the `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_*` fields
    /// (`ogkm-580: ctrl2080bus.h:355-390`). ⚠ The decode is a **reading aid printed beside
    /// the raw word**, never a substitute for it: the raw `u32` is what a port would have
    /// to serve, and a field layout the header states for `LINK_CAPS` is only *documented*
    /// to apply to `GEN_INFO` by a comment (`:154-175`).
    fn decode(v: u32) -> String {
        // `GEN_GEN1 == 0`, so the printed generation is the field value plus one. ⚠ `gen`
        // is a reserved keyword in edition 2024 and cannot be the name here.
        let generation = |n: u32| n + 1;
        format!(
            "MAX_SPEED={} MAX_WIDTH={} ASPM={} GEN=gen{} CURR_LEVEL=gen{} GPU_GEN=gen{} \
             SPEED_CHANGES={} hi31_25={:#x}",
            v & 0xf,
            (v >> 4) & 0x3f,
            (v >> 10) & 0x3,
            generation((v >> 12) & 0xf),
            generation((v >> 16) & 0xf),
            generation((v >> 20) & 0xf),
            (v >> 24) & 0x1,
            v >> 25,
        )
    }

    println!(
        "info  R22 businfo sweep   = {MAX_LIST} indices x 1 call each, {PARAMS}-byte params, \
         tail seeded 0xCD"
    );
    println!("info  R22 legend          = idx status data   (the port serves 0x2d ONLY)");

    for index in 0..MAX_LIST {
        let mut params = request(&[(index, 0)], 0xCD);
        let result = rm.control(subdevice, ControlCmd(CMD), &mut params);
        let echoed = u32::from_le_bytes([params[4], params[5], params[6], params[7]]);
        let data = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
        let tail_written = params[12..].iter().any(|&b| b != 0xCD);
        match result {
            Ok(()) => println!(
                "★     R22 {index:#04x}  NV_OK      data={data:#010x} ({data})  echo={echoed:#010x}\
                 {}",
                if tail_written {
                    "  tail=WRITTEN"
                } else {
                    "  tail=untouched"
                }
            ),
            Err(e) => println!("info  R22 {index:#04x}  refused    {e:?}"),
        }
    }

    // ★★★ The two requests libcuda actually issues, replayed BYTE FOR BYTE off
    // `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:44` and `:46` — stale `data`
    // words included, because libcuda reuses the buffer and the second call's request
    // carries the first call's answers. A replay that zeroed them would be a different
    // request, and this port's whole failure mode is a request-dependent reply.
    for (line, entries) in [
        (44usize, &[(0x00, 0), (0x02, 0), (0x0b, 0)][..]),
        (
            46,
            &[
                (0x0f, 3),
                (0x10, 0),
                (0x2c, 5),
                (0x2d, 0),
                (0x03, 0),
                (0x06, 0),
            ][..],
        ),
    ] {
        let mut params = request(entries, 0x00);
        match rm.control(subdevice, ControlCmd(CMD), &mut params) {
            Ok(()) => {
                let out: String = pairs(&params)
                    .iter()
                    .map(|(i, d)| format!("{i:#04x}={d:#010x} "))
                    .collect();
                println!("★     R22 libcuda:{line:<3}    = NV_OK  {out}");
            }
            Err(e) => println!(
                "FAIL  R22 libcuda:{line:<3}    = refused {e:?} — the committed trace says NV_OK"
            ),
        }
    }

    // ★★★ H1 vs H2. Sixteen reads of the ONE forwarded index, spaced, each decoded. H1
    // predicts sixteen identical words; H2 predicts the `CURR_LEVEL` field moving with the
    // link, which on an idle GA106 sits at gen1 and climbs the moment anything touches it.
    // ⊘ Sixteen identical words do NOT prove H1 — an idle link is a constant link — so the
    // decode is printed with them: a word that CONTAINS a current-speed field is
    // link-describing whether or not it happened to move during this run.
    println!("info  R22 0x2d x16        = is PCIE_GEN_INFO a constant? (H1) or link state? (H2)");
    let mut seen: Vec<u32> = Vec::new();
    for round in 0..16 {
        let mut params = request(&[(PCIE_GEN_INFO, 0)], 0xCD);
        match rm.control(subdevice, ControlCmd(CMD), &mut params) {
            Ok(()) => {
                let v = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
                if !seen.contains(&v) {
                    seen.push(v);
                }
                println!("★     R22 0x2d #{round:<2}      = {v:#010x}  {}", decode(v));
            }
            Err(e) => println!("info  R22 0x2d #{round:<2}      = refused {e:?}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    println!(
        "info  R22 0x2d distinct   = {} value(s): {}",
        seen.len(),
        seen.iter()
            .map(|v| format!("{v:#010x} "))
            .collect::<String>()
    );
}

/// ★★★ R23 — **`BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS`: is the refusal the CALLER, or the
/// INSTRUMENT'S OWN SEED?**
///
/// §14.30 measured `--probe-ctrl 0x2080182a:112` refused `0x56` **twice** on the same
/// physical GA106 that answers libcuda `NV_OK`, and concluded from the `_DISPATCH` suffix
/// that *"the answer depends on caller state `rmladder` does not reproduce"* — object
/// hierarchy, prior calls, client privilege.
///
/// ⊘ **Read the params struct before believing that.** `capType` is an **`[IN]`** field
/// (`ogkm-580: ctrl2080bus.h:1256-1258, 1311-1315`), and `probe_ctrl` seeds *every* byte
/// with `0xCD` — so R18 asked for `capType = 0xCDCDCDCD`, which is none of
/// `_CAPTYPE_SYSMEM(0)` / `_GPU(1)` / `_P2P(2)` (`:1226-1228`). libcuda hands RM a **zeroed**
/// buffer, so it asks for `_CAPTYPE_SYSMEM`. The two callers did not issue the same call.
///
/// ⇒ The sentinel that makes R18 able to tell *written* from *unwritten* is only safe on a
/// **pure-OUT** struct. On a struct with an `[IN]` field it is an input **mutation**, and
/// the instrument perturbs the very thing it measures.
///
/// | hypothesis | prediction |
/// |---|---|
/// | **H1 — caller state** (§14.30's). The bare Subdevice is missing something libcuda has. | every arm below refuses `0x56`, whatever the request bytes say. |
/// | **H2 — the seed.** `capType = 0xCDCDCDCD` is an invalid captype and the refusal is the request's. | the arms whose `capType` is `0/1/2` answer `NV_OK` on the very same bare Subdevice; the `0xCD`-captype arms refuse. |
///
/// The arms are the 2x2 `{capType ∈ 0, 0xCDCDCDCD} x {tail ∈ 0x00, 0xCD}` plus the three
/// declared captypes and one out-of-range one. The 2x2 is what separates *"the captype is
/// invalid"* from *"a seeded byte anywhere in the buffer is refused"* — H2 is only
/// established if the poison **follows `capType`** and not the tail.
///
/// ★ The `0xCD` tail is retained wherever it is not the variable under test, because it is
/// still the only thing that can tell *"RM wrote 104 zeros"* from *"RM wrote nothing"* —
/// which is exactly the ambiguity that made the committed trace's all-zero `out=` decide
/// nothing (`traces/real_ga106/README.md`).
///
/// ⚠ The kernel RM cannot answer this control at all on a bare-metal GSP client: its flags
/// are `0x40048` = `NON_PRIVILEGED | ROUTE_TO_PHYSICAL | PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`
/// (`ogkm-580: g_subdevice_nvoc.c:6796-6819`, `rmapi/control.h:202-308`), so it is RPC'd to
/// GSP-RM, and the local `_92bfc3` arm NVOC installs for every non-VF variant is a bare
/// `return NV_ERR_NOT_SUPPORTED` (`g_subdevice_nvoc.h:6999-7002`) that exists precisely
/// because it should never run. ⇒ **the `_DISPATCH` is not the decider here**, and whatever
/// this rung prints is GSP firmware's answer, not the open tree's.
fn atomics_probe(rm: &mut HostRmBackend, subdevice: kayfabe_isolate::HostHandle) {
    const CMD: u32 = 0x2080_182a;
    // `4 + 4 + 13 * 8`. Confirmed on the wire as `size=112` in both the real-GA106 and the
    // guest cuInit traces.
    const PARAMS: usize = 112;
    const OP_COUNT: usize = 13;
    /// `ogkm-580: ctrl2080bus.h:1275-1287`, in declaration order — the array index IS the
    /// op type, so the order is the ABI.
    const OP_NAMES: [&str; OP_COUNT] = [
        "IADD", "IMIN", "IMAX", "INC", "DEC", "IAND", "IOR", "IXOR", "EXCH", "CAS", "FADD", "FMIN",
        "FMAX",
    ];

    /// `capType` at `[0..4]`, `dbdf` at `[4..8]`, then 13 x `{NvBool bSupported; NvU32
    /// attributes;}` — 8 bytes each because the `NvU32` forces 4-byte alignment, so bytes
    /// `+1..+4` of every entry are PADDING and RM is under no obligation to write them.
    fn request(cap_type: u32, dbdf: u32, tail: u8) -> Vec<u8> {
        let mut p = vec![tail; PARAMS];
        p[0..4].copy_from_slice(&cap_type.to_le_bytes());
        p[4..8].copy_from_slice(&dbdf.to_le_bytes());
        p
    }

    /// `ogkm-580: ctrl2080bus.h:1316-1338`.
    fn attrs(v: u32) -> String {
        const BITS: [&str; 8] = [
            "SCALAR",
            "VECTOR",
            "REDUCTION",
            "SIZE_32",
            "SIZE_64",
            "SIZE_128",
            "SIGNED",
            "UNSIGNED",
        ];
        let named: String = BITS
            .iter()
            .enumerate()
            .filter(|(i, _)| v & (1 << i) != 0)
            .map(|(_, n)| format!("{n} "))
            .collect();
        let unknown = v & !0xffu32;
        if unknown == 0 {
            named
        } else {
            format!("{named}hi8+={unknown:#x} ")
        }
    }

    println!(
        "info  R23 atomics probe   = {CMD:#010x}, {PARAMS}-byte params, one bare Subdevice, \
         no channel"
    );
    println!("info  R23 H1=caller-state (all arms refuse)  H2=the 0xCD seed IS the capType");

    // (label, capType, dbdf, tail seed)
    let arms: [(&str, u32, u32, u8); 8] = [
        // The 2x2 that separates the two hypotheses.
        ("R18 replay  cap=CD tail=CD", 0xCDCD_CDCD, 0xCDCD_CDCD, 0xCD),
        ("cap=CD tail=00           ", 0xCDCD_CDCD, 0x0000_0000, 0x00),
        ("cap=SYSMEM(0) tail=CD    ", 0, 0, 0xCD),
        ("libcuda replay all-zero  ", 0, 0, 0x00),
        // The other declared captypes, and one that is declared nowhere.
        ("cap=GPU(1) tail=CD       ", 1, 0, 0xCD),
        ("cap=P2P(2) tail=CD       ", 2, 0, 0xCD),
        ("cap=3 (undeclared) tail=CD", 3, 0, 0xCD),
        ("cap=SYSMEM dbdf=CD tail=CD", 0, 0xCDCD_CDCD, 0xCD),
    ];

    for (label, cap_type, dbdf, tail) in arms {
        let mut p = request(cap_type, dbdf, tail);
        let result = rm.control(subdevice, ControlCmd(CMD), &mut p);
        if let Err(e) = result {
            println!("info  R23 {label} = refused {e:?}");
            continue;
        }
        let echo_cap = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
        let echo_dbdf = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
        // ★ Reported separately from the values, for R18's reason: `NV_OK` over an
        // untouched buffer is a different fact from `NV_OK` over 104 written zeros, and
        // only the seed can tell them apart.
        let touched = p[8..].iter().any(|&b| b != tail);
        println!(
            "★     R23 {label} = NV_OK  echo cap={echo_cap:#010x} dbdf={echo_dbdf:#010x}  \
             body={}",
            if touched { "WRITTEN" } else { "UNTOUCHED" }
        );
        let hex: String = p.iter().map(|b| format!("{b:02x}")).collect();
        println!("      R23 {label}   raw={hex}");
        if !touched {
            continue;
        }
        for (i, name) in OP_NAMES.iter().enumerate() {
            let at = 8 + i * 8;
            let supported = p[at];
            let pad = &p[at + 1..at + 4];
            let a = u32::from_le_bytes([p[at + 4], p[at + 5], p[at + 6], p[at + 7]]);
            println!(
                "      R23 op[{i:>2}] {name:<5} bSupported={supported:#04x} attributes={a:#010x} \
                 [{}] pad={:02x}{:02x}{:02x}",
                attrs(a),
                pad[0],
                pad[1],
                pad[2]
            );
        }
    }
}

/// ★★★ R24 — **`CE_GET_CE_PCE_MASK` (`0x20802a02`) per copy engine, from the real part.**
///
/// §14.42's wall is `queryCopyEngines` (`ogkm-580: nv_gpu_ops.c:8449-8541`), and its per-CE
/// loop issues **two** controls that reach this port's boundary, back to back:
/// `0x20802a01 CE_GET_CAPS` — which the guest kernel turns into `0x20802a07
/// CE_GET_PHYSICAL_CAPS` (`kernel_ce.c:551-556`) — and then, six lines later,
/// `0x20802a02 CE_GET_CE_PCE_MASK`. Both are checked with a hard `goto done` on any status
/// but `NV_OK`, so serving only the first moves the wall by six lines.
///
/// ## ★★★ Why this rung exists at all, when `0x20802a07`'s answer needed no rung
///
/// The two ids are in **opposite** epistemic positions, and that is the whole point:
///
/// - `0x20802a07` is `KERNEL_PRIVILEGED` (flags `0x301d0`, `ogkm-580:
///   g_subdevice_nvoc.c:7645-7658` — neither `PRIVILEGED(0x4)` nor `NON_PRIVILEGED(0x8)`,
///   which is the default that refuses every usermode client including root,
///   `control.h:170-247`). ⊘ **Unreachable from here**, exactly like `0x20802a0b`. Its
///   answer is *derived* instead — projected out of [`kayfabe_abi::cecaps`], which already
///   states this silicon fact from two independent real-GA106 captures.
/// - `0x20802a02` carries flags `0x30349` (`g_subdevice_nvoc.c:7585-7598`), which **does**
///   include `NON_PRIVILEGED(0x8)` — and `ROUTE_TO_PHYSICAL(0x40)`, with no body anywhere in
///   the vendored tree (only the export row references `subdeviceCtrlCmdCeGetCePceMask_IMPL`;
///   the implementation is inside GSP-RM firmware). ⇒ It is **reachable and unreadable**: a
///   real part is the only oracle, and it is one we can actually ask.
///
/// ★ So the rung is not "measure because measuring is nice". It is: this is the one of the
/// two that *can* be measured, and `derive_what_you_cannot_query_then_oracle_it` says the
/// measurable one gets measured rather than guessed alongside its neighbour.
///
/// ## ⊘⊘ Why `--probe-ctrl 0x20802a02:8` would have been WRONG, and silently so
///
/// R18 seeds the whole params buffer with `0xCD` so it can tell *written* from *untouched*.
/// `NV2080_CTRL_CE_GET_CE_PCE_MASK_PARAMS` is `{ NvU32 ceEngineType; NvU32 pceMask; }`
/// (`ogkm-580: ctrl2080ce.h:167-170`) and **`ceEngineType` is `[IN]`** — the seed would ask
/// for engine type `0xCDCDCDCD`, which is not a copy engine, and the answer would be a
/// refusal that says nothing about the control. That is [`seed_only_the_out_region`] and
/// §14.31's `[IN]`-field trap, third sighting. This rung therefore sets `ceEngineType`
/// itself and seeds **only** the `pceMask` word.
///
/// ## ⚠ The engine-type encoding is TWO-BRANCH
///
/// `NV2080_ENGINE_TYPE_COPY(i) = (i < 10) ? COPY0 + i : COPY10 + i - 10`, with
/// `COPY0 = 0x09` and `COPY10 = 0x34` (`ogkm-580: cl2080_notification.h:291`, `:340`,
/// `:396`). A `0x09 + i` shortcut is right on every engine this part has and wrong on a
/// bigger one, so it is spelled out.
///
/// ## What is asked, and what each answer means
///
/// [`kayfabe_abi::cecaps`] measured `present = 0x0f` on this part — LCE0..LCE3 — against a
/// `NV_CE_MAX_LCE_MASK = 0x1f` that permits five. This rung asks **0..=4**, i.e. one past
/// the advertised end, precisely so the boundary is measured rather than assumed: a refusal
/// at `i = 4` corroborates `present = 0x0f` from a second, independent control.
fn ce_pce_mask_probe(rm: &mut HostRmBackend, subdevice: kayfabe_isolate::HostHandle) {
    const CMD: u32 = 0x2080_2a02;
    const PARAMS: usize = 8;
    /// `ogkm-580: cl2080_notification.h:291`.
    const COPY0: u32 = 0x09;
    /// `ogkm-580: cl2080_notification.h:340`.
    const COPY10: u32 = 0x34;

    /// `NV2080_ENGINE_TYPE_COPY(i)` — `ogkm-580: cl2080_notification.h:396`, both branches.
    fn engine_type_copy(i: u32) -> u32 {
        if i < 10 { COPY0 + i } else { COPY10 + i - 10 }
    }

    println!(
        "info  R24 pce-mask probe  = {CMD:#010x}, {PARAMS}-byte params, one bare Subdevice, \
         no channel"
    );
    println!("info  R24 ceEngineType is [IN] — seeding ONLY the pceMask word, NOT the R18 blanket");
    println!("info  R24 cecaps measured present=0x0f (LCE0..3); asking 0..=4 to MEASURE the edge");

    for i in 0..=4u32 {
        let et = engine_type_copy(i);
        let mut p = vec![0xCDu8; PARAMS];
        p[0..4].copy_from_slice(&et.to_le_bytes());
        match rm.control(subdevice, ControlCmd(CMD), &mut p) {
            Err(e) => {
                println!("info  R24 LCE{i} (type {et:#04x}) = refused {e:?} (no value measured)");
            }
            Ok(()) => {
                let echo = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                let mask = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                // ★ Reported separately from the value, for R18's reason: `NV_OK` over an
                // untouched word is a different fact from `NV_OK` over a written zero, and
                // only the seed can tell them apart. `0xCDCDCDCD` here means RM returned
                // success without writing the [OUT] field at all.
                let touched = mask != 0xCDCD_CDCD;
                println!(
                    "★     R24 LCE{i} (type {et:#04x}) = NV_OK  echo={echo:#010x} \
                     pceMask={mask:#010x} popcount={} [{}]",
                    mask.count_ones(),
                    if touched { "WRITTEN" } else { "UNTOUCHED" }
                );
            }
        }
    }
}

/// ★★★★★ R29 — **the SAME question as R25, asked of the code that SHIPS.**
///
/// ⊘ **Lead with what R25 already settled**, because this rung is easy to mis-sell: a
/// sealed `memfd` described to RM and placed at a dictated VA is a **PORT**, measured on a
/// real GA106 (`traces/real_ga106/rmladder_r25_osdescriptor_real_ga106.txt`). This rung
/// does not re-open that.
///
/// What it adds is that R25's route is a **parallel implementation** — it maps the block
/// itself, into its own `Reservation`, and calls `alloc_os_descriptor` directly. The
/// production route is `GuestRamPlane::honour` → `RmBackend::describe_guest_ram` →
/// `RmBackend::map_gpu_va`, driven by a VMM-minted `GuestRamGrant`, and **none of it is on
/// R25's path**. A defect anywhere in those three would leave R25 green.
///
/// ```text
///   ★     R29 guestpin     = placed at 0x… AS ASKED, window word matches   -> the ROUTE works
///   FAIL  R29 plane        = the grant was refused <RmError>               -> the plane
///   FAIL  R29 describe     = OS_DESCRIPTOR refused <RmError>               -> the verb
///   FAIL  R29 place        = asked 0x…, RM chose 0x…                       -> the fixed map
///   ??    R29 window       = placed as asked, but the isolate reads word … -> the OFFSET
/// ```
///
/// ★ The last cell is why the evidence has two predicates rather than one: a plane that
/// ignored the grant's offset would place correctly and read the **wrong bytes**, and
/// scoring that as a placement failure sends the next reader to the wrong file.
fn guest_ram_pin_probe(rm: &mut HostRmBackend, gpu: u32) -> bool {
    // ⊘ Neither R25's `0x3_0040_0000` nor R9's constant: two rungs sharing an address
    // cannot show that the address was honoured rather than remembered.
    const AT: GpuVa = GpuVa(0x3_0140_0000);
    const PATTERN: u32 = 0x9A11_0001;

    println!(
        "info  R29 guestpin probe = GPU {gpu}, euid {} — a shared memfd behind a real \
         GuestRamPlane, granted at a NON-ZERO offset, described through the port's own \
         `describe_guest_ram`, mapped at {:#018x}",
        kayfabe_linux_raw::geteuid(),
        AT.0,
    );

    let vas = match rm.alloc_vaspace() {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  R29 vaspace        = {e:?} (the rung needs its own address space)");
            return false;
        }
    };
    match rm.prove_guest_ram_pin(vas, AT, PATTERN) {
        Ok(e) if e.placed_as_asked() && e.window_is_the_granted_one() => {
            println!(
                "★     R29 guestpin       = placed at {:#018x} AS ASKED, {} bytes at grant \
                 offset {:#x}, and the isolate's mapping reads {:#010x} — the word the VMM \
                 wrote at that offset. ⊘ This is the PRODUCTION route (GuestRamPlane -> \
                 describe_guest_ram -> map_gpu_va), not R25's parallel one. ⊘ It is a memfd \
                 THIS process made: it is SHAPED like guest RAM and it is not the guest's.",
                e.got_va, e.bytes, e.offset, e.first_word
            );
            true
        }
        Ok(e) if !e.placed_as_asked() => {
            println!(
                "FAIL  R29 place          = asked {:#018x}, RM chose {:#018x} \
                 (DMA_OFFSET_FIXED_TRUE not honoured for guest-RAM-shaped memory through \
                 the production route) — address identity does not hold and a shadow \
                 channel cannot work",
                e.asked_va, e.got_va
            );
            false
        }
        Ok(e) => {
            println!(
                "??    R29 window         = placed at {:#018x} as asked, but the isolate's \
                 mapping reads {:#010x} where the VMM wrote {:#010x} at grant offset \
                 {:#x}. ⊘ NOT a placement failure: the address plane is fine and the \
                 GRANT'S OFFSET is not being honoured, which is a different file",
                e.got_va, e.first_word, e.expected_word, e.offset
            );
            false
        }
        Err(e) => {
            println!(
                "FAIL  R29 route          = refused {e:?}. `GuestRamUnavailable` is the \
                 plane, `NoMemory` on the describe is the `OS_DESCRIPTOR`, and anything \
                 else came from the placement — the three are separable and this line says \
                 which by its payload.",
            );
            false
        }
    }
}

/// ★★★★★ **R30 — THE CPU VIEW.** `docs/design/fb_cpu_view.md`.
///
/// `w228` backed three of `cuCtxCreate`'s framebuffer operands with real host vidmem and
/// left them with **no CPU view**, so the engine and the guest address two different
/// memories. This rung measures, on real hardware, which object can carry the missing view
/// — and it is deliberately a **host-side** ladder rung rather than a guest boot, because
/// every question it asks is about RM and none of them is about the guest.
/// ★★★★★ **w393 — THE BAR1 CROSSING, measured on bare metal: an ARMED DEVICE NODE crosses
/// a process boundary, the other process `mmap`s it, and the two processes' views of one
/// vidmem object are ONE MEMORY.** Then, if `/dev/kvm` is here, a real vCPU stores through a
/// memslot placed over a third view and the store lands on the card with NO exit.
///
/// This is the owner's `DEVICE_LOCAL | HOST_VISIBLE` target reduced to the two facts the
/// design rests on and nothing in this tree had measured:
///
/// 1. `nvidia_mmap_helper`'s framebuffer arm names no calling process (a READING,
///    `ogkm-580: kernel-open/nvidia/nv-mmap.c:505-641`) — so a node armed by one process can
///    be `mmap`ed by another. **Leg A** turns that reading into a measurement.
/// 2. A hypervisor memslot over a `VM_IO | VM_PFNMAP` mapping serves a guest store natively
///    (the VFIO shape; `nvkvm-pv` ships it at `nvkvm_mmap_host.c:1203`). **Leg B** measures
///    it with this tree's own raw KVM harness: the ONLY exit the guest takes is its
///    trapped signal store, never the data store, and the data is then read back through a
///    view the guest never had.
///
/// ⊘ What it does NOT measure, said here: that the **engine** reads those bytes. The GPU
/// half of the pair is `map_gpu_va` over the same object, which R25/R30 already exercise;
/// wiring a copy-engine readback into this rung is the next step, not this one.
///
/// The child is this same binary in `--bar1-crossing-child`, given one end of a socketpair
/// as its stdin. The node arrives as `SCM_RIGHTS` ancillary data on a length-prefixed frame
/// (`fdcross`), is kind-checked against the KERNEL (`require_kind`, never the sender's word),
/// `mmap`ed once at file offset zero, written, fenced and dropped. The parent closes its own
/// copy of that node the moment it is sent, so the child's mapping is the only one.
fn bar1_crossing_probe(rm: &mut HostRmBackend, gpu: u32) -> bool {
    use kayfabe_isolate_host::write_frame_with_fds;
    use kayfabe_linux_raw::{
        Backing, CachePolicy, GuestWindow, HostOffset, HostPageSize, Kvm, KvmMemslot, KvmVcpu,
        VcpuExit, VolatileRegion, release_fence,
    };
    use std::io::Read;
    use std::os::fd::AsFd;

    const LEN: u64 = 0x1_0000;
    const WORDS: u32 = 64;
    const PATTERN: u32 = 0xba51_0000;
    const KVM_PATTERN: u32 = 0xc0de_0001;

    println!(
        "==    W393 bar1-crossing = gpu {gpu}, euid {}, object {LEN:#x} bytes, {WORDS} words",
        kayfabe_linux_raw::geteuid()
    );
    let page = HostPageSize::query();

    // ---- 0. The object. Device-local, this connection's own.
    let mem = match rm.alloc_vidmem(LEN) {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  W393 vidmem        = {e:?}");
            return false;
        }
    };

    // ---- 1. THREE armed nodes on ONE object: A for the child, B for this process's
    // readback, C for the KVM leg. Each is a fresh `/dev/nvidia<N>` carrying its own
    // one-shot context (`nv-usermap.c:53-57`); the object is mapped once per node.
    let arm = |rm: &mut HostRmBackend, what: &str| match rm.export_device_view(mem, 0, LEN) {
        Ok(v) => {
            println!(
                "ok    W393 arm {what}       = node token {} mmap_len {:#x} (NV_ESC_RM_MAP_MEMORY \
                 accepted; the node is NOT mmapped here)",
                v.token, v.mmap_len
            );
            Some(v)
        }
        Err(e) => {
            println!(
                "FAIL  W393 arm {what}       = {e:?} ⊘ the premise — a CPU view of vidmem — \
                 refused; nothing below can run"
            );
            None
        }
    };
    let Some(va) = arm(rm, "A") else { return false };
    let Some(vb) = arm(rm, "B") else { return false };
    let (fd_a, fd_b) = match (rm.exports().lend(va.token), rm.exports().lend(vb.token)) {
        (Ok(a), Ok(b)) => (a, b),
        (a, b) => {
            println!("FAIL  W393 lend          = A {:?} B {:?}", a.err(), b.err());
            return false;
        }
    };

    // ---- 2. This process's own view, through B. Write-combining is the right REQUIREMENT
    // for a framebuffer object (`map_cpu`'s own docs); it is a requirement this layer
    // cannot verify, which is why it is a parameter there and a stated choice here.
    let view_b = match VolatileRegion::map(
        Backing::DeviceFile { fd: fd_b.as_fd() },
        vb.mmap_len,
        CachePolicy::WriteCombining,
        page,
    ) {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL  W393 mmap B        = {e} (this process, its own armed node)");
            return false;
        }
    };
    // The negative control's known state: zero every word through B and fence.
    for i in 0..WORDS {
        if view_b
            .store_u32(HostOffset::new(u64::from(i) * 4), 0)
            .is_err()
        {
            println!("FAIL  W393 prefill B     = store refused at word {i}");
            return false;
        }
    }
    release_fence();
    let before: Vec<u32> = (0..WORDS)
        .map(|i| {
            view_b
                .load_u32(HostOffset::new(u64::from(i) * 4))
                .unwrap_or(u32::MAX)
        })
        .collect();
    let before_nonzero = before.iter().filter(|w| **w != 0).count();
    println!(
        "ok    W393 control       = {WORDS} words zeroed through B, {before_nonzero} read back \
         non-zero (must be 0 — a view that does not hold its own stores decides nothing)"
    );
    if before_nonzero != 0 {
        return false;
    }

    // ---- 3. LEG A — the crossing. Spawn the child with one socket end as its stdin, send
    // node A on a frame, close our copy, wait.
    let Ok((ours, theirs)) = std::os::unix::net::UnixStream::pair() else {
        println!("FAIL  W393 socketpair    = refused");
        return false;
    };
    let Ok(exe) = std::env::current_exe() else {
        println!("FAIL  W393 current_exe   = unknown");
        return false;
    };
    let mut child = match std::process::Command::new(exe)
        .arg("--bar1-crossing-child")
        .stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(
            theirs,
        )))
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            println!("FAIL  W393 spawn         = {e}");
            return false;
        }
    };
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&va.mmap_len.to_le_bytes());
    body.extend_from_slice(&PATTERN.to_le_bytes());
    body.extend_from_slice(&WORDS.to_le_bytes());
    if let Err(e) = write_frame_with_fds(ours.as_fd(), &body, &[fd_a.as_fd()]) {
        println!("FAIL  W393 send A        = {e:?}");
        let _ = child.kill();
        return false;
    }
    // ★ Our copy of A goes NOW. From here the child's descriptor is the only one, so a
    // mapping the child makes is a mapping this process could not have made for it.
    drop(fd_a);
    drop(ours);
    let mut child_out = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_string(&mut child_out);
    }
    let status = child.wait().map(|s| s.code()).unwrap_or(None);
    for line in child_out.lines() {
        println!("      W393 child         | {line}");
    }
    let child_ok = status == Some(0) && child_out.contains("BAR1X-CHILD wrote=");
    println!(
        "{}  W393 leg A child    = exit {status:?} — {}",
        if child_ok { "ok  " } else { "FAIL" },
        if child_ok {
            "the OTHER process mmapped the node we armed and stored through it"
        } else {
            "the child did not report a completed write; read its lines above"
        }
    );
    if !child_ok {
        return false;
    }

    // ---- 4. Read back through B, and compare against what the child says it wrote.
    release_fence();
    let mut agree = 0u32;
    let mut first_bad: Option<(u32, u32, u32)> = None;
    for i in 0..WORDS {
        let want = PATTERN ^ i;
        let got = view_b
            .load_u32(HostOffset::new(u64::from(i) * 4))
            .unwrap_or(u32::MAX);
        if got == want {
            agree += 1;
        } else if first_bad.is_none() {
            first_bad = Some((i, want, got));
        }
    }
    let leg_a = agree == WORDS;
    match first_bad {
        None => println!(
            "★★★★★ W393 LEG A        = ONE MEMORY: all {WORDS} words the child stored through \
             node A read back through node B in THIS process. ⇒ an armed device node CROSSES \
             a process boundary and the framebuffer mmap path names no caller — the reading \
             of nv-mmap.c:505-641 is now a measurement."
        ),
        Some((i, want, got)) => println!(
            "FAIL  W393 LEG A        = {agree}/{WORDS} words agree; first disagreement at word \
             {i}: wanted {want:#010x}, read {got:#010x} through B. ⊘ Either the child's mapping \
             was not this object, or the two views are not coherent — read the child's lines."
        ),
    }
    if !leg_a {
        return false;
    }

    // ---- 5. LEG B — a REAL GUEST STORE through a memslot over a device view. Skipped BY
    // NAME without /dev/kvm; never silently.
    let kvm = match Kvm::open() {
        Ok(k) => k,
        Err(e) => {
            println!(
                "⊘     W393 LEG B        = SKIPPED: /dev/kvm is not usable here ({e}). Leg A \
                 stands on its own; leg B needs a KVM-capable box (vast: vms_enabled=true + \
                 the KVM template)."
            );
            return true;
        }
    };
    let Some(vc) = arm(rm, "C") else { return false };
    let Ok(fd_c) = rm.exports().lend(vc.token) else {
        println!("FAIL  W393 lend C        = refused");
        return false;
    };
    // The window a memslot needs, with the device view placed WHOLE inside it — the exact
    // verb `QemuMachine::install_device_window` uses, one crate up.
    let win = match GuestWindow::create(vc.mmap_len, page) {
        Ok(w) => Arc::new(w),
        Err(e) => {
            println!("FAIL  W393 window        = {e}");
            return false;
        }
    };
    if let Err(e) = win.place_device_view(HostOffset::ZERO, vc.mmap_len, fd_c.as_fd()) {
        println!("FAIL  W393 place C       = {e} (GuestWindow::place_device_view)");
        return false;
    }
    drop(fd_c);
    let vm = match kvm.create_vm() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            println!("FAIL  W393 kvm vm        = {e}");
            return false;
        }
    };
    let _ = vm.set_tss_addr_if_supported();
    // Guest layout: code page at `page`, the device view at DATA_GPA, a trap address no
    // slot covers as the guest's "done" signal.
    let code_gpa = page.bytes();
    let data_gpa: u64 = 0x1000_0000;
    let trap_gpa: u64 = 0x8000_0000;
    //   0: C7 05 <data> <value>   mov dword [data], value   ; the STORE under test
    //  10: A1 <data>              mov eax, [data]           ; a LOAD from the slot
    //  15: 89 03                  mov [ebx], eax            ; the trapped signal store
    //  17: F4                     hlt
    let d = u32::try_from(data_gpa).expect("below 4 GiB").to_le_bytes();
    let v = KVM_PATTERN.to_le_bytes();
    let image: [u8; 18] = [
        0xC7, 0x05, d[0], d[1], d[2], d[3], v[0], v[1], v[2], v[3], // mov [data], value
        0xA1, d[0], d[1], d[2], d[3], // mov eax, [data]
        0x89, 0x03, // mov [ebx], eax
        0xF4, // hlt
    ];
    let code_win = match GuestWindow::create(page.bytes(), page) {
        Ok(w) => Arc::new(w),
        Err(e) => {
            println!("FAIL  W393 code window   = {e}");
            return false;
        }
    };
    if code_win.write_from(HostOffset::ZERO, &image).is_err() {
        println!("FAIL  W393 code fill     = refused");
        return false;
    }
    let _code_slot = match KvmMemslot::install(
        Arc::clone(&vm),
        0,
        code_gpa,
        Arc::clone(&code_win),
        0,
        page.bytes(),
        false,
    ) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL  W393 code slot     = {e}");
            return false;
        }
    };
    let _data_slot = match KvmMemslot::install(
        Arc::clone(&vm),
        1,
        data_gpa,
        Arc::clone(&win),
        0,
        vc.mmap_len,
        false,
    ) {
        Ok(s) => {
            println!(
                "ok    W393 device slot   = KVM_SET_USER_MEMORY_REGION accepted a VM_PFNMAP \
                 device view as memslot 1 at {data_gpa:#x} ({:#x} bytes)",
                vc.mmap_len
            );
            s
        }
        Err(e) => {
            println!(
                "FAIL  W393 device slot   = {e} ⊘ the kernel refused a memslot over the device \
                 mapping — THAT is a finding: KVM will not take this VMA as guest memory"
            );
            return false;
        }
    };
    let mut vcpu = match KvmVcpu::create(&kvm, Arc::clone(&vm), 0) {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL  W393 vcpu          = {e}");
            return false;
        }
    };
    if let Err(e) = vcpu.enter_flat_protected_mode(code_gpa, trap_gpa) {
        println!("FAIL  W393 vcpu mode     = {e}");
        return false;
    }
    // ★ THE ACCEPTANCE TEST, criterion 1: the first exit must be the SIGNAL store, carrying
    // the value the guest loaded back from the slot. An exit AT data_gpa is the device slot
    // failing to serve the store — the opposite finding, and it is named.
    let exit = loop {
        match vcpu.run() {
            Ok(VcpuExit::Interrupted) => continue,
            Ok(e) => break e,
            Err(e) => {
                println!("FAIL  W393 vcpu run      = {e}");
                return false;
            }
        }
    };
    let leg_b_exit = match exit {
        VcpuExit::Mmio {
            gpa,
            len,
            is_write,
            data,
        } if gpa == trap_gpa && is_write => {
            let got = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            println!(
                "ok    W393 guest exit    = the ONLY exit is the signal store at {gpa:#x} \
                 ({len} bytes), carrying eax={got:#010x} loaded back from the slot (want \
                 {KVM_PATTERN:#010x}) — the data store and the data load took NO exit"
            );
            got == KVM_PATTERN
        }
        VcpuExit::Mmio { gpa, is_write, .. } if gpa == data_gpa => {
            println!(
                "FAIL  W393 guest exit    = the guest EXITED on its {} at the DEVICE SLOT \
                 ({gpa:#x}): the memslot over the device view did not serve it. ⊘ This is \
                 the KVM-over-VM_PFNMAP question answered NO on this kernel.",
                if is_write { "store" } else { "load" }
            );
            false
        }
        other => {
            println!(
                "FAIL  W393 guest exit    = {other:?} — neither the signal store nor a data-slot \
                 exit; the vCPU did not run the program as written"
            );
            false
        }
    };
    // Criterion 3, from the other side: the bytes are on the card, read through B — a view
    // the guest never had and this process wrote nothing into since the prefill.
    release_fence();
    let through_b = view_b.load_u32(HostOffset::ZERO).unwrap_or(u32::MAX);
    let leg_b = leg_b_exit && through_b == KVM_PATTERN;
    println!(
        "{} W393 LEG B        = word 0 through B after the guest ran = {through_b:#010x} (want \
         {KVM_PATTERN:#010x}). {}",
        if leg_b { "★★★★★" } else { "FAIL " },
        if leg_b {
            "⇒ DEVICE_LOCAL | HOST_VISIBLE, measured: a guest CPU store took no VM exit and \
             landed in card memory another CPU view reads. ⊘ The ENGINE half is not measured \
             here; see the rung's doc."
        } else {
            "⊘ the guest's store did not reach the card through the slot"
        }
    );
    leg_b
}

/// ★ w393 — the child half of [`bar1_crossing_probe`]. Reads ONE frame carrying ONE
/// descriptor from stdin, checks the kernel's word on what it is, maps it once at offset
/// zero for the length the parent named, stores the pattern, fences, and reports.
fn bar1_crossing_child() -> std::process::ExitCode {
    use kayfabe_isolate_host::read_frame_with_fds;
    use kayfabe_linux_raw::{
        Backing, CachePolicy, DescriptorKind, HostOffset, HostPageSize, VolatileRegion,
        release_fence, require_kind,
    };
    use std::os::fd::AsFd;

    let stdin = std::io::stdin();
    let mut buf = Vec::new();
    let mut fds = Vec::new();
    match read_frame_with_fds(stdin.as_fd(), &mut buf, &mut fds, 1) {
        Ok(true) => {}
        other => {
            println!("BAR1X-CHILD FAIL frame={other:?}");
            return std::process::ExitCode::from(2);
        }
    }
    if buf.len() != 16 || fds.len() != 1 {
        println!(
            "BAR1X-CHILD FAIL frame shape: {} body bytes, {} descriptor(s)",
            buf.len(),
            fds.len()
        );
        return std::process::ExitCode::from(2);
    }
    let mmap_len = u64::from_le_bytes(buf[0..8].try_into().expect("8 bytes"));
    let pattern = u32::from_le_bytes(buf[8..12].try_into().expect("4 bytes"));
    let words = u32::from_le_bytes(buf[12..16].try_into().expect("4 bytes"));
    let fd = fds.pop().expect("exactly one");
    // ★ The KERNEL says what arrived. The parent asked for a character device; anything else
    // is refused by name before it is mapped.
    if let Err(e) = require_kind(fd.as_fd(), DescriptorKind::CharDevice) {
        println!("BAR1X-CHILD FAIL kind={e}");
        return std::process::ExitCode::from(3);
    }
    let view = match VolatileRegion::map(
        Backing::DeviceFile { fd: fd.as_fd() },
        mmap_len,
        CachePolicy::WriteCombining,
        HostPageSize::query(),
    ) {
        Ok(v) => v,
        Err(e) => {
            println!(
                "BAR1X-CHILD FAIL mmap={e} ⊘ the driver refused THIS process's mmap of a node \
                 ANOTHER process armed — if leg A's premise fails, it fails here"
            );
            return std::process::ExitCode::from(4);
        }
    };
    let mut wrote = 0u32;
    for i in 0..words {
        if view
            .store_u32(HostOffset::new(u64::from(i) * 4), pattern ^ i)
            .is_ok()
        {
            wrote += 1;
        }
    }
    release_fence();
    // Read our own stores back through the same view, so the parent can tell "the child's
    // view holds stores" from "the two views agree" — two facts, reported separately.
    let mut held = 0u32;
    for i in 0..words {
        if view.load_u32(HostOffset::new(u64::from(i) * 4)) == Ok(pattern ^ i) {
            held += 1;
        }
    }
    drop(view);
    drop(fd);
    println!(
        "BAR1X-CHILD wrote={wrote} held={held} of {words} kind=CharDevice mmap_len={mmap_len:#x}"
    );
    if wrote == words {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::from(5)
    }
}

fn fb_view_probe(rm: &mut HostRmBackend, gpu: u32, join: FbViewJoin) -> bool {
    // ⊘ A VA in the same band the other ladder rungs use, and one this process's own VAS
    // demonstrably does not already bind — the point is `DMA_OFFSET_FIXED_TRUE`, not the
    // number.
    const AT: GpuVa = GpuVa(0x0000_7f00_0000_0000);
    const PATTERN: u32 = 0xfbc0_0001;

    println!(
        "==    R30 fb-cpu-view    = gpu {gpu}, euid {}, join {join:?}, FIXED at {:#018x}{}",
        kayfabe_linux_raw::geteuid(),
        AT.0,
        match join {
            FbViewJoin::Shared => "",
            FbViewJoin::Private =>
                "  [NEGATIVE CONTROL: the guest-side view is PRIVATE ANONYMOUS, so it is \
                 NOT the same pages — BOTH directions must MISMATCH, and that is the PASS]",
        }
    );

    let vas = match rm.alloc_vaspace() {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  R30 vaspace        = {e:?} (the rung needs its own address space)");
            return false;
        }
    };

    let ev = match rm.prove_fb_view(vas, AT, PATTERN, join) {
        Ok(e) => e,
        Err(e) => {
            println!("FAIL  R30 fb-cpu-view    = {e:?}");
            return false;
        }
    };

    // ---- 1. THE PREMISE. Printed first and judged on its own, because every later line is
    // about a different object and a reader must not have to infer which.
    match (ev.vidmem_cpu_view, ev.vidmem_cpu_refusal) {
        (Some(v), _) if v.agrees() => println!(
            "★     R30 premise        = the vidmem object `alloc_vidmem` mints IS \
             CPU-MAPPABLE: NV_ESC_RM_MAP_MEMORY succeeded and {} words round-tripped \
             through that mapping. ⊘ Through the SAME mapping, so this says the view takes \
             stores and loads — NOT that the card holds them.",
            v.words_compared
        ),
        (Some(v), _) => println!(
            "FAIL  R30 premise        = the mapping succeeded but word {} read {:#010x} \
             where {:#010x} was stored — a CPU view that does not hold its own writes",
            v.mismatch.map_or(0, |m| m.word),
            v.read_back,
            v.wrote
        ),
        (None, Some(st)) => println!(
            "⊘     R30 premise REFUTED= NV_ESC_RM_MAP_MEMORY REFUSED the vidmem object \
             (status {st:#x}). The brief's premise is FALSE and the successor rung cannot \
             be built on it.",
        ),
        (None, None) => println!("FAIL  R30 premise        = neither a view nor a refusal"),
    }

    // ---- 2. THE NEGATIVE CONTROL on the crossing.
    let control_ok = match ev.device_export {
        DeviceExportOutcome::RefusedByName => {
            println!(
                "ok    R30 neg control    = export_backing(HostDeviceMemory) on that SAME \
                 live object → NotExportableAsMemory, BY NAME. ⇒ the CPU view exists and \
                 CANNOT cross to the VMM as memory, which is the whole of why `w228`'s \
                 objects have no guest-reachable view."
            );
            true
        }
        DeviceExportOutcome::RefusedOtherwise => {
            println!(
                "FAIL  R30 neg control    = it refused, but NOT by name. The named boundary \
                 decision (b) rests on is arriving as an opaque status."
            );
            false
        }
        DeviceExportOutcome::Succeeded => {
            println!(
                "★★    R30 neg control    = it SUCCEEDED. Three cited driver facts say a \
                 host GPU page cannot cross as memory; one of them is wrong, and THAT is \
                 this run's finding."
            );
            false
        }
    };

    // ---- 3. THE JOIN, both directions, judged by the arm this run is.
    let say = |name: &str, v: ViewCompare| match v.mismatch {
        None if v.words_compared > 0 => println!(
            "      R30 {name:<14}= all {} words AGREE ({:#010x} → {:#010x})",
            v.words_compared, v.wrote, v.read_back
        ),
        None => println!("      R30 {name:<14}= ⊘ the loop compared ZERO words — VOID"),
        Some(m) => println!(
            "      R30 {name:<14}= DISAGREE at word {} (got {:#010x}, want {:#010x}) of {} \
             compared",
            m.word, m.got, m.want, v.words_compared
        ),
    };
    say("guest→host", ev.guest_to_host);
    say("host→guest", ev.host_to_guest);

    match join {
        // ⊘ The control is judged by its OWN rule, and it must fail in BOTH directions at
        // word 0: private pages are zero-filled, and the pattern's word 0 is non-zero.
        // A control that failed in only one direction would mean the two mappings are
        // partially shared, which is not a state this code can produce — so it would be a
        // fact about the instrument.
        FbViewJoin::Private => {
            let g = ev.guest_to_host.mismatch;
            let h = ev.host_to_guest.mismatch;
            let both_at_zero =
                matches!(g, Some(m) if m.word == 0) && matches!(h, Some(m) if m.word == 0);
            if both_at_zero {
                println!(
                    "ok    R30 CONTROL FIRED  = with the guest-side view on PRIVATE pages the \
                     differential fails at word 0 in BOTH directions. ⇒ the comparison CAN \
                     fail, so the shared run's agreement is a measurement and not a tautology."
                );
                control_ok
            } else {
                println!(
                    "FAIL  R30 CONTROL        = private pages compared EQUAL (or failed \
                     late). The differential is not reading what it claims to, and every \
                     green this rung prints is void."
                );
                false
            }
        }
        FbViewJoin::Shared => {
            if !ev.placed_as_asked() {
                println!(
                    "FAIL  R30 placement      = asked {:#018x}, RM gave {:#018x} — every byte \
                     above is about a different address",
                    ev.asked_va, ev.got_va
                );
                return false;
            }
            if ev.joined() {
                println!(
                    "★     R30 JOINED         = ONE fabricated backing, TWO independent \
                     mappings, {} bytes agreeing in BOTH directions, described to RM as an \
                     OS_DESCRIPTOR and placed at {:#018x} AS ASKED. ⇒ this is the shape the \
                     framebuffer join must take; the vidmem object's view cannot cross and \
                     this one needs no crossing at all.",
                    ev.bytes, ev.got_va
                );
                true
            } else {
                println!(
                    "FAIL  R30 JOINED         = not all four facts hold (see the lines above)"
                );
                false
            }
        }
    }
}

/// ★★★ R25 — **does memory shaped like GUEST RAM reach the host GPU's MMU?**
///
/// The rung that decides whether `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` is a **port** or a
/// **design**, settled before a line of shadow-channel code exists. Host-only: no guest, no
/// doorbell, no GR, no VM boot. A sealed `memfd` — what a VMM backs guest RAM with — is
/// placed in a reservation, written with a per-word pattern by ordinary CPU stores,
/// described to RM, mapped into a host VAS **at an address we choose**, read by a real copy
/// engine, and compared word for word out the other side.
///
/// ## ★★ Why the number is R25 and not R20
///
/// ⊘ **R20 is taken.** It is the `NV2081_BINAPI` probe, forty lines up, and R21–R24 are the
/// four sweeps after it. A rung number is how a bench result is attributed months later;
/// two rungs sharing one is how a green line gets read as evidence for the wrong thing.
///
/// ## The four outcomes, and the fourth is the point
///
/// ```text
///   ok    R25 osdesc      = st=0, placed at 0x…, CE retired, N/N bytes match   -> a PORT
///   FAIL  R25 osdesc      = alloc refused <RmError>                            -> arm B
///   FAIL  R25 place       = asked 0x…, got 0x…                                 -> arm C
///   ??    R25 coherency   = placed, CE retired, MISMATCH at word N             -> arm ⊘
/// ```
///
/// ★ **Arm ⊘ is the cell a "did the ioctl succeed?" test scores GREEN**: permission and
/// placement fine, but the pages the GPU saw are not the pages we wrote. It is printed with
/// `??` rather than `FAIL` because it is not a failure of the chain under test — it is a
/// different subject (cache policy / coherency) arriving through the same door, and
/// labelling it `FAIL` would send the next reader to re-check the alloc flags.
///
/// ## ⊘ What a PASS here does **not** establish
///
/// - **Not the cap-dropped case.** This binary runs as whatever invoked it, and on the
///   bench that is root — so a pass takes `osIsAdministrator()`'s fast path through
///   `RmValidateMmapRequest` exactly as R16's docs describe. `euid` is printed with the
///   result for that reason, and the rung is worth running twice.
/// - **Not a guest VA.** The address is one *we* choose. Whether a host GPU walking a host
///   VAS built from *guest* VAs would miss is not visible from here, and with fault
///   delivery unbuilt such a miss is a **hang** inside UVM's replayable-fault loop rather
///   than an error.
fn osdesc_probe(rm: &mut HostRmBackend, gpu: u32, seed: OsDescSeed) -> bool {
    // A plausible guest sysmem VA: above whatever the driver reserves at the bottom of a
    // fresh `FERMI_VASPACE_A`, 2 MiB-aligned, and deliberately NOT R9's constant — two
    // rungs sharing an address cannot show that the address was honoured rather than
    // remembered.
    const AT: GpuVa = GpuVa(0x3_0040_0000);
    const PATTERN: u32 = 0x5EED_0001;

    println!(
        "info  R25 osdesc probe   = GPU {gpu}, euid {} — a sealed memfd, described to RM, \
         mapped at {:#018x}, read by a real CE{}",
        kayfabe_linux_raw::geteuid(),
        AT.0,
        match seed {
            OsDescSeed::BeforeDescribe => "",
            OsDescSeed::Never =>
                "  [NEGATIVE CONTROL: the memfd is deliberately NOT written, so a \
                 MISMATCH AT WORD 0 is the PASS]",
        }
    );

    let vas = match rm.alloc_vaspace() {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  R25 vaspace        = {e:?} (the rung needs its own address space)");
            return false;
        }
    };

    let verdict = match rm.prove_os_descriptor(vas, AT, PATTERN, seed) {
        // ⊘ The negative control, judged FIRST and by its own rule: everything up to the
        // bytes must hold, and the bytes must DISAGREE at word 0. A `reached()` here would
        // mean the comparison is not reading what it claims to.
        Ok(e) if !e.seeded => match e.mismatch {
            Some(m) if m.word == 0 && m.want == PATTERN => {
                println!(
                    "ok    R25 neg control    = the memfd was never written and the CE \
                         delivered {:#010x} at word 0 where the pattern would have been \
                         {:#010x} — the comparison CAN fail, so the positive run's \
                         agreement is a measurement",
                    m.got, m.want
                );
                true
            }
            Some(m) => {
                println!(
                    "??    R25 neg control    = it mismatched, but at word {} not word 0 \
                         (got {:#010x}, want {:#010x}) — the first {} bytes of an UNWRITTEN \
                         memfd matched a pattern nobody stored, which is a fact about the \
                         instrument, not about the descriptor",
                    m.word,
                    m.got,
                    m.want,
                    m.word * 4
                );
                false
            }
            None => {
                println!(
                    "FAIL  R25 neg control    = an UNWRITTEN memfd compared EQUAL over \
                         all {} of {} bytes. The comparison is not reading the destination, \
                         and every green this rung has ever printed is void.",
                    e.bytes_compared, e.bytes
                );
                false
            }
        },
        Ok(e) if e.reached() => {
            println!(
                "★     R25 osdesc         = placed at {:#018x} AS ASKED, CE retired \
                 (sem {:#010x}), dst[0] {:#010x} -> {:#010x}, and {} of {} bytes compared \
                 EQUAL — guest-RAM-shaped memory reaches the host GPU's MMU",
                e.got_va, e.submit.semaphore, e.before, e.after, e.bytes_compared, e.bytes
            );
            true
        }
        // ★ A comparison that stopped early without recording a mismatch is an instrument
        // failure, and it is checked BEFORE the coherency arm so it can never be reported
        // as one: "the loop did not run" and "the pages disagree" are different subjects.
        Ok(e) if e.mismatch.is_none() && !e.compared_everything() => {
            println!(
                "FAIL  R25 instrument     = the comparison covered {} of {} bytes and found \
                 no mismatch — a partial loop with a clean verdict. This run is VOID.",
                e.bytes_compared, e.bytes
            );
            false
        }
        // ★ Arm C first, because a mapping that landed somewhere else makes every byte
        // downstream a statement about a different address.
        Ok(e) if !e.placed_as_asked() => {
            println!(
                "FAIL  R25 place          = asked {:#018x}, RM chose {:#018x} \
                 (DMA_OFFSET_FIXED_TRUE not honoured for DESCRIBED memory) — address \
                 identity does not extend to OS_DESCRIPTOR, so shadow-forwarding cannot \
                 work as designed",
                e.asked_va, e.got_va
            );
            false
        }
        Ok(e) if e.submit.semaphore != e.payload => {
            println!(
                "FAIL  R25 CE             = placed at {:#018x}, but the engine did not \
                 retire: sem {:#010x} (want {:#010x}) GP_GET {} GP_PUT {} — {}",
                e.got_va,
                e.submit.semaphore,
                e.payload,
                e.submit.gp_get,
                e.submit.gp_put,
                if e.submit.gp_get == e.submit.gp_put {
                    "the entry WAS fetched and the methods did nothing"
                } else {
                    "the entry was never fetched"
                }
            );
            false
        }
        // ⊘ Arm ⊘. Everything the chain is *about* worked; the bytes disagree.
        Ok(e) => {
            match e.mismatch {
                Some(m) => println!(
                    "??    R25 coherency      = placed at {:#018x} as asked, CE retired \
                     (sem {:#010x}), but MISMATCH at word {} (byte {}): got {:#010x}, \
                     want {:#010x} — permission and placement are fine and the pages the \
                     GPU saw are NOT the pages we wrote. This is cache policy / coherency \
                     (the C chose COHERENCY_CACHED, `C: nvkvm_gpu_emul.c:7519-7524`), not \
                     the descriptor.",
                    e.got_va,
                    e.submit.semaphore,
                    m.word,
                    m.word * 4,
                    m.got,
                    m.want
                ),
                // Every word matched but `reached()` still said no — the only way left is
                // the non-vacuity check, i.e. the destination never held the sentinel. That
                // is an instrument failure, not a result, and it is named as one.
                None => println!(
                    "FAIL  R25 instrument     = every word matched but the destination's \
                     `before` was {:#010x}, not the sentinel {:#010x} — the pre-fill did \
                     not take, so a match proves nothing and this run is VOID",
                    e.before, e.sentinel
                ),
            }
            false
        }
        // ★ Arm B, and it is the one that redirects everything. Printed with the RM status
        // by name, because "refused" and "refused with NV_ERR_INSUFFICIENT_PERMISSIONS" are
        // different findings.
        Err(e) => {
            println!(
                "FAIL  R25 osdesc         = alloc refused {e:?} — a process of this \
                 privilege may not describe its own pages to RM. If this is euid 0, the \
                 whole guest-RAM plan needs another route; if it is not, the SANDBOX POLICY \
                 is the subject and this is a FINDING, not a licence to relax it."
            );
            false
        }
    };
    let _ = rm.free(vas);
    verdict
}

/// ★★★ R32 — **is ONE memfd, mapped TWICE, ONE memory on BOTH sides of the GPU?**
///
/// The two properties the framebuffer-memfd design rests on and that R25 does not test:
///
/// - **J1** — write through mapping `S`, describe mapping `I`, and the GPU reads `S`'s
///   bytes. R25 writes and describes through the *same* mapping. The shell holds the BAR
///   view and the isolate holds the described view; they are different mappings, and a
///   design proved only through the described one has not been proved.
/// - **J2** — the GPU **writes** and a CPU mapping **reads it back**. ★ This is the
///   direction `cuCtxCreate` is stuck on: the guest's completion semaphore is a word the
///   engine writes and the guest reads, and every byte of OS_DESCRIPTOR evidence this tree
///   owns runs the other way.
///
/// ```text
///   ok    R32 cpu join      = the two mappings are one memory  -> J1/J2 can be asked
///   ok    R32 forward       = the GPU read what the OTHER mapping wrote  -> J1
///   ok    R32 reverse       = the OTHER mapping read what the GPU wrote  -> J2
///   ??    R32 reverse       = ... got P1 ... -> the engine did nothing; we read our own seed
///   FAIL  R32 osdesc        = alloc refused <RmError>                    -> R25's arm B
/// ```
///
/// ⊘ **Not a boot.** This measures the primitive a framebuffer-memfd port would be built
/// on, so that a failed boot could not be blamed on it. It says nothing about the shell's
/// BAR trap path, about sparsity, or about two *processes*.
fn fb_memfd_join_probe(rm: &mut HostRmBackend, gpu: u32, seed: OsDescSeed) -> bool {
    // ⊘ Deliberately neither R25's constant nor R26's: two rungs sharing an address cannot
    // show that the address was honoured rather than remembered.
    const AT: GpuVa = GpuVa(0x3_0140_0000);

    println!(
        "info  R32 fb-join probe  = GPU {gpu}, euid {} — ONE sealed memfd mapped TWICE; \
         mapping I described to RM at {:#018x}; every byte written and read through \
         mapping S{}",
        kayfabe_linux_raw::geteuid(),
        AT.0,
        match seed {
            OsDescSeed::BeforeDescribe => "",
            OsDescSeed::Never =>
                "  [NEGATIVE CONTROL: S is never written, so a FORWARD MISMATCH AT WORD 0 \
                 is the PASS — while the REVERSE arm, which does not depend on the seed, \
                 must still hold]",
        }
    );

    let vas = match rm.alloc_vaspace() {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  R32 vaspace        = {e:?} (the rung needs its own address space)");
            return false;
        }
    };

    let verdict = match rm.prove_fb_memfd_join(vas, AT, seed) {
        // ★ The join is judged FIRST and on its own, because if the two mappings are not
        // one memory then neither J1 nor J2 is a question about the GPU at all.
        Ok(e) if !e.joined() => {
            println!(
                "FAIL  R32 cpu join       = mapping S read {:#010x} before and {:#010x} after \
                 mapping I wrote {:#010x} — the two mappings are NOT one memory, so nothing \
                 downstream is about the GPU. This run is VOID.",
                e.join_before, e.join_after, e.join_want
            );
            false
        }
        // ★ Arm C next: a mapping that landed elsewhere makes every byte downstream a
        // statement about a different address.
        Ok(e) if !e.placed_as_asked() => {
            println!(
                "FAIL  R32 place          = asked {:#018x}, RM chose {:#018x} — address \
                 identity does not extend to OS_DESCRIPTOR",
                e.asked_va, e.got_va
            );
            false
        }
        Ok(e) => {
            println!(
                "ok    R32 cpu join       = mapping S read {:#010x} before and {:#010x} after \
                 mapping I wrote it — ONE memory, measured with no GPU in the path",
                e.join_before, e.join_after
            );

            // ── J1 ──────────────────────────────────────────────────────────────────
            let fwd = if e.seeded {
                if e.forward_reached() {
                    println!(
                        "ok    R32 forward (J1)   = placed at {:#018x} AS ASKED, CE retired \
                         (sem {:#010x}), dst[0] {:#010x} -> {:#010x}, and {} of {} bytes \
                         compared EQUAL — the GPU read what the OTHER mapping wrote",
                        e.got_va,
                        e.fwd_submit.semaphore,
                        e.fwd_before,
                        e.fwd_after,
                        e.fwd_bytes_compared,
                        e.bytes
                    );
                    true
                } else {
                    println!(
                        "??    R32 forward (J1)   = sem {:#010x} (want {:#010x}) GP_GET {} \
                         GP_PUT {}, before {:#010x} (sentinel {:#010x}), after {:#010x}, \
                         compared {} of {}, mismatch {:?}",
                        e.fwd_submit.semaphore,
                        e.fwd_payload,
                        e.fwd_submit.gp_get,
                        e.fwd_submit.gp_put,
                        e.fwd_before,
                        e.fwd_sentinel,
                        e.fwd_after,
                        e.fwd_bytes_compared,
                        e.bytes,
                        e.fwd_mismatch
                    );
                    false
                }
            } else {
                // ⊘ The negative control, judged by its own rule: everything up to the
                // bytes must hold and the bytes must DISAGREE at word 0 with a ZERO, which
                // is what an unwritten memfd holds. A `forward_reached()` here would mean
                // the comparison is not reading what it claims to.
                match e.fwd_mismatch {
                    Some(m) if m.word == 0 && m.got == 0 => {
                        println!(
                            "ok    R32 neg control    = S was never written and the CE \
                             delivered {:#010x} at word 0 where the pattern would have been \
                             {:#010x} — the forward comparison CAN fail, so a seeded run's \
                             agreement is a measurement",
                            m.got, m.want
                        );
                        true
                    }
                    other => {
                        println!(
                            "FAIL  R32 neg control    = an unwritten memfd produced {other:?} \
                             — the expected reading is a mismatch at word 0 with got=0. \
                             Either the comparison is not reading S's pages, or something \
                             wrote them."
                        );
                        false
                    }
                }
            };

            // ── J2 ──────────────────────────────────────────────────────────────────
            // ★ Judged the same way in BOTH arms: the reverse copy does not depend on the
            // seed, so the negative control must still produce it. A control that turned
            // this arm off would be testing a different chain.
            let expected_before = if e.seeded { e.rev_first } else { 0 };
            let rev = if e.reverse_reached() {
                println!(
                    "ok    R32 reverse (J2)   = CE retired (sem {:#010x}); the memfd held \
                     {:#010x} through S immediately before the copy and {} of {} bytes read \
                     back through S EQUAL afterwards — ★ the GPU WROTE and the OTHER \
                     mapping READ IT. This is the completion-semaphore direction.",
                    e.rev_submit.semaphore, e.rev_before, e.rev_bytes_compared, e.bytes
                );
                true
            } else {
                let named = match e.rev_mismatch {
                    Some(m) if m.got == 0 => "the copy never landed",
                    Some(m) if m.got == expected_before => {
                        "★ we are reading the memfd's PREVIOUS contents — the engine \
                         retired and wrote nothing these pages can see"
                    }
                    Some(_) => "the bytes are neither the old contents nor the new ones",
                    None => {
                        "no mismatch was recorded, so the failure is upstream of the \
                             comparison (semaphore, non-vacuity, or a short loop)"
                    }
                };
                println!(
                    "??    R32 reverse (J2)   = sem {:#010x} (want {:#010x}) GP_GET {} \
                     GP_PUT {}, memfd-through-S before the copy {:#010x} (expected \
                     {:#010x}), compared {} of {}, mismatch {:?} — {named}",
                    e.rev_submit.semaphore,
                    e.rev_payload,
                    e.rev_submit.gp_get,
                    e.rev_submit.gp_put,
                    e.rev_before,
                    expected_before,
                    e.rev_bytes_compared,
                    e.bytes,
                    e.rev_mismatch
                );
                false
            };
            fwd && rev
        }
        // ★ R25's arm B, unchanged in meaning: "refused" and "refused with
        // NV_ERR_INSUFFICIENT_PERMISSIONS" are different findings.
        Err(e) => {
            println!(
                "FAIL  R32 osdesc         = alloc/map/copy refused {e:?} — if this is euid 0 \
                 the framebuffer-memfd route needs another door; if it is not, the SANDBOX \
                 POLICY is the subject and this is a FINDING, not a licence to relax it."
            );
            false
        }
    };
    let _ = rm.free(vas);
    verdict
}

/// ★★★ R26 — **will host RM build a channel whose GPFIFO ring is at an address WE
/// dictate, and will the engine then FETCH from it?**
///
/// R25 established that guest-RAM-shaped memory reaches a real host GPU's MMU at a VA we
/// choose. That was a *data* mapping, read by a copy engine as an operand. This is the
/// same question one plane over, about the **control** plane: a channel's ring is the one
/// mapping hardware's host unit walks by itself, from an address baked into
/// `NV_CHANNEL_ALLOC_PARAMS::gpFifoOffset` at allocation time. A shadow-forwarded channel
/// has to name the *guest's* ring address there, so if RM insists on choosing, the design
/// is unbuildable and everything downstream of it is wasted.
///
/// ## ★★★ The evidence bar is TWO facts, and the second is the whole reason for the rung
///
/// ```text
///   1. the ring landed where we asked   -- read back from ChannelParts, not from Ok(())
///   2. the GPU CONSUMED a ring entry    -- GP_GET advanced, and the semaphore released
/// ```
///
/// ⊘ **Fact 1 alone is the R25 tautology wearing a different hat.** `alloc_channel_at`
/// returning `Ok` is the thing under test; checking that it returned `Ok` measures
/// nothing. Worse, fact 1 can hold *while the channel is dead*: RM records a mapping at
/// our address, the channel allocates, the token is minted, and hardware never fetches a
/// byte — the exact `userdOffset` shape the C spent M5.47 on, which produced **zero
/// utilisation and no Xid**. So the rung submits, and `GP_GET` — the one word in this
/// crate hardware writes and we do not — has to move.
///
/// ★ Hence the four arms below are not "did it work"; they are four *different* things
/// that can be true, and the interesting one is the **last** — the inert channel, which is
/// the only arm every other check in this file would score as a pass.
///
/// ```text
///   ★     R26 dictated ring   = asked X, RM placed X, GP_GET advanced, sem released  -> a PORT
///   FAIL  R26 place           = PlacementRefused want X got Y     -> RM chooses; shadow-forward is dead
///   FAIL  R26 alloc           = RM refused the channel outright   -> the address is legal to ASK and not to USE
///   ??    R26 inert channel   = placed at X, but GP_GET never moved -> ★ THE CELL A "did the alloc
///                                                                      succeed?" TEST SCORES GREEN
/// ```
///
/// ## ⊘ What it cannot see
///
/// - **Not a guest VA.** The address is one *we* chose, deliberately neither R25's
///   `0x3_0040_0000` nor R9's constant, so a pass cannot be a remembered address. Whether
///   a *guest's* VA is acceptable is a question about the number, and this rung only
///   establishes that the number is ours to pick.
/// - **Not the guest's ring LAYOUT.** `alloc_channel_at` places the isolate's own 64 KiB
///   ring object; `gpFifoOffset` is still derived from our `GPFIFO_OFFSET`. A guest ring
///   is guest memory with a guest layout, and that is the next increment.
/// - **Not a host fault.** If the engine never fetched, this rung cannot tell "the ring
///   was unreachable" from "the ring was reachable and empty". That distinction lives in
///   the **host** `dmesg`, and `scripts/bench/host_xid_watch.sh` is what reads it — this
///   rung is meant to be run inside that watcher, which is why it prints nothing about
///   faults itself rather than printing a guess.
fn dictated_ring_probe(rm: &mut HostRmBackend, gpu: u32) -> bool {
    // ★ Neither R25's `0x3_0040_0000` nor R9's constant: a rung that passes at an address
    // some earlier rung already proved is a rung that may be reading a remembered answer.
    // 64 KiB-aligned because that is the ring object's size and RM's device-local
    // granularity; an unaligned ask would be refused for the alignment and read as a
    // refusal of the *idea*.
    const RING_AT: u64 = 0x0000_0004_1100_0000;
    // Neither 0 (the sentinel `submit_semaphore_probe` writes first) nor a plausible token.
    const PAYLOAD: u32 = 0x1DEA_0026;

    println!(
        "info  R26 dictated ring   = GPU {gpu}, euid {} — allocate a host channel whose \
         RING OBJECT is placed at {RING_AT:#018x} BY US, then make the engine fetch from it",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R26 the bar is TWO facts: the placement RM reports back, and GP_GET moving. \
         ⊘ `Ok(())` from the call under test is not one of them"
    );

    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R26 vaspace         = the rung needs its own address space");
        return false;
    };
    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  R26 engine          = COPY0 is not expressible");
        let _ = rm.free(vas);
        return false;
    };

    let verdict = match rm.alloc_channel_at(vas, engine_type, Some(GpuVa(RING_AT))) {
        Ok((chan, token)) => {
            // ★★ Fact 1, read back from the connection's record of RM's [OUT] `dmaOffset`
            // rather than from the call's return value. Two parties, not one.
            let got = rm.channel_ring_va(chan);
            if got != Some(RING_AT) {
                println!(
                    "FAIL  R26 place           = the alloc succeeded but the recorded ring VA \
                     is {got:?}, not {RING_AT:#018x} — `alloc_channel_at` accepted a \
                     placement it should have refused"
                );
                let _ = rm.free(chan);
                let _ = rm.free(vas);
                return false;
            }
            println!(
                "ok    R26 placement       = RM reports the ring at {RING_AT:#018x} AS ASKED \
                 (token {token:#010x}) — necessary, and NOT yet sufficient"
            );
            if let Err(e) = rm.schedule(chan) {
                println!("FAIL  R26 schedule        = {e:?} (the channel exists and cannot run)");
                let _ = rm.free(chan);
                let _ = rm.free(vas);
                return false;
            }
            // ★★★ Fact 2. The pushbuffer, the GPFIFO entry and the semaphore are all read
            // by hardware at `ring_va + <offset>` — i.e. at OUR address. If the host MMU
            // could not resolve it, nothing here can succeed.
            match rm.submit_semaphore_probe(chan, token, PAYLOAD, std::time::Duration::from_secs(2))
            {
                Ok(o) if o.landed(PAYLOAD) => {
                    println!(
                        "★     R26 dictated ring   = ring placed at {RING_AT:#018x} AS ASKED, \
                         GP_GET {} caught GP_PUT {}, sem {:#010x} (want {PAYLOAD:#010x}) — \
                         the GPU FETCHED from an address we chose",
                        o.gp_get, o.gp_put, o.semaphore
                    );
                    true
                }
                Ok(o) if o.gp_get == 0 && o.gp_put != 0 => {
                    // ★★★ THE CELL. Everything a "did the alloc succeed?" test looks at is
                    // green, and the channel is inert.
                    println!(
                        "??    R26 INERT CHANNEL   = placed at {RING_AT:#018x} as asked and the \
                         engine NEVER FETCHED: GP_GET {} GP_PUT {} sem {:#010x}. ⊘ This is NOT \
                         a placement failure and NOT a success — the ring is where we said and \
                         hardware did not read it. Read the host Xid log: an `Xid 31 FAULT_PDE` \
                         says the address was unreachable, a CLEAN log says it was reachable \
                         and something else (USERD, the token, the schedule) is wrong",
                        o.gp_get, o.gp_put, o.semaphore
                    );
                    false
                }
                Ok(o) => {
                    println!(
                        "FAIL  R26 submit          = placed at {RING_AT:#018x}, entry FETCHED \
                         (GP_GET {} GP_PUT {}) and the methods did not release: sem {:#010x}, \
                         want {PAYLOAD:#010x}",
                        o.gp_get, o.gp_put, o.semaphore
                    );
                    false
                }
                Err(e) => {
                    println!("FAIL  R26 submit          = {e:?}");
                    false
                }
            }
            .tap_free(rm, chan)
        }
        Err(RmError::PlacementRefused { want, got }) => {
            println!(
                "FAIL  R26 place           = asked {want:#018x}, RM chose {got:#018x} \
                 (DMA_OFFSET_FIXED_TRUE not honoured for a CHANNEL RING) — a shadow-forwarded \
                 channel cannot name the guest's ring, and the design must change"
            );
            false
        }
        Err(e) => {
            println!(
                "FAIL  R26 alloc           = the channel was refused {e:?} at ring \
                 {RING_AT:#018x}. ⊘ Do NOT read this as `RM chooses the address` — that is \
                 the arm above, and it looks different. This says the ADDRESS was legal to \
                 ask for and the channel was not built over it"
            );
            false
        }
    };
    let _ = rm.free(vas);
    verdict
}

/// ★★★ The negative control for R26 — **occupy the address first**, then ask for it.
///
/// ⊘ A green whose red is unreachable proves nothing, and R26's green has a specific way of
/// being vacuous: if `channel_ring_va` merely echoed the address we asked for, every run
/// would pass at every address and the rung would be measuring its own argument. R25 was
/// bitten by precisely this shape one plane over (`§16.67.4`: `65536 of 65536` printed from
/// one variable twice), so the check has to be *watched to fail*.
///
/// This maps a device-local object at `RING_AT` **first**, in the same `Vas`, and then asks
/// for a channel ring there. The address is now taken, so one of two things must happen and
/// **either one is the control firing**:
///
/// - RM refuses the fixed map outright — the address is enforced by the driver;
/// - RM relocates and `alloc_channel_at` converts that into `PlacementRefused` — the
///   placement check is enforced by us.
///
/// ⊘ **A third outcome would be a finding, not a pass.** If the channel is built at
/// `RING_AT` while another object is mapped there, then two objects share one GPU VA, the
/// "address identity" the whole data plane rests on does not hold, and R26's green means
/// something much weaker than it says. That arm is printed as `FAIL`, loudly, and it is the
/// reason this control is worth its lines.
fn dictated_ring_negative(rm: &mut HostRmBackend, gpu: u32) -> bool {
    const RING_AT: u64 = 0x0000_0004_1100_0000;
    println!(
        "info  R26n neg control    = GPU {gpu}, euid {} — OCCUPY {RING_AT:#018x} first, then \
         ask for a channel ring at the same address. The control fires if the ask is REFUSED",
        kayfabe_linux_raw::geteuid()
    );
    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R26n vaspace        = the rung needs its own address space");
        return false;
    };
    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  R26n engine         = COPY0 is not expressible");
        let _ = rm.free(vas);
        return false;
    };
    // ★ The squatter is the same size as a ring object, so the collision is total rather
    // than a partial overlap RM might legitimately place around.
    let squatter = match rm.alloc_probe_local(0x1_0000) {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  R26n squatter       = could not allocate the occupying object: {e:?}");
            let _ = rm.free(vas);
            return false;
        }
    };
    let verdict = match rm.map_gpu_va(vas, squatter, 0x1_0000, GpuVa(RING_AT)) {
        Ok(va) if va == RING_AT => {
            println!("ok    R26n occupied       = {RING_AT:#018x} is now taken by another object");
            match rm.alloc_channel_at(vas, engine_type, Some(GpuVa(RING_AT))) {
                Err(RmError::PlacementRefused { want, got }) => {
                    println!(
                        "★     R26n CONTROL FIRED  = the channel ring was RELOCATED to \
                         {got:#018x} from {want:#018x} and `alloc_channel_at` REFUSED it — \
                         the placement check is not an echo of our own argument"
                    );
                    true
                }
                Err(e) => {
                    println!(
                        "★     R26n CONTROL FIRED  = RM refused the channel at an occupied \
                         {RING_AT:#018x} with {e:?} — the address is enforced by the driver, \
                         so R26's green is a fact about RM and not about our formatting"
                    );
                    true
                }
                Ok((chan, _)) => {
                    println!(
                        "FAIL  R26n TWO OBJECTS    = a channel ring was built at \
                         {RING_AT:#018x} while another object is mapped there. ⊘ This is not \
                         a control failure, it is a FINDING: one GPU VA now names two \
                         objects, and address identity does not hold the way #102 assumes"
                    );
                    let _ = rm.free(chan);
                    false
                }
            }
        }
        Ok(va) => {
            println!(
                "FAIL  R26n occupied       = the squatter asked {RING_AT:#018x} and landed at \
                 {va:#018x}; the control never got to run"
            );
            false
        }
        Err(e) => {
            println!("FAIL  R26n occupied       = could not place the squatter: {e:?}");
            false
        }
    };
    // ★ The squatter is NOT freed by freeing the `Vas`: `free` takes a channel down with its
    // address space, and an occupying memory object is neither. A diagnostic that is only
    // ever run once still has to free it — the next person to put this in a loop inherits
    // whatever it left behind, and a VA that is still occupied would make the control
    // "fire" for the wrong reason on iteration two.
    let _ = rm.free(squatter);
    let _ = rm.free(vas);
    verdict
}

/// A `bool` that frees a channel on its way out, so the four submit arms above can each be
/// a single expression without four copies of the teardown — the shape that grows a leak
/// on whichever arm gets edited last.
trait TapFree {
    fn tap_free(self, rm: &mut HostRmBackend, chan: kayfabe_isolate::HostHandle) -> Self;
}
impl TapFree for bool {
    fn tap_free(self, rm: &mut HostRmBackend, chan: kayfabe_isolate::HostHandle) -> Self {
        let _ = rm.free(chan);
        self
    }
}

/// The sentinel the race target holds before any engine runs. Neither `0` nor `1`: a zero
/// would be indistinguishable from freshly-allocated memory, and the point of the sentinel
/// is that "not MAGIC" must mean "nothing wrote here", not "we cannot tell".
const RACE_SENTINEL: u32 = 0xDEAD_0000;
/// The payload the `SEM_RELEASE` writes. Un-forgeable by anything else in this process.
const RACE_MAGIC: u32 = 0x1DEA_0031;
/// The value the CPU stores to let the acquire through. Not `1`, for `RACE_SENTINEL`'s
/// reason — a fence that passes on a value some other writer could plausibly leave is not
/// a fence.
const RACE_FENCE_VAL: u32 = 0xFACE_0377;
/// Bytes of the race target object.
const RACE_TARGET_BYTES: u64 = 0x1000;

/// What one arm of the late-map race actually did.
///
/// ⊘ Five outcomes and not a `bool`, because three of them are *"the experiment did not
/// run"* rather than *"the experiment ran and failed"* — and collapsing those into `false`
/// is exactly how a rung reports a finding it never measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RaceOutcome {
    /// ★ `MAGIC` landed: the release executed and reached the target.
    Landed { gp_get: u32, gp_put: u32 },
    /// ⊘ The target already held `MAGIC` before the fence was written — the acquire did
    /// **not** block, so the window this rung exists to open was never open.
    NeverBlocked,
    /// ⊘ The entry was never fetched (`gp_get == 0`, `gp_put != 0`). USERD, the token or
    /// the schedule — not a mapping question at all.
    NeverFetched { gp_get: u32, gp_put: u32 },
    /// The fence was written and `MAGIC` never arrived. **This is the interesting red**,
    /// and it is still two hypotheses until the host Xid log is read.
    Stalled {
        gp_get: u32,
        gp_put: u32,
        fence: u32,
        target: u32,
    },
}

impl RaceOutcome {
    /// Only [`RaceOutcome::Landed`] is a pass. Named so no caller has to remember which of
    /// the four non-passes are "did not run".
    fn landed(self) -> bool {
        matches!(self, RaceOutcome::Landed { .. })
    }
}

/// Run one arm of the late-map race and say what hardware did.
///
/// `map_before_doorbell` is the ONLY difference between the positive control (arm A) and
/// the experiment (arm B). Everything else — the channel, the pushbuffer, the sentinel,
/// the fence, the timeouts — is byte-identical, so a difference in outcome is attributable
/// to the mapping's *timing* and to nothing else.
fn late_map_arm(
    rm: &mut HostRmBackend,
    arm: &str,
    ring_at: u64,
    target_at: u64,
    map_before_doorbell: bool,
    engine_type: u32,
) -> Result<RaceOutcome, RmError> {
    let vas = rm.alloc_vaspace()?;
    let mut cleanup_va: Option<u64> = None;
    let mut chan_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut mem_h: Option<kayfabe_isolate::HostHandle> = None;

    let mut go = || -> Result<RaceOutcome, RmError> {
        let (chan, token) = rm.alloc_channel_at(vas, engine_type, Some(GpuVa(ring_at)))?;
        chan_h = Some(chan);
        // Fact 1, from the connection's record of RM's [OUT] `dmaOffset`, not from the
        // call's return value.
        let got = rm.channel_ring_va(chan);
        if got != Some(ring_at) {
            return Err(RmError::PlacementRefused {
                want: ring_at,
                got: got.unwrap_or(0),
            });
        }
        rm.schedule(chan)?;

        // The target object, sentinel-filled through a CPU mapping that is dropped before
        // anything is submitted.
        let mem = rm.alloc_probe_local(RACE_TARGET_BYTES)?;
        mem_h = Some(mem);
        rm.fill_words(mem, RACE_TARGET_BYTES, RACE_SENTINEL, 0)?;

        if map_before_doorbell {
            let va = rm.map_local_at(vas, mem, RACE_TARGET_BYTES, Some(target_at))?;
            cleanup_va = Some(va);
            if va != target_at {
                return Err(RmError::PlacementRefused {
                    want: target_at,
                    got: va,
                });
            }
            println!("info  {arm} map            = BEFORE the doorbell, at {target_at:#018x}");
        } else {
            println!("info  {arm} map            = deferred until AFTER the doorbell");
        }

        // The fence starts closed. ⚠ From here until the fence is written the channel is
        // stalled inside the acquire; every exit below must still reach the cleanup that
        // frees it.
        rm.ring_store_u32(chan, RACE_FENCE_OFFSET, 0)?;
        rm.submit_fenced_release(
            chan,
            token,
            RACE_FENCE_OFFSET,
            RACE_FENCE_VAL,
            target_at,
            RACE_MAGIC,
        )?;

        // ★★★ THE CONTROL THAT MAKES ARM B MEAN ANYTHING. Give the engine time to fetch
        // and reach the acquire, then prove it is PARKED there: the target must still hold
        // the sentinel, and the entry must have been fetched.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let parked = rm.read_words_independently(mem, RACE_TARGET_BYTES, &[0])?[0];
        let (gp_get, gp_put) = rm.userd_cursors(chan)?;
        if parked == RACE_MAGIC {
            return Ok(RaceOutcome::NeverBlocked);
        }
        // ⊘⊘⊘ CORRECTED 2026-09-06 (w379) — **THIS CHECK USED TO RETURN HERE, AND THAT
        // ABORTED THE EXPERIMENT BEFORE IT RAN.**
        //
        // `GP_GET` is USERD dword 34, and **hardware is its only writer**. Measured w379,
        // over the whole workspace: `USERD_GP_GET` appears at five sites and **every one is
        // a read** — the single store to USERD anywhere in this tree is `USERD_GP_PUT`
        // (`rm.rs`, `submit_entry`). On the Mode-2 emulated device there is no PBDMA and no
        // code that stores that word, so `gp_get == 0` is the **only** value it can hold, on
        // every configuration — whether the doorbell was served locally, forwarded, or
        // refused.
        //
        // ⇒ Returning `NeverFetched` on that predicate made a guest run report a
        // HARDWARE-ONLY diagnosis — *"USERD, the token or the schedule"* — about a cursor that
        // structurally has no writer, and it did so **before the fence was written**, so
        // arm A's positive control failed without the release ever being attempted. That is
        // this repo's *"a probe's private constant is the caller's trap"* class, and the fix
        // is ordering: **the primary observable decides; the cursor only explains a red.**
        //
        // ⚠ The predicate itself is kept, unchanged, below the poll. On real hardware it
        // still separates *"the entry was never fetched"* from *"the release never ran"* — it
        // simply may no longer pre-empt the measurement it exists to qualify.
        if gp_get == 0 && gp_put != 0 {
            println!(
                "⚠     {arm} cursor         = GP_GET has not advanced. ⊘ On a device with no \
                 PBDMA nothing ever writes that word, so this is NOT evidence about the \
                 doorbell — the experiment continues and the TARGET decides"
            );
        }
        println!(
            "ok    {arm} parked         = target still {parked:#010x} (sentinel), \
             GP_GET {gp_get} GP_PUT {gp_put} — the channel is STALLED IN THE ACQUIRE, \
             which is what makes the window below real"
        );

        // 5 — the mapping, made while the channel is already running and after its only
        // doorbell has been rung.
        if !map_before_doorbell {
            let va = rm.map_local_at(vas, mem, RACE_TARGET_BYTES, Some(target_at))?;
            cleanup_va = Some(va);
            if va != target_at {
                return Err(RmError::PlacementRefused {
                    want: target_at,
                    got: va,
                });
            }
            println!(
                "ok    {arm} late map       = mapped at {target_at:#018x} AFTER the \
                 doorbell, with the channel running. NO further doorbell is sent"
            );
        }

        // 6 — open the fence with a plain CPU store. This is the ONLY thing that happens
        // between the mapping and the engine's first touch of `target_at`.
        rm.ring_store_u32(chan, RACE_FENCE_OFFSET, RACE_FENCE_VAL)?;

        // 7 — poll.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut target = rm.read_words_independently(mem, RACE_TARGET_BYTES, &[0])?[0];
        while target != RACE_MAGIC && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
            target = rm.read_words_independently(mem, RACE_TARGET_BYTES, &[0])?[0];
        }
        let (gp_get, gp_put) = rm.userd_cursors(chan)?;
        if target == RACE_MAGIC {
            // ★ The payload landed. ⊘ A pass **whatever the cursors say**: a release that
            // reached the target cannot have failed to be fetched, so a `gp_get` of 0 here
            // indicts the cursor and never the run.
            Ok(RaceOutcome::Landed { gp_get, gp_put })
        } else if gp_get == 0 && gp_put != 0 {
            // Reachable only now that the target is known NOT to hold the payload — which is
            // what makes this a qualifier on a red rather than a verdict in its own right.
            Ok(RaceOutcome::NeverFetched { gp_get, gp_put })
        } else {
            let fence = rm.ring_load_u32(chan, RACE_FENCE_OFFSET)?;
            Ok(RaceOutcome::Stalled {
                gp_get,
                gp_put,
                fence,
                target,
            })
        }
    };

    let out = go();

    // ⚠ Cleanup on EVERY path, error paths included: an acquire whose fence was never
    // written leaves the channel stalled, and freeing it is what reclaims the engine.
    if let Some(va) = cleanup_va {
        let _ = rm.unmap_local(vas, va);
    }
    if let Some(h) = mem_h {
        let _ = rm.free(h);
    }
    if let Some(h) = chan_h {
        let _ = rm.free(h);
    }
    let _ = rm.free(vas);
    out
}

/// ★★★★★ **W377 — DOES A MAPPING MADE *AFTER* THE DOORBELL REACH AN ALREADY-RUNNING
/// CHANNEL?**
///
/// The owner's scenario, made executable. A guest userspace client can queue work behind a
/// fence, ring once, then map memory and open the fence with a plain store — at which point
/// the engine touches an address that was **not mapped when the doorbell was rung**, and no
/// second doorbell is ever sent. If a hypervisor's publication is triggered by the doorbell,
/// it has already run and it ran too early.
///
/// This rung reproduces exactly that, from a raw client with no libcuda anywhere:
///
/// ```text
/// pushbuffer = [ SEM_ACQUIRE(fence, FENCE_VAL) ][ SEM_RELEASE(target, MAGIC) ]
/// ```
///
/// - **ARM A** maps the target BEFORE the doorbell. It is the positive control, it must
///   pass, and if it does not then arm B is measuring the harness rather than the driver.
/// - **ARM B** maps it AFTER. That is the question.
///
/// ⊘ **On bare metal arm B is expected to PASS**, and a pass is the useful answer: it is
/// what says the real driver makes a late mapping live to a running channel with no
/// submission-side signal, which is the property a doorbell-triggered publisher cannot
/// have. A *failure* here would mean real CUDA cannot do this either — a much bigger claim,
/// and one to distrust before believing.
fn late_map_race(rm: &mut HostRmBackend, gpu: u32) -> bool {
    // ★ Ring and target are ≥ 512 MiB apart, and the two arms use different regions again.
    // A target adjacent to its own ring could be covered by a large PTE the ring's mapping
    // already installed — which would make arm B pass for a reason that has nothing to do
    // with the late map. Regions: A-ring 0x4_4…, A-target 0x5_4…, B-ring 0x4_6…,
    // B-target 0x5_6…, all 64 KiB-aligned.
    const A_RING_AT: u64 = 0x0000_0004_4100_0000;
    const A_TARGET_AT: u64 = 0x0000_0005_4100_0000;
    const B_RING_AT: u64 = 0x0000_0004_6100_0000;
    const B_TARGET_AT: u64 = 0x0000_0005_6100_0000;

    println!(
        "info  W377 late-map race  = GPU {gpu}, euid {} — queue work behind a fence, ring \
         ONCE, then map the memory it will touch",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  W377 the bar        = ARM A (map before the doorbell) must PASS, or ARM B \
         is uninterpretable. ⊘ Neither arm's verdict is the ioctl's return value"
    );

    // ⊘ Resolved ONCE, before either arm allocates anything: an engine the ABI cannot name
    // is a refusal of the harness, not of the driver, and it must not be reported from
    // inside an arm where it would read as that arm's outcome.
    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  W377 engine         = COPY0 is not expressible");
        return false;
    };

    let a = late_map_arm(rm, "W377-A", A_RING_AT, A_TARGET_AT, true, engine_type);
    match &a {
        Ok(o) => println!("info  W377-A outcome      = {o:?}"),
        Err(e) => println!("FAIL  W377-A refused      = {e:?}"),
    }
    let a_ok = matches!(&a, Ok(o) if o.landed());
    if a_ok {
        println!(
            "ok    W377-A control      = MAGIC landed with the target mapped BEFORE the \
             doorbell — the channel, the fence and the release all work"
        );
    } else {
        println!(
            "??    W377-A CONTROL FAILED = the positive control did not land. ⊘ ARM B IS \
             UNINTERPRETABLE: a red there would be this harness, not the driver. Read the \
             outcome above — `NeverBlocked` means the acquire did not stall, \
             `NeverFetched` means USERD/token/schedule, `Stalled` means the release never \
             ran"
        );
    }

    // ★★★ w379 — THE MACHINE-READABLE VERDICT, and it is not decoration.
    // `scripts/bench/racemap_hook.sh` and `w377_racemap.sh` both grade by
    // `sed -n 's/^RACEMAP_ARM_A=//p'`, and until now **this rung never printed that line**:
    // a native run that passed 3/3 was graded `(E) UNMEASURED` because the verdict existed
    // only as prose. ⊘ A grader and a probe that disagree about the vocabulary produce the
    // *unmeasured* answer, which is the one that reads as nobody's fault.
    println!("RACEMAP_ARM_A={}", if a_ok { "PASS" } else { "FAIL" });

    let b = late_map_arm(rm, "W377-B", B_RING_AT, B_TARGET_AT, false, engine_type);
    match &b {
        Ok(o) => println!("info  W377-B outcome      = {o:?}"),
        Err(e) => println!("FAIL  W377-B refused      = {e:?}"),
    }

    if !a_ok {
        println!("??    W377 VERDICT        = UNINTERPRETABLE (positive control A failed)");
        // ⊘ Printed even here, so *"no ARM_B line at all"* keeps meaning *"the rung never
        // reached its own verdict"* rather than *"A failed"* — two states the grader's
        // `(E) UNMEASURED` and `(D) UNINTERPRETABLE` arms are there to keep apart.
        println!("RACEMAP_ARM_B=NOTRUN");
        return false;
    }

    match b {
        Ok(RaceOutcome::Landed { gp_get, gp_put }) => {
            println!(
                "★     W377 LATE MAP LANDED = a mapping created AFTER the doorbell, with \
                 the channel already running and NO second doorbell, was walked by the \
                 engine (GP_GET {gp_get} GP_PUT {gp_put}, target {RACE_MAGIC:#010x}). ⇒ \
                 The driver publishes it, and a doorbell-triggered publisher CANNOT see it"
            );
            println!("RACEMAP_ARM_B=PASS");
            true
        }
        Ok(RaceOutcome::NeverBlocked) => {
            println!(
                "??    W377 RACE NOT RUN   = the target held MAGIC before the fence was \
                 written, so the acquire never stalled and the window was never open. ⊘ \
                 NOT a pass and NOT a finding — the SEM_ACQUIRE encoding is the first \
                 suspect (`SEM_EXECUTE_ACQUIRE_32BIT` is an all-zero word, so an unwritten \
                 pushbuffer slot decodes as one)"
            );
            println!("RACEMAP_ARM_B=NOTRUN");
            false
        }
        Ok(RaceOutcome::NeverFetched { gp_get, gp_put }) => {
            println!(
                "??    W377 NEVER FETCHED  = GP_GET {gp_get} GP_PUT {gp_put} — the entry \
                 was never read. ⊘ Not a mapping result: USERD, the token or the schedule"
            );
            println!("RACEMAP_ARM_B=FAIL");
            false
        }
        Ok(RaceOutcome::Stalled {
            gp_get,
            gp_put,
            fence,
            target,
        }) => {
            println!(
                "FAIL  W377 LATE MAP LOST  = fence {fence:#010x} (want \
                 {RACE_FENCE_VAL:#010x}), target {target:#010x} (want {RACE_MAGIC:#010x}), \
                 GP_GET {gp_get} GP_PUT {gp_put}"
            );
            println!(
                "⚠     W377 TWO HYPOTHESES = a timeout is NOT a fault. If `fence` does not \
                 hold the wanted value our own store failed and the acquire was never \
                 satisfied. If it DOES, the release ran and could not reach the target — \
                 and ONLY the host Xid log distinguishes them: look for `Xid 31 … \
                 FAULT_PDE … ACCESS_TYPE_VIRT_WRITE` at {B_TARGET_AT:#018x}"
            );
            println!("RACEMAP_ARM_B=FAIL");
            false
        }
        Err(_) => {
            println!("RACEMAP_ARM_B=NOTRUN");
            false
        }
    }
}

/// ★★★★★ **T1 — THE BLOCKAGE-COVERAGE PROBE (`REQUIREMENTS_TARGET.md` R1, falsifier C1).**
///
/// > Owner, 2026-09-07: *"anything thats async blockage gives coverage. so kernel emulated
/// > channels, rpc, tlb invalidate. … what we can't block is passthrough channels."*
///
/// # 0. ⊘ WHAT THIS RUNG MEASURES, AND — SAY IT FIRST — WHAT IT CANNOT
///
/// **The guest half is a driver, not a verdict.** C1 is a statement about the *device's*
/// counters: *"how many mappings were used with no prior blockage point"*. Those counters
/// live in `kayfabe_mmu::blockage` and are printed by the shim as
/// `kayfabe: BLOCKAGE-COVERAGE … token=…` on every doorbell. This program cannot read them —
/// it is inside the guest and they are on the host.
///
/// ⇒ What it does is **drive the two populations the counter must separate**, and **bracket
/// them with markers** (`BLOCKAGE_PROBE_BEGIN` / `_END`) so a host-side grader can attribute
/// the device lines to a phase instead of to a whole boot. A rung that printed a
/// coverage verdict from in here would be inventing one.
///
/// # 1. The two phases, and why the second one is the point
///
/// | phase | what it does | what the device counter should show |
/// |---|---|---|
/// | **A — COVERED** | map the target, *then* ring the doorbell, then let the engine touch it | every use of that row attributed to a blockage point |
/// | **B — THE KNOWN-POSITIVE** | ring first, map *after*, open a CPU fence — the R1.3 residual | ★ `USES_UNCOVERED` **> 0** |
///
/// ★★★★★ **Phase B is not a bug being reproduced; it is the counter's known-positive.**
/// `a_census_zero_needs_a_known_positive` is this tree's most-repeated lesson: a
/// `USES_UNCOVERED=0` on a boot that never contained an uncoverable mapping is evidence of
/// nothing. Phase B is the one construction the blockage model *admits* it cannot cover
/// (R1.3), so it is the only thing in the workload that can make the counter move — and if
/// it does not move, **the instrument is broken, not the model proven**.
///
/// ⊘ That is why the two phases are one flag and not two. A run that could take only phase A
/// would produce the favourable half of a differential with nothing to compare it to.
///
/// # 2. ⊘ Which blockage points this exercises, honestly enumerated
///
/// Both phases are a **guest userspace** RM client, so `project.rs:311` classifies their
/// channels `Passthrough` and their **doorbells are not a blockage point at all** (R1.1's
/// fourth row: *"⊘ none, by design"*). What the phases *do* cross is:
///
/// - **GSP RPC** — every `NV_ESC_RM_ALLOC` / `NV_ESC_RM_MAP_MEMORY_DMA` this issues is served
///   by the emulated GSP while the guest blocks in `_issueRpcAndWait`. ⚠ Not all of them:
///   `w387` §2 measured `bSplitVasManagementServerClientRm` defaulting true, so a map can be
///   entirely local to the guest's own CPU-RM and never cross. **That is precisely the hole
///   C1 exists to size**, and this rung does not assume either way.
/// - **TLB invalidate** — the guest's CPU-RM writes BAR0 `0x00B8_30B0` after a map unless
///   `DEFER_TLB_INVALIDATION` was set, which this rung never sets.
/// - ⊘ **Emulated doorbell** — **NOT** exercised here, by construction. It needs a
///   guest-*kernel* channel (UVM's), which a raw client cannot allocate. A boot line whose
///   `armed=[emulated-doorbell=0 …]` on a `--blockage-coverage`-only run is therefore
///   **correct and expected**, not a red. Run it beside a CUDA workload to arm that one.
///
/// # 3. ★★ PRE-REGISTERED OUTCOMES — every one, so none reads as the favourable one
///
/// ```text
/// (P) A=PASS B=PASS  => both phases ran. The device line is interpretable, and B is the
///                       known-positive: `USES_UNCOVERED` MUST be > 0 or the counter is
///                       blind. ⇒ THE MEASURABLE OUTCOME.
/// (Q) A=PASS B=FAIL  => B faulted instead of completing. Still a valid known-positive for
///                       the FAULT side (R1.3's backstop), and it bounds the exposure —
///                       but the device counter may legitimately read 0, because the engine
///                       never got to use the row. Say which; do not grade it as (P).
/// (R) A=FAIL         => ⊘ UNINTERPRETABLE. The positive control did not complete, so the
///                       submit path is broken and B measures that, not coverage.
/// (S) no verdict     => ⊘ UNMEASURED. NOT a failure value. Say where it stopped.
/// ```
///
/// ⚠ **`(Q)` is not a defeat and `(P)` is not a pass.** Neither says anything about C1 on its
/// own: C1 is graded on the device's `USES_UNCOVERED` for phase A's rows. This rung's job is
/// to make that number *attributable*.
fn blockage_coverage(rm: &mut HostRmBackend, gpu: u32) -> bool {
    // ⊘ Distinct regions from `--late-map-race`'s four, so the two rungs can run in one
    // invocation without either one's target being covered by the other's large PTE — the
    // same reason that rung separates its own arms by ≥ 512 MiB.
    const A_RING_AT: u64 = 0x0000_0006_4100_0000;
    const A_TARGET_AT: u64 = 0x0000_0007_4100_0000;
    const B_RING_AT: u64 = 0x0000_0006_6100_0000;
    const B_TARGET_AT: u64 = 0x0000_0007_6100_0000;

    // ★★★★★ THE BRACKET. Everything the device prints between these two markers is this
    // rung's; outside them it is the boot's. ⊘ Printed on stdout by the guest and correlated
    // by the harness against the HOST log, which is why they carry a timestamp-free, unique
    // string rather than a counter: the two logs have different clocks and no shared
    // sequence (`a_recorder_that_prints_at_teardown` — order survives, interval does not).
    println!(
        "BLOCKAGE_PROBE_BEGIN gpu={gpu} euid={} — R1/C1. Phase A maps BEFORE the doorbell \
         (the covered population); phase B maps AFTER, with the channel already running \
         (R1.3's residual, and this counter's KNOWN-POSITIVE)",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  T1 the bar          = the VERDICT IS ON THE HOST. This program drives two \
         populations and brackets them; `kayfabe: BLOCKAGE-COVERAGE …` on the host log is \
         what C1 is graded on. ⊘ A green line here is not a green C1"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  T1 engine           = COPY0 is not expressible");
        println!("BLOCKAGE_PROBE_A=NOTRUN");
        println!("BLOCKAGE_PROBE_B=NOTRUN");
        println!("BLOCKAGE_PROBE_END outcome=(S)");
        return false;
    };

    // --- phase A: the covered population, and the positive control -----------------------
    println!("--- T1 phase A: MAP, THEN RING. Every mapping exists before the engine runs ---");
    let a = late_map_arm(rm, "T1-A", A_RING_AT, A_TARGET_AT, true, engine_type);
    match &a {
        Ok(o) => println!("info  T1-A outcome        = {o:?}"),
        Err(e) => println!("FAIL  T1-A refused        = {e:?}"),
    }
    let a_ok = matches!(&a, Ok(o) if o.landed());
    println!("BLOCKAGE_PROBE_A={}", if a_ok { "PASS" } else { "FAIL" });
    if !a_ok {
        println!(
            "??    T1 VERDICT          = UNINTERPRETABLE — the positive control did not \
             complete, so phase B would measure the submit path and not coverage"
        );
        println!("BLOCKAGE_PROBE_B=NOTRUN");
        println!("BLOCKAGE_PROBE_END outcome=(R)");
        return false;
    }
    println!(
        "ok    T1-A control        = the payload landed with the target mapped BEFORE the \
         doorbell. ⇒ every row this phase used was published before the channel could use \
         it, BY CONSTRUCTION of the workload — the device line says whether we SAW that"
    );

    // --- phase B: the residual, and the counter's known-positive -------------------------
    println!(
        "--- T1 phase B: RING, THEN MAP. No second doorbell — R1.3, the one case the \
         blockage points structurally cannot cover ---"
    );
    let b = late_map_arm(rm, "T1-B", B_RING_AT, B_TARGET_AT, false, engine_type);
    match &b {
        Ok(o) => println!("info  T1-B outcome        = {o:?}"),
        Err(e) => println!("FAIL  T1-B refused        = {e:?}"),
    }
    let b_landed = matches!(&b, Ok(o) if o.landed());
    if b_landed {
        println!("BLOCKAGE_PROBE_B=PASS");
        println!(
            "★     T1-B KNOWN-POSITIVE = a mapping created AFTER the doorbell, with the \
             channel already running and NO second doorbell, was reached by the engine. ⇒ \
             the device's `USES_UNCOVERED` MUST be > 0 for this phase. If it reads 0, the \
             COUNTER is blind — that is not evidence of coverage"
        );
        println!("BLOCKAGE_PROBE_END outcome=(P)");
    } else {
        println!("BLOCKAGE_PROBE_B=FAIL");
        println!(
            "⚠     T1-B FAULTED        = the late mapping did NOT reach the engine. That is \
             R1.3's backstop case and it BOUNDS the exposure — but it is a WEAKER \
             known-positive: the engine never used the row, so the device's \
             `USES_UNCOVERED` may legitimately read 0 for this phase. ⊘ Do not grade this \
             as outcome (P)"
        );
        println!("BLOCKAGE_PROBE_END outcome=(Q)");
    }
    // ⊘ The rung's own boolean is phase A's and phase A's only. Phase B has no failing
    // value — both of its arms are informative — so folding it into the return would make a
    // legitimate `(Q)` read as a broken rung.
    a_ok
}

/// ★★★★★ R30 — **is the isolate's own completion semaphore NAMEABLE from the address
/// space a guest channel is bound to?**
///
/// The owner's invariant is *"VMM state must never be placed where a guest VA can name
/// it"*. Until this rung, the only thing upholding it on the copy-engine path was a
/// sentence in `raw_map_dma`'s doc comment — *"memory the isolate allocated for itself,
/// which no guest ever names"* — and the audit that produced this rung
/// (`C: docs/design/s1_what_does_it_protect.md` §3) found the address is **RM-chosen,
/// which makes it unpredictable rather than unnameable.** Unpredictability is not a
/// boundary.
///
/// ## ★★ THREE arms, and the middle one is what makes the first mean anything
///
/// ```text
///   A  guest space  @ sem_va -> must be FREE       -- nothing of ours is there
///   B  control space@ sem_va -> must be OCCUPIED   -- the same call, watched to REFUSE
///   C  a CE channel BOUND TO THE GUEST SPACE reads sem_va -> must NOT resolve
/// ```
///
/// ⊘ **Arm A alone is the arm every "did it pass?" check would score green**, and it is
/// green for free if `probe_va` can never refuse. Arm B is the identical call against the
/// space our ring *is* in, so a run where B reports `Free` indicts the **instrument**, not
/// the placement — and the rung says so instead of passing.
///
/// ★ Arm C is the only one that asks **hardware**. Arms A and B are questions for RM's VA
/// allocator; C points a real copy engine, bound to the guest's own space, at `sem_va` and
/// reads what lands. Before the placement fix it returns the payload our last copy
/// released — a number that channel has no other way to obtain. ⚠ After the fix it must
/// **fault**, and the host `dmesg` will carry an `Xid 31 FAULT_PDE`: that is the boundary
/// working, and `scripts/bench/host_xid_watch.sh` is what should be reading the log while
/// this runs.
///
/// ⊘ **What a pass does NOT establish.** Nothing about the *guest's* materialized channel
/// ring, which is isolate-allocated memory that stays in the guest's space by design; and
/// nothing about any address other than this one. It is a statement about the copy-engine
/// control structures, which is the scope the rung claims and the whole scope it claims.
fn executor_vas_probe(rm: &mut HostRmBackend, gpu: u32, want_alias_arm: bool) -> bool {
    use kayfabe_isolate_host::rm::{GuestReach, VaProbe};

    println!(
        "info  R30 executor VAS    = GPU {gpu}, euid {} — build the isolate's OWN copy-engine \
         channel over a `Vas`, then ask whether its semaphore VA is nameable from that same \
         `Vas` (the space a GUEST channel is bound to)",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R30 the bar is THREE arms: A the guest space must be FREE at sem_va, B the \
         SAME call against the control space must REFUSE, C an engine bound to the guest \
         space must NOT resolve sem_va. ⊘ A alone is vacuous"
    );

    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R30 vaspace         = the rung needs its own address space");
        return false;
    };

    // ★ A real copy, for two reasons: it is the only thing that BUILDS the isolate's CE
    // channel (which is what the rung is about), and it leaves a payload in the semaphore
    // that arm C can recognise. It is also, incidentally, the R17 round-trip — so a
    // regression in the thing being changed fails here first.
    const PATTERN: u32 = 0xC0FF_EE30;
    match rm.prove_ce_copy(vas, PATTERN) {
        Ok(e) if e.copied() => println!(
            "ok    R30 CE round-trip   = {} bytes moved, dst[0] {:#010x} -> {:#010x} — the \
             isolate's own copy-engine channel now EXISTS, which is what the rest of this \
             rung is about",
            e.bytes, e.before, e.after
        ),
        Ok(e) => {
            println!(
                "FAIL  R30 CE round-trip   = the copy did not land (dst[0] {:#010x} -> \
                 {:#010x}, sem {:#010x}) — every arm below would be about a channel that \
                 does not work",
                e.before, e.after, e.submit.semaphore
            );
            let _ = rm.free(vas);
            return false;
        }
        Err(e) => {
            println!("FAIL  R30 CE round-trip   = {e:?}");
            let _ = rm.free(vas);
            return false;
        }
    }

    let Some(p) = rm.ce_control_placement(vas) else {
        println!(
            "FAIL  R30 placement       = the copy landed and no CE channel is recorded over \
             this `Vas` — the accessor and the copy disagree, and the accessor is what every \
             arm below reads"
        );
        let _ = rm.free(vas);
        return false;
    };
    let colocated = p.guest_space == p.control_space;
    println!(
        "{}  R30 spaces          = guest range {:#010x}, control range {:#010x}, ring \
         {:#018x}, sem {:#018x}, our last payload {:#010x}{}",
        if colocated { "??   " } else { "ok   " },
        p.guest_space,
        p.control_space,
        p.ring_va,
        p.sem_va,
        p.last_payload,
        if colocated {
            " — ★ THE SAME ADDRESS SPACE. This is the co-location defect stated as two \
             equal handles; the arms below say what it costs"
        } else {
            " — two different address spaces"
        }
    );

    // --- arm A: the guest-bound space must have nothing of ours where our RING is -------
    //
    // ★★ The probe is at the ring OBJECT's base and the ring object's size, not at
    // `sem_va`. [measured 2026-08-10, `vh`] RM maps device-local memory with 64 KiB big
    // pages: a fixed ask at `ring_va + 0x2000` is placed at `ring_va` whether the probe
    // object is 64 KiB or 4 KiB, so the allocator cannot be asked a finer question than
    // "is this 64 KiB region taken". ⊘ The semaphore lives at `+0x2000` INSIDE that object,
    // so "the object is not mapped here" is strictly stronger than "the word is not mapped
    // here" — the arm is not weakened by asking it this way, it is made answerable.
    let arm_a = match rm.probe_va(p.guest_space, p.ring_va, p.ring_bytes) {
        Ok(VaProbe::Free) => {
            println!(
                "ok    R30 arm A guest     = the isolate's {}-byte ring object at {:#018x} \
                 (semaphore {:#018x}) is UNCLAIMED in the guest-bound space — a fresh object \
                 took the address and RM reported it back",
                p.ring_bytes, p.ring_va, p.sem_va
            );
            true
        }
        Ok(VaProbe::Occupied(e)) => {
            println!(
                "FAIL  R30 arm A guest     = the isolate's ring at {:#018x} (semaphore \
                 {:#018x}) is ALREADY MAPPED in the space a guest channel is bound to \
                 ({e:?}) — VMM state is placed where a guest VA can name it, which is the \
                 invariant, violated",
                p.ring_va, p.sem_va
            );
            false
        }
        Ok(VaProbe::Relocated(got)) => {
            println!(
                "FAIL  R30 arm A guest     = asked {:#018x}, RM placed {got:#018x} — occupied, \
                 AND the fixed ask was a hint. ⊘ Two findings, not one",
                p.ring_va
            );
            false
        }
        Err(e) => {
            println!("FAIL  R30 arm A guest     = the probe could not allocate: {e:?}");
            false
        }
    };

    // --- arm B: THE CALIBRATION. The same call, against the space our ring IS in --------
    let arm_b = match rm.probe_va(p.control_space, p.ring_va, p.ring_bytes) {
        Ok(VaProbe::Occupied(e)) => {
            println!(
                "ok    R30 arm B control   = the SAME call REFUSES at {:#018x} in the control \
                 space ({e:?}) — the probe can detect occupancy, so arm A's `Free` is a \
                 measurement and not a constant",
                p.ring_va
            );
            true
        }
        Ok(VaProbe::Relocated(got)) => {
            println!(
                "ok    R30 arm B control   = the SAME call was RELOCATED to {got:#018x} rather \
                 than granted {:#018x} — occupancy detected, by the other of the two legal \
                 shapes",
                p.ring_va
            );
            true
        }
        Ok(VaProbe::Free) => {
            println!(
                "FAIL  R30 arm B control   = {:#018x} reads as FREE in the very space our ring \
                 is mapped in. ⊘ This does NOT say the placement is fine — it says the \
                 INSTRUMENT is broken, and arm A's answer is worth nothing this run",
                p.ring_va
            );
            false
        }
        Err(e) => {
            println!("FAIL  R30 arm B control   = the probe could not allocate: {e:?}");
            false
        }
    };

    // --- arm C: hardware's own answer ---------------------------------------------------
    let arm_c = if !want_alias_arm {
        println!(
            "info  R30 arm C           = NOT RUN (pass `--executor-vas-alias`). It provokes a \
             real host fault when the boundary HOLDS, so it is opt-in and belongs under \
             `scripts/bench/host_xid_watch.sh`"
        );
        true
    } else {
        // ⊘ `Vidmem`, NAMED: this is R30's NATIVE arm, and the vidmem notifier is the one
        // w287's known-positive was measured on. ⊘ Not a default — see `NotifierAperture`,
        // which refuses to make either arm a fallback.
        //
        // ⊘⊘ **SUPERSEDED 2026-08-13 (w289):** the parenthesis here used to add *"a sysmem
        // notifier was refused natively in both flag settings tried"*, as if that justified
        // the choice. It does not any more — both refusals were ours (`_NO_MAP` at the
        // allocation, the control node at the map; see `rm::MapNode`). The `Vidmem` choice
        // stands on w287's known-positive alone, which is a **weaker** warrant than it read
        // as, and R30 is free to move to `Sysmem` once this rung has measured it.
        match rm.probe_guest_reachability(
            vas,
            p.sem_va,
            kayfabe_isolate_host::rm::NotifierAperture::Vidmem,
            // ⊘ w309 — R30 is NOT one of the criterion-1 arms. `default()` is the committed
            // shape (dictated addresses, notifier present), so this call is byte-identical to
            // every run before w309 and the flags below cannot reach it.
            kayfabe_isolate_host::rm::ReachProbeArms::default(),
        ) {
            Ok(r) => match r.reach {
                GuestReach::ControlFailed => {
                    println!(
                        "??    R30 arm C control   = the POSITIVE CONTROL did not land (sem \
                         {:#010x}, GP_GET {} GP_PUT {}, moved {:#010x} want {:#010x}) — the \
                         probe was never issued and this run says NOTHING about reachability",
                        r.control.semaphore,
                        r.control.gp_get,
                        r.control.gp_put,
                        r.control_read,
                        r.control_want
                    );
                    false
                }
                GuestReach::Read { word, outcome } => {
                    let ours = word == p.last_payload;
                    println!(
                        "FAIL  R30 arm C REACHED   = a copy engine BOUND TO THE GUEST'S SPACE \
                         retired a read of {:#018x} and moved {word:#010x} (GP_GET {} GP_PUT \
                         {}). Our last payload was {:#010x} — {}",
                        p.sem_va,
                        outcome.gp_get,
                        outcome.gp_put,
                        p.last_payload,
                        if ours {
                            "★★★ THE SAME VALUE. The guest-bound engine read the isolate's \
                             own completion semaphore. The defect is not latent; it is \
                             MEASURED"
                        } else {
                            "a different value — the address RESOLVED in the guest's space \
                             either way, which is already the violation"
                        }
                    );
                    false
                }
                GuestReach::NotResolved(outcome) => {
                    println!(
                        "★     R30 arm C REFUSED   = the guest-bound engine did NOT retire a \
                         read of {:#018x} (sem {:#010x}, GP_GET {} GP_PUT {}) — the address \
                         does not resolve in the space a guest channel is bound to. ⚠ Expect \
                         one `Xid 31 FAULT_PDE` in the host dmesg for this channel; that is \
                         the boundary, not a bug",
                        p.sem_va, outcome.semaphore, outcome.gp_get, outcome.gp_put
                    );
                    true
                }
                GuestReach::Ambiguous { word, outcome } => {
                    println!(
                        "??    R30 arm C ambiguous = the destination changed to {word:#010x} \
                         and the engine did not release (sem {:#010x}, GP_GET {} GP_PUT {}). \
                         Neither arm is claimed",
                        outcome.semaphore, outcome.gp_get, outcome.gp_put
                    );
                    false
                }
            },
            Err(e) => {
                println!(
                    "FAIL  R30 arm C           = the probe could not be built: {e:?} (an error \
                     here is never a fault — nothing had been submitted)"
                );
                false
            }
        }
    };

    let _ = rm.free(vas);
    let verdict = arm_a && arm_b && arm_c;
    if verdict {
        println!(
            "★     R30 executor VAS    = the isolate's CE semaphore is NOT nameable from the \
             address space a guest channel is bound to"
        );
    }
    verdict
}

/// ★★★★★ **R33 — THE RAW CE CLIENT: a copy engine driven end to end with NO `libcuda`,
/// and the exact ioctl count it costs.**
///
/// The owner's design, 2026-08-12: *"create with manual ioctl calls a CE channel … a small
/// program that copies, maps, reads completions, and tests the entire
/// ring/pushbuffer/USERD/semaphore/ioctl surface using a raw client without libcuda. One
/// black-box layer removed. … Far fewer ioctls, so far less to break. And it can run on the
/// real host and later be part of a test."*
///
/// ## ⊘⊘ IT IS AN EXTRACTION, AND SAYING SO IS THE POINT
///
/// Nothing in the data path below is new. `alloc_vaspace` → `prove_ce_copy` is the ladder's
/// own R7/R17 pair, `probe_va` is R30's arms A/B, `probe_guest_reachability` is R30's arm C.
/// What is new is (a) the **census** — how many times this enters the driver, which nothing
/// measured before — and (b) that it is **one flag with no isolate, no sandbox, no second
/// channel and no concurrency rung**, so it is small enough to push into a guest and run
/// against an emulated GPU. A full ladder run cannot do that: it spawns a sandboxed child.
///
/// ## ★★★ WHICH ADDRESS SPACE EVERY MEASUREMENT IS IN — the caveat that decides what this means
///
/// *"Is this GPU VA mapped?"* is only a question **relative to a VAS**, and a probe channel
/// of our own asks it about **our** VAS. This rung therefore prints both handles RM assigned
/// (`guest range` / `control range` — `w229`'s executor split) and says, in the output, that
/// every verdict below is scoped to them. ⊘ **It cannot answer a question about a VA in
/// somebody else's page-table tree**, and in particular it says nothing about the PDB the GR
/// engine faults on in `cup2`: that channel is the guest driver's, with its own PDB, and a
/// probe in the wrong address space is this campaign's exact recorded failure shape.
///
/// ## The three arms, and the second is what makes the first mean anything
///
/// ```text
///   1  THE COPY      device memory moves, read back through an INDEPENDENT mapping,
///                    semaphore carries the declared payload, GP_GET reaches GP_PUT
///   2  VA-OCCUPIED   probe_va at the ring's own address, in the space it IS in -> OCCUPIED
///   3  VA-FREE       the SAME call at an address nothing was ever mapped at   -> FREE
/// ```
///
/// Arm 3 alone is green for free if the probe can never refuse; arm 2 is the identical call
/// **watched to fail**, which is what makes arm 3 a measurement rather than a constant.
///
/// `want_fault` adds a fourth arm that asks **hardware** the same question — a copy engine
/// pointed at an address nothing is mapped at, which must NOT retire. ⚠ It provokes a real
/// `Xid 31 FAULT_PDE` and kills its own channel, so it is opt-in and belongs under
/// `scripts/bench/host_xid_watch.sh`. Its own positive control runs first, so *"the probe
/// never retired"* and *"this channel never worked"* stay distinguishable.
///
/// ## ⊘ What a green run does NOT establish
///
/// - **Not that `libcuda`'s path works.** It removes `libcuda`; it does not simulate it.
/// - **Not anything about the guest's VAS**, per the caveat above.
/// - **Not throughput.** One 4 KiB copy, polled.
/// - The ioctl count is **this program's**, not a lower bound on what a copy costs: it
///   includes bring-up (`R0`–`R6`), the probes and the teardown, and the census says which.
///
/// ⊘⊘ **`notifier_aperture` is the caller's and it decides whether arm 4 measures at
/// all.** See `kayfabe_isolate_host::rm::NotifierAperture`: a VIDMEM notifier decodes to
/// `ErrorNotifier::Unreachable` on the guest path, so **no host notifier is attached** and
/// the arm runs, faults, and reports a quiet notifier that means nothing. It is printed on
/// every run, including the quiet ones, because that silence is the hazard.
/// ★★★★★ **THE ONE LINE A HARNESS GREPS FOR CRITERION 1 — exactly one per run.**
///
/// See [`kayfabe_isolate_host::rm::Crit1State`] for why this is an enumeration and not the
/// boolean guard it replaces: `w289g`'s guard PASSED while its VA-identity zeros were vacuous,
/// because the run had reached vacuity by a route the guard did not enumerate.
///
/// ⊘ It prints `VA-IDENTITY MEASURED = yes|no` **beside** the state, so a reader never has to
/// know which states are which to know whether the numbers mean anything.
fn crit1(state: kayfabe_isolate_host::rm::Crit1State) {
    println!(
        "★     R33 CRIT1 STATE     = {} | VA-IDENTITY MEASURED = {} — {}",
        state.as_str(),
        if state.va_identity_is_measured() {
            "yes"
        } else {
            "no ⊘ every VA-IDENTITY count on this run is VACUOUS"
        },
        state.why()
    );
}

fn ce_client(
    rm: &mut HostRmBackend,
    gpu: u32,
    want_fault: bool,
    // ★ w305 — arm 4 runs in arm 1's ALREADY-WORKING VAS rather than a fresh one. See
    // `--ce-client-fault-shared-vas`; `false` is the byte-identical committed default.
    want_fault_shared_vas: bool,
    // ★★★★★ w309 — the other two settable confounds. See `rm::ReachProbeArms`: every field's
    // committed default is `true`, so `--ce-client-fault` alone is byte-identical to w305.
    fault_arms: kayfabe_isolate_host::rm::ReachProbeArms,
    notifier_aperture: kayfabe_isolate_host::rm::NotifierAperture,
) -> bool {
    use kayfabe_isolate_host::rm::{GuestReach, VaProbe};
    use kayfabe_linux_raw::census;

    /// Neither zero nor the sentinel the destination is pre-filled with.
    const PATTERN: u32 = 0xC0FF_EE33;
    /// ★ An address nothing in this program ever maps, chosen far above every fixed
    /// placement the ladder uses (`0x2_0020_0000`, `0x4_1100_0000`) so *"free"* cannot be an
    /// accident of adjacency. 64 KiB — RM's big-page granularity for device-local memory, so
    /// the allocator cannot be asked a finer question than this (measured, `probe_va` docs).
    ///
    /// ⊘⊘ **THE FIRST CHOICE WAS `0x7_0000_0000` AND IT WAS THE PROBE'S OWN RING.**
    /// `[measured 2026-08-12, vh, run `r33_ce_client_fault`]` arm 4 asked whether that address
    /// was mapped, the engine **retired the read and moved `0x20018000`**, and the arm printed
    /// `RESOLVED`. Nothing had mapped it — except `probe_guest_reachability`, which places its
    /// own channel ring there **by design**, in the space it builds. The instrument read
    /// itself and the answer was indistinguishable from a real one.
    /// ⇒ The window is published now, and the assert below makes the collision
    /// **unrepresentable rather than merely avoided** — a comment saying "keep these apart" is
    /// exactly what was already there, one layer down, and it did not hold.
    const UNMAPPED_VA: u64 = 0x9_0000_0000;
    const UNMAPPED_LEN: u64 = 0x1_0000;
    const _: () = assert!(
        UNMAPPED_VA + UNMAPPED_LEN <= kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.0
            || UNMAPPED_VA >= kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.1,
        "the address arm 3/arm 4 probe must lie OUTSIDE the window probe_guest_reachability \
         dictates for its own ring and operands, or both arms measure the instrument"
    );

    println!(
        "info  R33 raw CE client   = GPU {gpu}, euid {} — a copy engine allocated, mapped, \
         submitted and completed through RAW RM IOCTLS ONLY. No libcuda is loaded by this \
         process",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R33 the bar is FOUR facts: the bytes moved (read back through a mapping that \
         is not the one written), the semaphore carries the DECLARED payload at the DECLARED \
         address, GP_GET reached GP_PUT, and the VA probe REFUSES where something is mapped. \
         ⊘ `Ok(())` from any call under test is not one of them"
    );

    // ★★★★★ THE READER'S KNOWN-POSITIVE, BEFORE ANY ROW IT WILL BE USED TO JUDGE.
    in_band_known_positive(rm, kayfabe_isolate::HostHandle::NULL);

    census::phase("R7 vaspace");
    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R33 vaspace         = the rung needs its own address space");
        return false;
    };

    // --- arm 1: THE COPY ----------------------------------------------------------------
    census::phase("R33 arm1 ce-copy");
    // ⊘⊘ `met_the_whole_bar()`, NOT `copied()`. See [`CeEvidence::met_the_whole_bar`]: the
    // ★ arm used to be gated on `copied()`, which checks the bytes and the semaphore and
    // NEVER compares the cursors — so `w283c` printed a ★ line reading `GP_GET 0 caught
    // GP_PUT 1` and returned `R33_RC=0`. The banner three lines up says the bar is FOUR
    // facts; the verdict implemented three.
    let ce1 = rm.prove_ce_copy(vas, PATTERN);
    let copied = match &ce1 {
        Ok(e) if e.met_the_whole_bar() => {
            println!(
                "★     R33 arm 1 COPY      = {} bytes moved: dst[0] {:#010x} -> {:#010x}, \
                 dst[last] {:#010x} (want {:#010x}), engine semaphore {:#010x} (declared \
                 {:#010x}), GP_GET {} caught GP_PUT {} — read back through an INDEPENDENT \
                 mapping (its own device node, its own mmap, a kernel-chosen address)",
                e.bytes,
                e.before,
                e.after,
                e.after_last,
                e.expect_after_last,
                e.submit.semaphore,
                e.payload,
                e.submit.gp_get,
                e.submit.gp_put,
            );
            true
        }
        Ok(e) => {
            println!(
                "FAIL  R33 arm 1 COPY      = dst[0] {:#010x} -> {:#010x} (want {:#010x}), \
                 dst[last] {:#010x} (want {:#010x}), semaphore {:#010x} (want {:#010x}), \
                 GP_GET {} GP_PUT {} — {}",
                e.before,
                e.after,
                e.expect_after,
                e.after_last,
                e.expect_after_last,
                e.submit.semaphore,
                e.payload,
                e.submit.gp_get,
                e.submit.gp_put,
                // ★★★★★ **NAME WHICH OF THE FOUR FAILED.** ⊘ The old text branched on the
                // cursors ALONE and so described a whole-submission failure even when the
                // bytes had moved and the semaphore had landed — which is precisely the
                // state `w283c` reached. A diagnosis that is true of one fact and printed
                // as if it were true of all four is how a partial pass reads as a total
                // failure, and it is the mirror of the ★ line's own defect.
                match (e.copied(), e.cursor_caught_up()) {
                    (true, false) =>
                        "★★★ THREE OF FOUR: the bytes MOVED and the semaphore carries the \
                         DECLARED payload at the DECLARED address — only GP_GET did not \
                         reach GP_PUT. ⊘ That cursor is THIS channel's own USERD; a \
                         forwarding path that executes the work on a DIFFERENT host channel \
                         cannot advance it, and this line is what says so",
                    (false, true) =>
                        "the entry WAS fetched and the methods did nothing: SET_OBJECT \
                         class, subchannel, or an operand that does not resolve",
                    (false, false) =>
                        "the entry was NEVER fetched: USERD, the doorbell token, or the \
                         schedule",
                    // Unreachable — `met_the_whole_bar()` is exactly this conjunction, so
                    // the ★ arm took it. Named rather than `unreachable!()`: a client that
                    // panics on a guest-reachable state is a DoS we hand the guest.
                    (true, true) =>
                        "⊘ ALL FOUR HELD AND THIS ARM STILL RAN — the verdict predicate and \
                         this diagnosis disagree, which is an instrument bug, not a result",
                }
            );
            false
        }
        Err(e) => {
            println!("FAIL  R33 arm 1 COPY      = {e:?}");
            false
        }
    };

    // ★★★★★ **THE JOIN LINE — printed on EVERY arm, pass or fail.**
    //
    // ⊘ `w288nc1` carried a host `Xid 31 … CE0 HUBCLIENT_CE1 faulted @ 0x1_20000000 …
    // FAULT_PTE ACCESS_TYPE_VIRT_READ` in the same boot as this client, and the `RESULT` could
    // only record it as unattributable — because the client never printed an address for
    // anything arm 1 touched. One greppable line with both operands fixes that permanently and
    // costs a run nothing. ⚠ Deliberately OUTSIDE the pass/fail match: a diagnostic that
    // prints only on the arm you expected is absent exactly when it matters.
    if let Ok(e) = &ce1 {
        println!(
            "info  R33 arm 1 OPERANDS  = src {:#018x} dst {:#018x} ({} bytes each, \
             device-local, in the operand space). ⇒ A HOST `Xid` naming either of these \
             addresses IS THIS SUBMISSION; one naming neither belongs to a different channel",
            e.src_va, e.dst_va, e.bytes
        );
    }

    // --- the address spaces, NAMED, before any probe is read ----------------------------
    census::phase("R33 arm2/3 va-probe");
    let placement = rm.ce_control_placement(vas);
    let (arm2, arm3) = match placement {
        None => {
            println!(
                "FAIL  R33 placement       = no CE channel is recorded over this `Vas`, so \
                 there is no address to probe and arms 2/3 are NOT MEASURED"
            );
            (false, false)
        }
        Some(p) => {
            println!(
                "ok    R33 ADDRESS SPACES  = operands in range {:#010x}; the channel's ring, \
                 USERD and semaphore in range {:#010x}{}; ring {:#018x}, semaphore {:#018x}. \
                 ⊘⊘ EVERY VERDICT BELOW IS SCOPED TO THESE TWO HANDLES and to no other page \
                 table — a guest channel's PDB is not asked anything by this rung",
                p.guest_space,
                p.control_space,
                if p.guest_space == p.control_space {
                    " (THE SAME SPACE)"
                } else {
                    " (two different spaces — the w229 executor split)"
                },
                p.ring_va,
                p.sem_va
            );
            // Arm 2 — the calibration. Watched to REFUSE.
            let a2 = match rm.probe_va(p.control_space, p.ring_va, p.ring_bytes) {
                Ok(VaProbe::Occupied(e)) => {
                    println!(
                        "★     R33 arm 2 OCCUPIED  = a fresh object asked for {:#018x} in range \
                         {:#010x} and RM REFUSED it ({e:?}) — the ring really is there, so the \
                         probe can detect occupancy and arm 3's answer is a measurement",
                        p.ring_va, p.control_space
                    );
                    true
                }
                Ok(VaProbe::Relocated(got)) => {
                    println!(
                        "★     R33 arm 2 OCCUPIED  = the fixed ask at {:#018x} in range {:#010x} \
                         was RELOCATED to {got:#018x} — occupancy detected, by the other of the \
                         two legal shapes",
                        p.ring_va, p.control_space
                    );
                    true
                }
                Ok(VaProbe::Free) => {
                    println!(
                        "FAIL  R33 arm 2 CONTROL   = {:#018x} reads FREE in range {:#010x} — the \
                         space our own ring is mapped in. ⊘ This does NOT say the address is \
                         free; it says THE INSTRUMENT IS BROKEN, and arm 3 is worth nothing \
                         this run",
                        p.ring_va, p.control_space
                    );
                    false
                }
                Err(e) => {
                    println!("FAIL  R33 arm 2 OCCUPIED  = the probe could not allocate: {e:?}");
                    false
                }
            };
            // Arm 3 — the question the owner wants answerable: is a GPU VA mapped?
            let a3 = match rm.probe_va(p.control_space, UNMAPPED_VA, UNMAPPED_LEN) {
                Ok(VaProbe::Free) => {
                    println!(
                        "★     R33 arm 3 FREE      = {UNMAPPED_VA:#018x} is UNCLAIMED in range \
                         {:#010x} — a fresh object took the address and RM reported it back. \
                         ⇒ THIS IS THE `is this GPU VA mapped?` PRIMITIVE, with both polarities \
                         calibrated on one run and NO DEBUGGER involved",
                        p.control_space
                    );
                    true
                }
                Ok(VaProbe::Occupied(e)) => {
                    println!(
                        "??    R33 arm 3 OCCUPIED  = {UNMAPPED_VA:#018x} is already mapped in \
                         range {:#010x} ({e:?}) — nothing in this rung put it there, so the \
                         chosen address collides with something RM reserves. Pick another; the \
                         run is not indicted",
                        p.control_space
                    );
                    false
                }
                Ok(VaProbe::Relocated(got)) => {
                    println!(
                        "??    R33 arm 3 RELOCATED = asked {UNMAPPED_VA:#018x}, RM placed \
                         {got:#018x} — occupied AND the fixed ask was a hint. Two findings"
                    );
                    false
                }
                Err(e) => {
                    println!("FAIL  R33 arm 3 FREE      = the probe could not allocate: {e:?}");
                    false
                }
            };
            (a2, a3)
        }
    };

    // ★★★★★ **ARM 6 — ASK RM ITSELF WHICH OF THESE VAs IT HOLDS A PTE FOR.**
    //
    // ⊘⊘ **THE INSTRUMENT IS CALIBRATED IN-RUN, BOTH POLARITIES, BEFORE ANY VERDICT.** An
    // oracle that answers "absent" for everything reads as a discovery, and this campaign has
    // paid for that shape more than once. So the first two questions have KNOWN answers:
    //   - the channel's own RING (`ring_va`) — arm 2 just watched RM REFUSE a placement there,
    //     so something IS mapped: this MUST come back present. If it does not, arm 6 measured
    //     nothing this run and says so instead of reporting the rest.
    //   - `UNMAPPED_VA` — arm 3 just watched RM hand that address out as FREE, so nothing is
    //     mapped: this MUST come back absent.
    // Only then are the two addresses under test read.
    //
    // ★★ **WHY THIS IS THE DIFFERENTIAL AND `probe_va` IS NOT.** `probe_va` asks the
    // ALLOCATOR ("may I have this address?"); this asks the PAGE TABLES ("do you hold a PTE?")
    // — and it takes the VA space as a parameter, so the guest arm and the native arm ask
    // about THE SAME NUMBER IN THE SAME NAMED SPACE. That is a join key, not an alignment
    // guess.
    //
    // ⚠ And see `DmaGetPteInfoParams`: this sees populate source (1) — bind-time RPC/ioctl
    // bindings — and is BLIND to the observed CE page-table write. A miss therefore does not
    // say "unmapped"; it says "not mapped BY THIS SOURCE", which is the fork the whole
    // question turns on.
    census::phase("R33 arm6 pte-info");
    if let Some(p) = placement {
        // ⊘ Three named states, never one boolean — `Err` is "the question was not asked".
        let say = |rm: &mut HostRmBackend,
                   label: &str,
                   space: kayfabe_isolate::HostHandle,
                   va: u64|
         -> Option<bool> {
            // ⊘⊘ **PDE, NOT PTE, AND THE NAME MATTERS.** `GET_PTE_INFO` is
            // `RMCTRL_FLAGS_RM_TEST_ONLY_CODE` and answers `NV_ERR_TEST_ONLY_CODE_NOT_ENABLED`
            // on a release driver — measured, then explained from source. This is the sibling
            // that is callable, and it reports whether a page TABLE covers the VA. Our fault
            // is `FAULT_PTE`, so PRESENT here is CONSISTENT WITH THE FAULT.
            match rm.pde_info(space, va) {
                Ok((Some(b), pdb)) => {
                    println!(
                        "      R33 arm 6 {label:<14} {va:#018x} = PDE PRESENT (pageSize {:#x}, \
                         ptePhysAddr {:#x}, addrSpace {:#x}, pdeFlags {:#010x}, pdbAddr \
                         {pdb:#x}) ⊘ a page TABLE covers this VA; says NOTHING about the leaf",
                        b.page_size, b.pte_phys_addr, b.pte_addr_space, b.pde_flags
                    );
                    Some(true)
                }
                Ok((None, pdb)) => {
                    println!(
                        "      R33 arm 6 {label:<14} {va:#018x} = PDE ABSENT (pdbAddr \
                         {pdb:#x}) — RM's descent found NO page table. ⊘ That is structurally \
                         a `FAULT_PDE`, and our operands fault `FAULT_PTE`"
                    );
                    Some(false)
                }
                Err(e) => {
                    println!(
                        "      R33 arm 6 {label:<14} {va:#018x} = ⊘ NOT ASKED ({e:?}) — the \
                         control refused or the reply did not decode. NOT the same as ABSENT"
                    );
                    None
                }
            }
        };
        println!(
            "info  R33 arm 6 PDE-INFO  = NV0080_CTRL_CMD_DMA_GET_PDE_INFO (0x801809) against \
             RM's OWN VA space objects. ⊘ It reports what RM BELIEVES it mapped (populate \
             source 1), NOT what hardware can resolve — never report a valid PTE here as \
             `hardware can reach it`"
        );
        // --- the calibration, printed first ------------------------------------------------
        let pos = say(rm, "CAL+ ring", vas, p.ring_va);
        let neg = say(rm, "CAL- free", vas, UNMAPPED_VA);
        let calibrated = pos == Some(true) && neg == Some(false);
        if calibrated {
            println!(
                "★     R33 arm 6 CALIBRATED = the SAME call answers PRESENT at the ring \
                 (which arm 2 proved occupied) and ABSENT at {UNMAPPED_VA:#018x} (which arm 3 \
                 proved free). ⇒ both polarities are reachable and the rows below are \
                 measurements"
            );
        } else {
            println!(
                "FAIL  R33 arm 6 CALIBRATION = ring -> {pos:?} (want Some(true)), free -> \
                 {neg:?} (want Some(false)). ⊘⊘ THE ROWS BELOW ARE NOT MEASUREMENTS THIS RUN. \
                 An oracle that cannot show both polarities cannot distinguish `nothing is \
                 mapped` from `I cannot see mappings`"
            );
        }
        // --- the addresses under test ------------------------------------------------------
        // ★ The operands the host CE faulted on, by exact address. `w289g` measured
        //   `Xid 31 ... faulted @ 0x1_20000000 ... FAULT_PTE ACCESS_TYPE_VIRT_READ` and
        //   `@ 0x7_00100000`; both are printed by this program, so these rows are the same
        //   numbers the host log names.
        if let Ok(e) = &ce1 {
            let _ = say(rm, "arm1 src", vas, e.src_va);
            let _ = say(rm, "arm1 dst", vas, e.dst_va);
        }
        let _ = say(
            rm,
            "probe ctrlsrc",
            vas,
            kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.0 + 0x10_0000,
        );
        let _ = say(
            rm,
            "probe ring",
            vas,
            kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.0,
        );
    } else {
        println!(
            "info  R33 arm 6 PTE-INFO  = NOT MEASURED — no CE channel is recorded over this \
             `Vas`, so there is no calibrated ring address to anchor the instrument on"
        );
    }

    // --- arm 4 (opt-in): ask HARDWARE, not RM's allocator --------------------------------
    let arm4 = if !want_fault {
        println!(
            "info  R33 arm 4           = NOT RUN (pass `--ce-client-fault`). It points a real \
             copy engine at an unmapped VA, which provokes `Xid 31 FAULT_PDE` and kills its \
             own channel — opt-in, and it belongs under scripts/bench/host_xid_watch.sh"
        );
        crit1(kayfabe_isolate_host::rm::Crit1State::ArmNotSelected);
        true
    } else {
        census::phase("R33 arm4 hw-fault");
        // ★ Its OWN address space, allocated after arms 1–3 have already been read: a
        // faulted channel must not be able to retract a verdict already printed.
        //
        // ★★★★★ **w305 — `--ce-client-fault-shared-vas` MAKES THE VAS A NAMED ARM, and the
        // reason it exists is that the ruling it tests is AMBIGUOUS.**
        //
        // `road_to_v1_after_cup2.md` §2 (2026-08-14) rules: *"Arm 4's operands live in a third
        // VAS because arm 4 put them there ⇒ the fix is one line in the probe: allocate its
        // control operands in the VAS of the channel it rings."*
        //
        // ⊘⊘ **READ LITERALLY, THAT FIX IS A NO-OP, and it is a no-op in this file's own
        // source.** `probe_guest_reachability(fvas, …)` maps every operand into
        // `narrow(fvas)` (`rm.rs:6976`, `:7036`, `:7044`) and creates the channel it rings on
        // **the same `fvas`** (`rm.rs:7080`). The operands are ALREADY in the VAS of the
        // channel that rings them; there is no line to change.
        //
        // ⇒ The only ACTIONABLE reading is a different claim: not *"same VAS as the channel"*
        //   (already true) but *"a VAS that has ALREADY CARRIED WORK"* — i.e. reuse `vas`,
        //   which arm 1 has just proven end to end in this very process, instead of a VAS
        //   that is fresh. That is what this flag selects, and it is opt-in so the default
        //   stays byte-identical to every committed run.
        //
        // ⚠ The cost of the shared arm is stated rather than hidden: arm 4 kills its channel,
        //   so on `shared` the fault lands in the space arms 1–3 used. Their verdicts are
        //   already PRINTED by this point, so nothing can be retracted — but a reader must
        //   not treat arms 1–3 as independent of arm 4 on that arm, and the banner says so.
        let shared = want_fault_shared_vas;
        let vas_arm = if shared {
            // ⊘ NOT a fresh allocation: the point is that this space has already carried a
            // retired copy on a channel this process rang.
            Ok(vas)
        } else {
            rm.alloc_vaspace()
        };
        match vas_arm {
            Err(e) => {
                println!("FAIL  R33 arm 4 vaspace   = {e:?}");
                false
            }
            Ok(fvas) => {
                // ⊘⊘ NAMED BEFORE THE VERDICT, because the verdict is meaningless without it:
                // on the default arm this is a THIRD address space — not arm 1's operand space
                // and not arms 2/3's control space. It is NOT a cross-check of arm 3, and an
                // earlier draft of this rung printed one as if it were.
                if shared {
                    println!(
                        "info  R33 arm 4 SPACE     = ★ SHARED — arm 1's OWN address space \
                         (range {:#010x}), REUSED rather than freshly allocated. This is the \
                         only actionable reading of road_to_v1_after_cup2.md §2: the operands \
                         were ALWAYS in the ringing channel's VAS (rm.rs:6976/7080), so the \
                         literal fix is a no-op; what changes here is that the space has \
                         ALREADY CARRIED RETIRED WORK. ⚠ arms 1-3's verdicts are already \
                         printed above and cannot be retracted, but on THIS arm they are not \
                         independent of the fault below",
                        fvas.raw()
                    );
                } else {
                    println!(
                        "info  R33 arm 4 SPACE     = a THIRD, freshly allocated address space \
                         (range {:#010x}) — NOT arm 1's operand space and NOT arms 2/3's control \
                         space. ⊘ Arms 3 and 4 ask the same question about the same NUMBER in \
                         DIFFERENT address spaces, so they can disagree without either being \
                         wrong, and neither corroborates the other",
                        fvas.raw()
                    );
                }
                // ★★★★★ **w309 — THE ARM MATRIX, PRINTED FROM THE VALUES ACTUALLY PASSED,
                // on ONE greppable line, BEFORE the probe runs.**
                //
                // ⚠ *An arm you set is not an arm in force.* w305's runner tried to recover
                // the arm by grepping the probe log for `ce-client-fault` and printed `[]` on
                // both boots, because the hook never echoes its args. This line is built from
                // `fault_arms` and `shared` themselves — the same values the probe is about to
                // use — so it cannot disagree with what ran.
                //
                // ⊘ **CHANNEL ORDINAL IS HELD AT `2` ON EVERY ARM AND IS NOT SETTABLE.** Arms
                // 2/3/6 read `ce_control_placement`, which does not exist until arm 1 has
                // built a channel, so making arm 4 the process's FIRST channel is a different
                // program rather than a flag. Named here so a reader does not mistake the
                // matrix for complete: three of the four confounds move, one does not.
                println!(
                    "info  R33 arm 4 CONFIG    = vas={} {} chan-ordinal=2 (HELD, not settable) \
                     engine=COPY0 fault-va={UNMAPPED_VA:#018x}. ⇒ THIS is what ran; it is \
                     built from the values passed to the probe, not from the flags a harness \
                     believes it set",
                    if shared {
                        "SHARED (arm 1's, already carried retired work)"
                    } else {
                        "FRESH (a third, never used)"
                    },
                    fault_arms.as_str(),
                );
                let out = match rm.probe_guest_reachability(
                    fvas,
                    UNMAPPED_VA,
                    notifier_aperture,
                    fault_arms,
                ) {
                    Ok(r) => {
                        // ★★★★★ PRINTED FIRST, because it says how to read everything below it.
                        crit1(r.crit1_state());
                        // ★★★★★ **THE MANDATE'S SECOND CLIENT — HOW THE PROGRAM ITSELF
                        // LEARNS, printed BEFORE the verdict it qualifies.**
                        //
                        // Three planes, each reported separately and never collapsed:
                        //   A  the ERROR NOTIFIER  — RM writes `NvNotification` into memory
                        //                            this process allocated and can read.
                        //   C  the NEXT IOCTL      — asked explicitly on the dead channel.
                        //   ⊘  ABSENCE             — the semaphore that never released, which
                        //                            is all a client had before w287.
                        //
                        // ⊘⊘ **A QUIET NOTIFIER IS NOT A PASS AND NOT A BUG — it is a
                        // measurement**, and it is printed with the reason it is ambiguous
                        // attached. `status == 0` is what an unwired notifier, a refused
                        // handle and a channel that never faulted all read as.
                        // ★★★★★ THE CONTROL, PRINTED FIRST — a fired notifier means nothing
                        // until the same bytes have been shown quiet while the channel was
                        // alive and working.
                        match r.notifier_before {
                            Some(b) if !b.fired() => println!(
                                "★     R33 arm 5 CONTROL   = the SAME 16 bytes read QUIET \
                                 (status {:#06x} info32 {:#010x}) AFTER the positive control \
                                 retired and BEFORE the fault was issued. ⇒ anything below is \
                                 a CHANGE on one channel in one run, not a value that was \
                                 always there",
                                b.status, b.except_type
                            ),
                            Some(b) => println!(
                                "FAIL  R33 arm 5 CONTROL   = the notifier ALREADY read \
                                 status {:#06x} info32 {:#010x} before the fault was issued — \
                                 the channel was killed by something this rung did not \
                                 provoke, and the reading below is NOT attributable to the \
                                 deliberate fault",
                                b.status, b.except_type
                            ),
                            None => println!(
                                "??    R33 arm 5 CONTROL   = the pre-fault read did not \
                                 happen, so a fired notifier below cannot be attributed to \
                                 the fault rather than to channel creation"
                            ),
                        }
                        match r.notifier {
                            Some(n) if n.fired() => println!(
                                "★     R33 arm 5 NOTIFIER  = PLANE A FIRED — the driver wrote \
                                 this process's OWN memory: status {:#06x}, info32 \
                                 {:#010x} (`ROBUST_CHANNEL_*`, the number a host log prints \
                                 as `Xid`), info16 engine {:#06x}, timestamp {:#018x}. ⇒ THIS \
                                 is how a raw client learns IN-PROCESS that its channel was \
                                 killed: it POLLS 16 bytes, no host log and no debugger",
                                n.status, n.except_type, n.engine_type, n.timestamp
                            ),
                            Some(n) => println!(
                                "⊘     R33 arm 5 NOTIFIER  = PLANE A QUIET — status {:#06x} \
                                 info32 {:#010x}. ⊘ The page was ZEROED before the channel \
                                 was told about it, so this is not a stale read; but a quiet \
                                 notifier cannot distinguish `hObjectError` refused, RM not \
                                 writing, and the channel not having been RC-killed. NOT a \
                                 pass and NOT a refutation",
                                n.status, n.except_type
                            ),
                            None => println!(
                                "FAIL  R33 arm 5 NOTIFIER  = the notifier could not be read \
                                 at all — plane A is UNMEASURED this run, which is a \
                                 different thing from measuring it quiet"
                            ),
                        }
                        // ★★★★★ **w288 TIER 2 — PLANE D: *WHERE*, and this is the ONLY
                        // plane that can answer it.**
                        //
                        // The notifier gives `status` / `info32` (the Xid code) / `info16`
                        // (the engine). It has **no address field**. So *"the guest observed
                        // THE SAME FAULT, BY IDENTITY"* cannot be claimed from planes A-C:
                        // they can say a channel died and which engine, never where.
                        //
                        // ⊘ Printed as one joinable line so a runner can match it against the
                        // host's own `Xid 31 … @ 0x… FAULT_PDE` in the SAME run.
                        match &r.fault_info {
                            Some(info) => {
                                let got = info.address();
                                // ★★★★★ **THE FREE ORACLE — VA IDENTITY.** Guest ranges are
                                // mapped at IDENTICAL host VAs, so the address a fault reports
                                // MUST equal the address the engine was pointed at. ⊘ BOTH
                                // numbers are printed on both arms: a check that prints only
                                // the one it likes cannot be re-read by anyone who doubts it.
                                if got == r.fault_va {
                                    println!(
                                        "★     R33 arm 5 WHERE     = PLANE D SPEAKS — \
                                         GET_MMU_FAULT_INFO addr={got:#018x} \
                                         (hi={:#010x} lo={:#010x}) faultType={:#x} \
                                         faultString={:?} | VA-IDENTITY HOLDS: asked \
                                         {:#018x}, reported {got:#018x}",
                                        info.addr_hi,
                                        info.addr_lo,
                                        info.fault_type,
                                        info.fault_string_lossy(),
                                        r.fault_va,
                                    );
                                } else {
                                    println!(
                                        "FAIL  R33 arm 5 WHERE     = ⊘⊘ VA-IDENTITY BROKEN — \
                                         the engine was pointed at {:#018x} and the fault is \
                                         reported at {got:#018x} (hi={:#010x} lo={:#010x}, \
                                         faultType={:#x}, faultString={:?}). ⚠ Guest ranges \
                                         are mapped at IDENTICAL host VAs, so these MUST be \
                                         equal; a difference means the identity this whole \
                                         port rests on does not hold, or the record belongs \
                                         to a different fault",
                                        r.fault_va,
                                        info.addr_hi,
                                        info.addr_lo,
                                        info.fault_type,
                                        info.fault_string_lossy(),
                                    );
                                }
                            }
                            // ⊘ UNMEASURED, and named as such. The control refused or did not
                            // decode; it is NOT "the fault had no address". ⚠ And it may not
                            // be retried: the record is cleared by the read, so a second ask
                            // would answer all-zero and report a fault at address 0.
                            None => println!(
                                "FAIL  R33 arm 5 WHERE     = PLANE D UNMEASURED — \
                                 `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` refused or did not \
                                 decode, so this run carries the fault's CODE and not its \
                                 ADDRESS. ⊘ Not retried: the record is cleared by reading it, \
                                 so a second ask would answer all-zero and that decodes as a \
                                 fault at address 0"
                            ),
                        }
                        match &r.post_fault_ioctl {
                            Ok(()) => println!(
                                "⊘     R33 arm 5 IOCTL     = PLANE C SILENT — \
                                 `GET_WORK_SUBMIT_TOKEN` on the FAULTED channel returned \
                                 `NV_OK`. ⇒ THE IOCTL PLANE CARRIES NOTHING: a client that \
                                 polls only return codes cannot learn it was killed, which \
                                 is why plane A is not optional"
                            ),
                            Err(e) => println!(
                                "★     R33 arm 5 IOCTL     = PLANE C SPEAKS — the next ioctl \
                                 on the faulted channel refused with {e:?}. A second, \
                                 independent guest-observable path"
                            ),
                        }
                        match r.reach {
                            GuestReach::ControlFailed => {
                                println!(
                                    "??    R33 arm 4 control   = the POSITIVE CONTROL did not land \
                                 (sem {:#010x}, GP_GET {} GP_PUT {}, moved {:#010x} want \
                                 {:#010x}) — the fault probe was never issued, so this run says \
                                 NOTHING about whether {UNMAPPED_VA:#018x} resolves. \
                                 ⊘ THE CONTROL'S OWN OPERANDS: src {:#018x} dst {:#018x}, ring \
                                 {:#018x} — a HOST `Xid` naming ANY of these is THIS FAILURE, \
                                 and `w288nc1` wrote one up as unattributable for want of \
                                 exactly this line",
                                    r.control.semaphore,
                                    r.control.gp_get,
                                    r.control.gp_put,
                                    r.control_read,
                                    r.control_want,
                                    kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.0 + 0x10_0000,
                                    kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.0 + 0x20_0000,
                                    kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.0
                                );
                                false
                            }
                            GuestReach::NotResolved(o) => {
                                println!(
                                    "★     R33 arm 4 FAULTED   = a copy engine pointed at \
                                 {UNMAPPED_VA:#018x} did NOT retire (sem {:#010x}, GP_GET {} \
                                 GP_PUT {}) while its positive control on the SAME channel did \
                                 — hardware agrees the VA is unmapped. ⚠ Expect one `Xid 31 \
                                 FAULT_PDE`; that is the control FIRING, not a bug",
                                    o.semaphore, o.gp_get, o.gp_put
                                );
                                true
                            }
                            GuestReach::Read { word, outcome } => {
                                println!(
                                    "FAIL  R33 arm 4 RESOLVED  = the engine READ {UNMAPPED_VA:#018x} \
                                 in range {:#010x} and moved {word:#010x} (GP_GET {} GP_PUT \
                                 {}). Something IS mapped there IN THAT SPACE. ⊘ This does NOT \
                                 contradict arm 3, which asked about a different address \
                                 space — the first suspect is THE PROBE'S OWN dictated window \
                                 {:#018x}..{:#018x} (`rm::REACH_PROBE_WINDOW`), and if the VA \
                                 is outside it, something else in this space claimed it",
                                    fvas.raw(),
                                    outcome.gp_get,
                                    outcome.gp_put,
                                    kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.0,
                                    kayfabe_isolate_host::rm::REACH_PROBE_WINDOW.1
                                );
                                false
                            }
                            GuestReach::Ambiguous { word, outcome } => {
                                println!(
                                    "??    R33 arm 4 ambiguous = the destination changed to \
                                 {word:#010x} and the engine did not release (sem {:#010x}, \
                                 GP_GET {} GP_PUT {}). Neither arm is claimed",
                                    outcome.semaphore, outcome.gp_get, outcome.gp_put
                                );
                                false
                            }
                        }
                    }
                    Err(e) => {
                        println!(
                            "FAIL  R33 arm 4           = the probe could not be built: {e:?} (an \
                             error here is never a fault — nothing had been submitted)"
                        );
                        crit1(kayfabe_isolate_host::rm::Crit1State::ProbeNotBuilt);
                        false
                    }
                };
                let _ = rm.free(fvas);
                out
            }
        }
    };

    census::phase("R33 teardown");
    let _ = rm.free(vas);
    census::phase("");

    let verdict = copied && arm2 && arm3 && arm4;
    if verdict {
        println!(
            "★     R33 raw CE client   = a copy engine was allocated, mapped, submitted and \
             COMPLETED with no libcuda in the process, and a GPU VA was probed in both \
             polarities"
        );
    } else {
        println!("FAIL  R33 raw CE client   = at least one arm above did not meet its bar");
    }
    verdict
}

/// Print the ioctl census: the total, the per-phase split, the per-`NV_ESC` histogram and
/// the full ordered sequence.
///
/// ★★★ **The number is the KERNEL'S, not ours.** It is taken at `CharDevice::ioctl`, the one
/// funnel every RM ioctl in the workspace passes through, so a call site that forgot to
/// register still counts — which is why the phase subtotals are printed **against** the
/// grand total rather than instead of it. A shortfall is ioctls issued outside any phase.
fn print_ioctl_census(what: &str) {
    let c = kayfabe_linux_raw::census::snapshot();
    println!("=== IOCTL CENSUS ({what}) ===");
    println!(
        "  total={} failed={} logged={} dropped={}{}",
        c.total,
        c.failed,
        c.log.len(),
        c.dropped,
        if c.dropped == 0 {
            ""
        } else {
            "  ⚠⚠ THE LOG IS A PREFIX, NOT THE SEQUENCE"
        }
    );
    let phased: u64 = c.by_phase().iter().map(|p| p.1).sum();
    println!(
        "  --- by phase (⊘ the shortfall against `total` is ioctls issued outside any phase):"
    );
    for (p, n) in c.by_phase() {
        println!("      {:>24}  {n}", if p.is_empty() { "(none)" } else { p });
    }
    println!("      {:>24}  {phased} of {} accounted", "SUM", c.total);
    println!("  --- by request, `_IOC_TYPE`/`_IOC_NR` (the driver's own NV_ESC number):");
    for ((magic, nr), n, failed) in c.by_request() {
        println!(
            "      magic {:#04x} nr {nr:>3} ({nr:#04x}) {:<26} x{n}{}",
            magic,
            nv_esc_name(magic, nr),
            if failed == 0 {
                String::new()
            } else {
                format!("   ({failed} refused)")
            }
        );
    }
    // ★★★★★ **THE IN-BAND VERDICT, COUNTED SEPARATELY FROM `failed`.**
    let mut in_band_refused = 0usize;
    let mut unreadable = 0usize;
    println!(
        "  --- THE SEQUENCE, in order (seq: nr name  size  phase  errno  RM-STATUS):\n      \
         ⊘⊘ `errno` is the SYSCALL's answer; `RM-STATUS` is what RM wrote INTO THE PARAMETER \
         STRUCT. They disagree by design, and `errno ok` beside a non-zero RM-STATUS is a \
         SILENTLY REFUSED ioctl — the exact shape `failed=0` cannot see"
    );
    for r in &c.log {
        let st = rm_status_of(r);
        if matches!(st, RmStatus::Refused(_)) {
            in_band_refused += 1;
        }
        if matches!(st, RmStatus::Truncated) {
            unreadable += 1;
        }
        println!(
            "      {:>4}: nr {:>3} {:<26} size {:>5}  {:<22} {:<10} {}",
            r.seq,
            r.nr,
            nv_esc_name(r.magic, r.nr),
            r.size,
            r.phase,
            if r.errno == 0 {
                "ok".to_string()
            } else {
                format!("errno {}", r.errno)
            },
            st.describe()
        );
    }
    println!(
        "  --- ★★★ IN-BAND VERDICT: {in_band_refused} ioctl(s) REFUSED BY RM WITH `errno == 0`, \
         {unreadable} unreadable. ⊘ Compare against `failed={}` above: a difference is the \
         census's own blind spot being measured",
        c.failed
    );
    println!("=== END IOCTL CENSUS ===");
}

/// What RM wrote into the parameter struct — the answer `errno` cannot carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RmStatus {
    /// `status == NV_OK`.
    Ok,
    /// `status != NV_OK` — **refused, with `ioctl(2)` returning 0**.
    Refused(u32),
    /// This escape carries no status field (`CHECK_VERSION_STR`, `REGISTER_FD`).
    NoStatusField,
    /// The retained snapshot is shorter than the field's offset. ⊘ Reported as its own state,
    /// never as `Ok`: decoding past a truncated buffer would read a zero and print "served".
    Truncated,
}

impl RmStatus {
    fn describe(self) -> String {
        match self {
            RmStatus::Ok => "RM ok".to_string(),
            RmStatus::Refused(s) => format!("★ RM REFUSED status={s:#x}"),
            RmStatus::NoStatusField => "(no status)".to_string(),
            RmStatus::Truncated => "⊘ UNREADABLE".to_string(),
        }
    }
}

/// Decode one record's in-band status.
///
/// ★★★ **The offsets are the ABI crate's own, every one `ct_assert`ed there** — this function
/// selects between them by escape number and reads nothing it has not been told the shape of.
/// ⊘ An escape this table does not know answers [`RmStatus::NoStatusField`] rather than a
/// guess at offset 0, because offset 0 of every one of these structs is `hClient`/`hRoot` —
/// a handle, which is almost never zero and would print as a refusal on every single row.
fn rm_status_of(r: &kayfabe_linux_raw::census::IoctlRecord) -> RmStatus {
    if r.magic != b'F' {
        return RmStatus::NoStatusField;
    }
    // ⊘ Each arm cites the `ct_assert` that pins it.
    let at = match r.nr {
        0x29 => 12, // NV_ESC_RM_FREE            NVOS00, generated/nvos.rs:232
        0x2b => 28, // NV_ESC_RM_ALLOC           NVOS21, generated/nvos.rs:476
        0x2a => 28, // NV_ESC_RM_CONTROL         NVOS54, generated/nvos.rs:1318
        0x27 => 40, // NV_ESC_RM_ALLOC_MEMORY    NVOS02, bringup.rs (status @ +40)
        0x4e => 40, // NV_ESC_RM_MAP_MEMORY      NVOS33, submit.rs:3022
        0x58 => 40, // NV_ESC_RM_UNMAP_MEMORY_DMA NVOS47, generated/nvos.rs:1077
        0x57 => 56, // NV_ESC_RM_MAP_MEMORY_DMA  NVOS46, generated/nvos.rs:811
        _ => return RmStatus::NoStatusField,
    };
    let len = usize::from(r.reply_len);
    if at + 4 > len {
        return RmStatus::Truncated;
    }
    let s = u32::from_le_bytes([
        r.reply[at],
        r.reply[at + 1],
        r.reply[at + 2],
        r.reply[at + 3],
    ]);
    if s == 0 {
        RmStatus::Ok
    } else {
        RmStatus::Refused(s)
    }
}

/// ★★★★★ **THE MANDATORY KNOWN-POSITIVE for [`rm_status_of`]** — issue a call that MUST be
/// refused in-band, and confirm the reader sees it.
///
/// ⊘⊘ Without this, *"no ioctl was silently refused"* is exactly what a **blind** reader
/// prints. That is not hypothetical: one week, twice — `GET_PTE_INFO` answered
/// `NV_ERR_TEST_ONLY_CODE_NOT_ENABLED` for every address including a known-mapped one, and only
/// a calibration caught it.
///
/// The probe is an `NV_ESC_RM_CONTROL` carrying a **command that does not exist**, on a handle
/// that does: the frontend copies it in, RM refuses it, and `ioctl(2)` returns **0**. ⇒ it
/// produces precisely the shape under test — `errno ok`, non-zero RM status.
fn in_band_known_positive(rm: &mut HostRmBackend, subdevice: kayfabe_isolate::HostHandle) {
    use kayfabe_linux_raw::census;
    // A command in a valid class range that RM implements for nothing. Not 0: zero is
    // "no command" and is refused by the frontend before RM sees it, which is a different
    // layer and would calibrate the wrong thing.
    const NO_SUCH_CMD: u32 = 0x2080_0FFE;
    census::phase("R33 in-band known-positive");
    let before = census::snapshot();
    let mut buf = [0u8; 8];
    let got = rm.raw_control_for_probe(subdevice, NO_SUCH_CMD, &mut buf);
    let after = census::snapshot();
    let new: Vec<_> = after.log.iter().filter(|r| r.seq > before.total).collect();
    let seen = new.iter().map(|r| rm_status_of(r)).collect::<Vec<_>>();
    let refused_in_band = seen.iter().any(|s| matches!(s, RmStatus::Refused(_)));
    let syscall_ok = new.iter().all(|r| r.errno == 0);
    if refused_in_band && syscall_ok {
        println!(
            "★     R33 IN-BAND CAL     = KNOWN-POSITIVE FIRED — cmd {NO_SUCH_CMD:#x} returned \
             `errno == 0` from the syscall AND a non-zero RM status in the parameter struct \
             ({seen:?}). ⇒ the in-band reader below CAN see a refusal, so a run reporting zero \
             refusals is a measurement rather than a blind spot. (caller saw {got:?})"
        );
    } else {
        println!(
            "FAIL  R33 IN-BAND CAL     = the known-positive did NOT produce the shape under \
             test (refused_in_band={refused_in_band}, syscall_ok={syscall_ok}, {seen:?}). ⊘⊘ \
             EVERY `IN-BAND VERDICT: 0 refused` BELOW IS VACUOUS THIS RUN — an unproven reader \
             reporting no refusals is indistinguishable from a reader that cannot see them"
        );
    }
    census::phase("");
}

/// The NVIDIA frontend escape names, by `_IOC_NR`.
///
/// ⊘ Deliberately here and not in `kayfabe-linux-raw`: that crate holds no business logic
/// (`l1_os_shell.md` §4.7), and *"`nr` 42 means `NV_ESC_RM_CONTROL`"* is business logic about
/// one driver. An unknown number prints as itself rather than as a guess.
fn nv_esc_name(magic: u8, nr: u8) -> &'static str {
    if magic != b'F' {
        return "(not the NVIDIA frontend)";
    }
    // ⚠ TRANSCRIBED FROM THE DRIVER'S OWN HEADERS, not from memory — an earlier draft of
    // this table was wrong on eleven rows because the numbers *looked* plausible, and a
    // wrong name on a right count is worse than no name at all.
    //   `ogkm-580.159.04: kernel-open/common/inc/nv-ioctl-numbers.h:29-42`
    //     (NV_IOCTL_MAGIC = 'F', NV_IOCTL_BASE = 200)
    //   `ogkm-580.159.04: src/nvidia/arch/nvalloc/unix/include/nv_escape.h` (the 0x27..0x5F set)
    match nr {
        0x27 => "RM_ALLOC_MEMORY",
        0x28 => "RM_ALLOC_OBJECT",
        0x29 => "RM_FREE",
        0x2A => "RM_CONTROL",
        0x2B => "RM_ALLOC",
        0x32 => "RM_CONFIG_GET",
        0x33 => "RM_CONFIG_SET",
        0x34 => "RM_DUP_OBJECT",
        0x35 => "RM_SHARE",
        0x37 => "RM_CONFIG_GET_EX",
        0x38 => "RM_CONFIG_SET_EX",
        0x39 => "RM_I2C_ACCESS",
        0x41 => "RM_IDLE_CHANNELS",
        0x4A => "RM_VID_HEAP_CONTROL",
        0x4D => "RM_ACCESS_REGISTRY",
        0x4E => "RM_MAP_MEMORY",
        0x4F => "RM_UNMAP_MEMORY",
        0x52 => "RM_GET_EVENT_DATA",
        0x54 => "RM_ALLOC_CONTEXT_DMA2",
        0x56 => "RM_ADD_VBLANK_CALLBACK",
        0x57 => "RM_MAP_MEMORY_DMA",
        0x58 => "RM_UNMAP_MEMORY_DMA",
        0x59 => "RM_BIND_CONTEXT_DMA",
        0x5C => "RM_EXPORT_OBJECT_TO_FD",
        0x5D => "RM_IMPORT_OBJECT_FROM_FD",
        0x5E => "RM_UPDATE_DEVICE_MAPPING_INFO",
        0x5F => "RM_LOCKLESS_DIAGNOSTIC",
        200 => "CARD_INFO",
        201 => "REGISTER_FD",
        206 => "ALLOC_OS_EVENT",
        207 => "FREE_OS_EVENT",
        209 => "STATUS_CODE",
        210 => "CHECK_VERSION_STR",
        211 => "IOCTL_XFER_CMD",
        212 => "ATTACH_GPUS_TO_FD",
        213 => "QUERY_DEVICE_INTR",
        214 => "SYS_PARAMS",
        215 => "NUMA_INFO",
        216 => "SET_NUMA_STATUS",
        217 => "EXPORT_TO_DMABUF_FD",
        218 => "WAIT_OPEN_COMPLETE",
        _ => "(unnamed NV_ESC)",
    }
}

/// ★★★★★ R31 — **will host RM build a channel whose command queue is memory we did NOT
/// allocate, at the guest's own address and the guest's own entry count?**
///
/// The blocker, stated exactly: we allocate a host channel with **its own** queue, which
/// stays empty, so the engine consumes nothing forever while the guest pushes into **its**
/// queue, which our channel does not read. The fix is not a copier — it is to name the
/// guest's queue in the channel alloc. This rung is the one fact that stands between here
/// and that, asked with no guest in the picture.
///
/// # ★★ Two predictions, made from SOURCE before the run, so the result can refute them
///
/// - **Arm B** (`NV_ESC_RM_MAP_MEMORY` on the guest-backed ring) — expected **refused**.
///   Expected line: `status_check(out.status)` inside `RmConnection::map_cpu_windowed_on`,
///   i.e. the driver answering the escape.
/// - **Arm C** (the same channel alloc with `gpFifoOffset` at an address nothing was ever
///   mapped at) — ⚠ expected **ACCEPTED**, and that is a refutation of the brief this rung
///   was written from, not of the rung. The open driver forwards `gpFifoOffset` straight to
///   GSP without resolving it (`ogkm-580:
///   src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2664`), and RM *itself* allocates a
///   channel with `gpFifoOffset = 0` and says why: *"Set the gpFifoOffset to zero
///   intentionally since we only need this channel to be created, but will not submit any
///   work to it. So it's fine not to provide a valid offset here."* (`ogkm-580:
///   src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:2420-2424`). ⇒ If it is accepted, the
///   binding is needed when hardware **fetches**, not when the channel is **born** — and
///   the host channel's birth does not have to move to the doorbell.
///
/// ⊘ **What a green arm A does not establish.** Nothing here schedules the channel, rings
/// it, or writes `GP_PUT`; the engine has nothing to fetch and none of this runs the
/// guest's work. It establishes exactly that host RM accepts a channel over a queue it did
/// not allocate, at numbers its caller states.
fn guest_ring_channel_probe(rm: &mut HostRmBackend, gpu: u32) -> bool {
    println!(
        "info  R31 guest ring      = GPU {gpu}, euid {} — a sealed memfd → OS_DESCRIPTOR → \
         FIXED map → a channel whose gpFifoOffset and gpFifoEntries are the CALLER'S, with \
         no ring allocated and no CPU map of it",
        kayfabe_linux_raw::geteuid(),
    );
    let vas = match rm.alloc_vaspace() {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  R31 vaspace         = {e:?} (the rung needs its own address space)");
            return false;
        }
    };
    // ★ The in-process control, run FIRST and against the same entry point: a zero entry
    // count is the one number in the guest's declaration this port refuses, because it is
    // the modulus of the wrap arithmetic. Expected line: the `RING_ENTRIES_REFUSED` return
    // in `alloc_channel_in`, **before** any host object exists — which is why the CPU-map
    // counter is read across it and must not move.
    let before = rm.cpu_map_calls();
    let zero = rm.alloc_channel_over_guest_ring(
        vas,
        kayfabe_abi::submit::ENGINE_TYPE_COPY0,
        kayfabe_isolate_host::rm::GuestRing {
            memory: kayfabe_isolate::HostHandle::NULL,
            ring_va: 0,
            gp_fifo_va: 0,
            gp_fifo_entries: 0,
            userd: None,
        },
    );
    let arm_d = match zero {
        Err(RmError::Other(s)) if s == kayfabe_isolate_host::rm::RING_ENTRIES_REFUSED => {
            let moved = rm.cpu_map_calls() - before;
            if moved == 0 {
                println!(
                    "★     R31 arm D entries   = a zero `gpFifoEntries` was REFUSED BY NAME \
                     (`RING_ENTRIES_REFUSED`) and NOTHING was allocated on the way — the CPU-map \
                     counter did not move. The refusal is reachable, so the arms below are not \
                     vacuous"
                );
                true
            } else {
                println!(
                    "??    R31 arm D entries   = refused by name, but {moved} CPU mapping(s) were \
                     attempted first — the refusal is not where it claims to be"
                );
                false
            }
        }
        other => {
            println!(
                "FAIL  R31 arm D entries   = a zero `gpFifoEntries` was answered {other:?}, not \
                 `RING_ENTRIES_REFUSED`. A count of zero is the divisor of `submit_entry`'s wrap"
            );
            false
        }
    };

    let e = match rm.prove_guest_ring_channel(vas) {
        Ok(e) => e,
        Err(err) => {
            println!(
                "FAIL  R31 setup           = {err:?} (the memfd, the reservation, the \
                 OS_DESCRIPTOR or its FIXED map — none of which is the thing under test)"
            );
            let _ = rm.free(vas);
            return false;
        }
    };
    println!(
        "info  R31 what was asked  = ring object at {:#018x}, gpFifoOffset {:#018x} (= ring \
         + 0x3000, deliberately NOT our 0x1000), gpFifoEntries {} (the guest's measured \
         count; ours is 64)",
        e.ring_asked_va, e.gp_fifo_va, e.gp_fifo_entries
    );

    // Arm A, in the order that keeps each answer about its own subject.
    let arm_a = if !e.placed_as_asked() {
        println!(
            "FAIL  R31 place           = asked {:#018x}, RM chose {:#018x} — every number \
             below would be about a different address",
            e.ring_asked_va, e.ring_got_va
        );
        false
    } else {
        match &e.channel {
            Err(err) => {
                println!(
                    "FAIL  R31 adopt           = the channel alloc REFUSED {err:?} for a ring it \
                     did not allocate. ⇒ THE RUNG'S PREMISE IS REFUTED: host RM will not name a \
                     caller-supplied queue, and the shadow channel cannot be built this way"
                );
                false
            }
            Ok(token) => {
                let ok_numbers = e.adopted_the_guests_numbers();
                let ok_maps = e.mapped_only_userd();
                let ok_store = matches!(
                    e.ring_store,
                    Err(RmError::Other(s)) if s == kayfabe_isolate_host::rm::RING_NOT_OURS
                );
                if !ok_numbers {
                    println!(
                        "FAIL  R31 numbers         = the channel recorded {:?}, not \
                         ({:#018x}, {}) — something between the caller and RM substituted a \
                         constant",
                        e.declared, e.gp_fifo_va, e.gp_fifo_entries
                    );
                }
                if !ok_maps {
                    println!(
                        "FAIL  R31 no-cpu-map      = building the channel asked RM for {} CPU \
                         mappings, not 1. ⊘ Exactly one is correct — USERD, which is ours; a \
                         second one is a mapping of the GUEST'S ring",
                        e.cpu_maps.1 - e.cpu_maps.0
                    );
                }
                if !ok_store {
                    println!(
                        "FAIL  R31 ring store      = a store into the guest-backed ring answered \
                         {:?}, not `RING_NOT_OURS`. An `Ok` means a CPU view of the guest's ring \
                         exists after all",
                        e.ring_store
                    );
                }
                if ok_numbers && ok_maps && ok_store {
                    println!(
                        "★     R31 adopt           = HOST RM BUILT THE CHANNEL (token {token:#x}) \
                         over an object it did not allocate, placed AS ASKED at {:#018x}, told \
                         gpFifoOffset {:#018x} and gpFifoEntries {} — and building it asked RM \
                         for exactly ONE CPU mapping (USERD). A store into the ring is refused by \
                         name (`RING_NOT_OURS`)",
                        e.ring_got_va, e.gp_fifo_va, e.gp_fifo_entries
                    );
                }
                ok_numbers && ok_maps && ok_store
            }
        }
    };

    // Arm B — the mapping control. Reported either way; an `Ok` refutes G4's *"it
    // measurably fails"* without touching *"we do not need it"*.
    match &e.cpu_map_of_guest_ring {
        Err(err) => println!(
            "★     R31 arm B nomap     = the CPU map of the guest-backed ring was ATTEMPTED and \
             REFUSED {err:?} — so `no CPU map` is not a policy we chose, it is the only \
             available answer"
        ),
        Ok(()) => println!(
            "??    R31 arm B nomap     = the CPU map of the guest-backed ring SUCCEEDED (it was \
             dropped immediately). ⇒ `it measurably fails` is REFUTED; what still stands is that \
             we do not NEED it — the isolate already holds these pages through `GuestRamPlane`"
        ),
    }

    // Arm C — the binding control, and the prediction is that it does NOT fire.
    match &e.unbound {
        Err(err) => println!(
            "★     R31 arm C unbound   = the SAME call with gpFifoOffset {:#018x} — an address \
             nothing was ever mapped at — was REFUSED {err:?}. ⇒ RM validates the ring's binding \
             AT ALLOC, so a host channel cannot be born before its ring is bound",
            e.unbound_va
        ),
        Ok(token) => println!(
            "⚠⚠    R31 arm C unbound   = the SAME call with gpFifoOffset {:#018x} — an address \
             nothing was ever mapped at — was ACCEPTED (token {token:#x}, freed). ⇒ RM does NOT \
             resolve gpFifoOffset at alloc time, exactly as `ogkm-580: kernel_channel.c:2664` \
             and `kernel_graphics.c:2420-2424` say. TWO consequences: (1) arm A's acceptance is \
             about the ioctl and the NUMBERS, not about the binding; (2) the host channel's \
             birth does NOT have to move to the doorbell — the binding is needed when hardware \
             FETCHES, which is after it",
            e.unbound_va
        ),
    }

    let _ = rm.free(vas);
    let verdict = arm_a && arm_d;
    if verdict {
        println!(
            "★     R31 guest ring      = a host channel over the GUEST'S queue, at the GUEST'S \
             address and the GUEST'S entry count, on real hardware. ⊘ It is NOT runnable: \
             nothing writes `GP_PUT`, so the engine has nothing to fetch. cup2 does not pass \
             and the completion watcher stays NOT-OBSERVED"
        );
    }
    verdict
}

/// ★★★ R19 — **task `#128`: can an unprivileged process read the host GPU's own
/// nanosecond counter, and at WHICH page offset?**
///
/// `register_plane_read_native.md` rests entirely on the answer. `#128` says the guest's
/// timer reads become native passthrough onto the host GPU's register page, so the whole
/// design is unbuildable if RM refuses the mapping to a caller with no privilege — and
/// *"root can do it"* is not an answer, because the isolate is deliberately capability-less
/// (`guest_blast_radius.md` §3.1).
///
/// ⚠ **THIS RUNG MEANS NOTHING WHEN RUN AS ROOT** and says so in its own output.
/// `RmValidateMmapRequest` returns `NV_PROTECT_READ_WRITE` immediately for
/// `osIsAdministrator()` and never executes the range walk
/// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:2023-2054`). Both arms were run
/// 2026-08-02 on a GA106 at revision 9087090 and agreed. The measurement is the
/// run under an unprivileged uid; the root run is the **control** that shows the difference
/// is the privilege and not the code.
///
/// Five things are measured, and the fifth is the one that reshaped the task:
///
/// 1. `NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET` — the control that exists expressly *"so
///    that clients may map them directly"* (`ogkm-580: ctrl2080tmr.h:107-110`).
/// 2. **The PTIMER page**, via an `NV01_TIMER` object: a whole page of BAR0 `0x9000` with
///    no doorbell in it.
/// 3. **The usermode-window mirror**, which every isolate already maps in order to ring a
///    doorbell — no new object, no new mapping.
/// 4. Both advance, and **agree with each other**, which is what licenses treating them as
///    one counter at two addresses.
/// 5. ★★★ **Their PAGE OFFSETS**, printed side by side with the offset the *emulated*
///    device serves. A memslot maps a guest page onto a host page and **cannot re-base
///    within it**, so a host counter at page-offset `0x400` cannot answer a guest read at
///    page-offset `0x080` however mappable it is.
fn timer_probe(conn: &RmConnection) -> bool {
    use kayfabe_abi::submit::{
        NV01_TIMER_MAP_SIZE, PTIMER_BAR0_BASE, PTIMER_PAGE_SIZE, PTIMER_PAGE_TIME_0,
        USERMODE_NOTIFY_CHANNEL_PENDING, USERMODE_TIME_0,
    };
    const PAGE: u64 = 4096;
    // `geteuid` through the same raw layer everything else here uses. Not a permission
    // check — a LABEL on the measurement, so a root run cannot be quoted as the answer.
    // Both labels appear in the 2026-08-02 GA106 run at revision 9087090.
    let euid = kayfabe_linux_raw::geteuid();
    let privileged = euid == 0;
    println!(
        "info  R19 euid            = {euid}{}",
        if privileged {
            "  ⚠ ROOT — RmValidateMmapRequest takes the osIsAdministrator() fast path and \
             the range walk NEVER RUNS. This run is the CONTROL, not the measurement."
        } else {
            "  ← unprivileged: the mmap validation walk runs, which is the question"
        }
    );

    let mut ok = true;

    // (1) The documented route's first half.
    let mut payload = [0xCDu8; 4];
    match conn.timer_register_offset(&mut payload) {
        Ok(()) => {
            let off = u32::from_le_bytes(payload);
            println!(
                "★     R19 TIMER_GET_REGISTER_OFFSET = NV_OK, tmr_offset = {off:#x} \
                 (DRF_BASE(NV_PTIMER) is {PTIMER_BAR0_BASE:#x})"
            );
            if u64::from(off) != PTIMER_BAR0_BASE {
                println!(
                    "FAIL  R19 offset          = the control answered {off:#x}, not \
                     {PTIMER_BAR0_BASE:#x} — kayfabe_abi::submit::PTIMER_BAR0_BASE is wrong \
                     for this board and every offset below it is suspect"
                );
                ok = false;
            }
        }
        Err(e) => {
            println!(
                "info  R19 TIMER_GET_REGISTER_OFFSET = refused {e:?} — the documented \
                 client-mapping route is closed on this board"
            );
        }
    }

    // (2) The dedicated PTIMER page. ★★ TWO acts, reported separately, and a SWEEP of the
    // two plausible lengths — because the first run of this rung printed our own
    // page-alignment refusal (`NOT_IN_THIS_OBJECT`, 0x4B47) as though the driver had
    // refused the range. See `RmConnection::alloc_timer_object`.
    let ptimer_page = match conn.alloc_timer_object() {
        Err(e) => {
            println!(
                "info  R19 NV01_TIMER alloc = refused {e:?} — RM would not give this client a \
                 timer object at all, so the mapping question does not arise"
            );
            Err(e)
        }
        Ok(obj) => {
            println!("★     R19 NV01_TIMER alloc = hObject {obj:#010x}");
            // ★★★ The sweep is over (ioctl length, mmap length) PAIRS, not over one length.
            // The first two rows are the two ways of assuming they are the same number, and
            // each fails at a DIFFERENT layer; the third is the pair the driver's own code
            // describes. Running all three is what turns "it did not map" into an
            // attribution.
            let mut got = Err(RmError::Other(0));
            for (reg, mm, why) in [
                (
                    NV01_TIMER_MAP_SIZE,
                    NV01_TIMER_MAP_SIZE,
                    "the object's own size for both",
                ),
                (PTIMER_PAGE_SIZE, PTIMER_PAGE_SIZE, "a whole page for both"),
                (
                    NV01_TIMER_MAP_SIZE,
                    PTIMER_PAGE_SIZE,
                    "the object's size to RM, the rounded size to mmap",
                ),
            ] {
                match conn.map_object_uncached(obj, reg, mm) {
                    Ok((node, region)) => {
                        println!(
                            "★     R19 NV01_TIMER map  = ioctl {reg:#x} / mmap {mm:#x} \
                             ACCEPTED  ({why})"
                        );
                        got = Ok((obj, node, region));
                        break;
                    }
                    Err(e) => println!(
                        "info  R19 NV01_TIMER map  = ioctl {reg:#x} / mmap {mm:#x} refused \
                         {e:?}  ({why}){}",
                        match e {
                            RmError::Other(0x4B47) =>
                                "  ⚠ OUR OWN NOT_IN_THIS_OBJECT — the request never reached \
                                 RM; that mmap length is not page-aligned",
                            RmError::Other(0x2E) =>
                                "  ← NV_ERR_INVALID_LIMIT: the driver says the length is past \
                                 the resource's own size — a BOUND, not a privilege",
                            RmError::Other(s) if s & 0x8000_0000 != 0 =>
                                "  ← an errno: it DID reach the driver",
                            _ => "  ← an NV_STATUS: the driver decided",
                        }
                    ),
                }
            }
            got
        }
    };
    let page_readings = match &ptimer_page {
        Ok((_obj, _node, region)) => {
            let a = RmConnection::ptimer_page_read(region);
            std::thread::sleep(std::time::Duration::from_millis(20));
            let b = RmConnection::ptimer_page_read(region);
            match (a, b) {
                (Ok(a), Ok(b)) => {
                    println!(
                        "★     R19 PTIMER page     = {a} ns then {b} ns (+{} ns over a 20 ms \
                         sleep)",
                        b.saturating_sub(a)
                    );
                    // ⊘ The whole point. A counter that does not move is the failure this
                    // task exists to prevent, and it is INDISTINGUISHABLE from a working
                    // one on a single reading.
                    if b <= a {
                        println!(
                            "FAIL  R19 PTIMER page     = it did NOT ADVANCE across 20 ms — a \
                             mapping that reads a frozen value is worse than no mapping"
                        );
                        ok = false;
                    }
                    Some((a, b))
                }
                (a, b) => {
                    println!("FAIL  R19 PTIMER page     = read refused {a:?} / {b:?}");
                    ok = false;
                    None
                }
            }
        }
        Err(_) => {
            println!(
                "info  R19 PTIMER page     = no dedicated PTIMER-page mapping was obtained; \
                 read the two lines above for WHICH layer refused. A driver refusal is a \
                 FINDING; do not re-run this as root and quote the result."
            );
            None
        }
    };

    // (3) The mirror inside the window the isolate already holds.
    let mirror = conn.host_ptimer_via_usermode();
    match &mirror {
        Ok(a) => {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let b = conn.host_ptimer_via_usermode();
            match b {
                Ok(b) => {
                    println!(
                        "★     R19 usermode mirror = {a} ns then {b} ns (+{} ns over a 20 ms \
                         sleep) — read through the SAME window the doorbell is rung through, \
                         so it costs no new mapping",
                        b.saturating_sub(*a)
                    );
                    if b <= *a {
                        println!("FAIL  R19 usermode mirror = it did NOT ADVANCE across 20 ms");
                        ok = false;
                    }
                }
                Err(e) => {
                    println!("FAIL  R19 usermode mirror = second read refused {e:?}");
                    ok = false;
                }
            }
        }
        Err(e) => println!("info  R19 usermode mirror = refused {e:?}"),
    }

    // (4) ★★★ Are they the same counter? The two mappings are two different BAR0
    // addresses; nothing so far says they read one register block. The reads happen in a
    // known real-time order — PTIMER page `a`, sleep, PTIMER page `b`, then the mirror —
    // so a shared counter must satisfy `a < b <= mirror`, and `mirror - b` must be the
    // handful of microseconds those two reads are apart.
    //
    // ⊘ The first version of this check allowed a **one second** slack and passed on it.
    // That would have been satisfied by two unrelated counters that merely happened to be
    // near each other, which is not the claim. The bound is now 1 ms: wide enough that a
    // descheduled thread cannot manufacture a failure, far too narrow for two clocks that
    // are not the same clock.
    const AGREEMENT_NS: u64 = 1_000_000;
    if let (Some((a, b)), Ok(m)) = (page_readings, &mirror) {
        let after = *m >= b;
        let close = m.saturating_sub(b) <= AGREEMENT_NS;
        println!(
            "{}  R19 same counter?   = PTIMER page {a} then {b}, mirror {m} read next — \
             mirror is {} ns after the second page reading (bound {AGREEMENT_NS} ns)",
            if after && close { "★    " } else { "FAIL " },
            m.saturating_sub(b),
        );
        if !(after && close) {
            println!(
                "FAIL  R19 same counter?   = the two mappings are NOT reading one counter, so \
                 neither may stand in for the other"
            );
            ok = false;
        }
    }

    // (5) ★★★ The geography, which is what actually decides the design. Only the HOST
    // halves are printed here — what offset the *emulated* device serves is a fact about
    // the chip profile, not about this GPU, and it is asserted where it lives
    // (`kayfabe_device::ga10x`'s `the_guest_timer_offset_can_only_be_backed_by_the_host_
    // usermode_page`). Printing it here would make a hardware run look like the source of
    // a claim that never touched hardware.
    println!(
        "★     R19 host geography  = the PTIMER page carries the counter at page + {:#05x}; \
         the usermode window carries it at page + {:#05x}, with the doorbell {:#x} bytes \
         further on at page + {:#05x}",
        PTIMER_PAGE_TIME_0 & (PAGE - 1),
        USERMODE_TIME_0,
        USERMODE_NOTIFY_CHANNEL_PENDING - USERMODE_TIME_0,
        USERMODE_NOTIFY_CHANNEL_PENDING,
    );
    drop(ptimer_page);
    ok
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ w379 — THE RAW CLIENT AS THE MAPPING PLANE'S TEST ARTIFACT
//
// Owner, 2026-09-06: *"raw clients can remain useful, to test the allocate propagations
// and races about missing pages resulting in fault, mixing rpc with normal allocs, mean
// stress test, to have a test artifact on purely open source components easier to debug."*
//
// Every rung below prints a machine-readable `RUNG_<name>=PASS|FAIL|NOTRUN` line and a
// `RUNGCTL_<name>=PASS|FAIL` line for its positive control. ⊘ **`NOTRUN` is not a failure
// value** — it is what a rung prints when its own control did not pass, and a grader that
// folds it into `FAIL` reports a finding it never measured. That distinction is the whole
// reason the w377 harness could tell *"the binary was named wrong"* from *"the system is
// broken"*, and it is kept here deliberately.
//
// ## ⊘⊘⊘ SCOPE, MEASURED BEFORE ANY OF THESE WERE RUN — THE GUEST ARM CANNOT SERVE THEM
//
// These four rungs drive a real engine through a `SEM_RELEASE` at a caller-chosen GPU VA.
// **That probe is structurally unservable inside a Mode-2 guest**, for two independent
// reasons, both read out of this tree rather than inferred from a failure:
//
// 1. **The CPU copy-engine emulator decodes `SemRelease` and deliberately does not act on
//    it** — *"a `SemRelease` is deliberately NOT acted on here: it is the host semaphore …
//    and advancing it would satisfy our own counters while the guest spins on the word above
//    it"* (`kayfabe-rt/src/ceutils.rs:677-679`; the same decision is restated at `:1080`).
//    The only method it acts on is `LAUNCH_DMA`.
// 2. **`GP_GET` has no writer anywhere in the workspace.** Five occurrences of
//    `USERD_GP_GET`, every one a read; the sole USERD store in the tree is `USERD_GP_PUT`.
//    There is no PBDMA on an emulated device, so any rung that grades on that cursor grades
//    on a constant.
//
// ⇒ **These rungs are HOST-HARDWARE rungs.** Run natively they are real; run in the guest
// they measure the emulator's deliberate scope and must not be reported as a red. ⚠ The
// guest-side version of the same questions needs a `LAUNCH_DMA` probe, which is a different
// primitive and is NOT built here — see the report, where it is named as the follow-up
// rather than left implied.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// Bytes of every object the w379 rungs alias, map and fault on: one `FB_LEAF_GRANULE`
/// (`kayfabe-rt/src/device.rs`, `0x1_0000`).
///
/// ⊘ **Not 4 KiB, and the size is load-bearing.** RM maps device-local memory with 64 KiB
/// big pages regardless of what is asked, so a 4 KiB object cannot be placed or probed
/// independently of its neighbours — [`HostRmBackend::probe_va`]'s own docs record the
/// measurement. A rung that asked at 4 KiB would be reading its own geometry back.
const W379_BYTES: u64 = 0x1_0000;

/// The value every w379 object holds before any engine runs. Neither `0` nor `1`: a zero is
/// indistinguishable from freshly-allocated memory, and *"not the magic"* has to mean
/// *"nothing wrote here"* rather than *"we cannot tell"*.
const W379_SENTINEL: u32 = 0xDEAD_0379;

/// How long a release is given to land before it is called lost.
const W379_LAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Byte offsets inside a w379 object at which the three releases land. Distinct, so a
/// release through one VA cannot be mistaken for a release through another — which is the
/// entire content of the aliasing claim.
const W379_OFF_A: u64 = 0x0000;
/// See [`W379_OFF_A`].
const W379_OFF_B: u64 = 0x0040;
/// See [`W379_OFF_A`]. This is the offset the **re-release through VA_A after VA_B was
/// mapped** writes to — the one that goes silent if mapping the second VA revoked the first.
const W379_OFF_A2: u64 = 0x0080;

/// What one `SEM_RELEASE` through one GPU VA did.
///
/// ⊘ Three values and not a `bool`, because [`W379Release::Refused`] is *"the experiment
/// never ran"* rather than *"the experiment ran and failed"*, and folding it into `Lost`
/// would report our own harness as the system's red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum W379Release {
    /// ★ The payload arrived at the object offset it was aimed at.
    Landed,
    /// The payload never arrived. `saw` is what the word actually held — [`W379_SENTINEL`]
    /// means nothing wrote there, anything else means something else did.
    Lost {
        /// The word the object actually held when the deadline expired.
        saw: u32,
    },
    /// ⊘ The submission itself was refused, so the engine was never asked.
    Refused,
}

impl W379Release {
    /// Only [`W379Release::Landed`] is a pass.
    fn landed(self) -> bool {
        matches!(self, W379Release::Landed)
    }
}

/// ★★★★★ **w381 — WHICH PUSHBUFFER VERB EVERY RUNG USES TO PROVE A VA IS LIVE.**
///
/// The w379 battery had exactly one liveness primitive, and it is **unservable inside a
/// Mode-2 guest by design**: `submit_release_at` emits the host-FIFO semaphore run, the CPU
/// copy-engine emulator decodes it as `PushMethod::SemRelease` and *deliberately does not
/// act on it* (`kayfabe-rt/src/ceutils.rs`, the `else` arm of the `CeLaunchDma` `let`;
/// restated in `release_targets_of`). ⇒ **the guest arm could never reach its own positive
/// control**, so the whole battery was a native-only control and not an iteration handle.
///
/// ⊘ **The two arms are NOT interchangeable and must never be silently substituted.** They
/// exercise different engine machinery, and a run that does not say which one it used
/// cannot be compared to any other run — which is why [`W381Probe::as_str`] is printed on
/// every invocation before a single rung starts.
///
/// # ★ WHAT MAKES THE SUBSTITUTION HONEST
///
/// Both arms answer the same question — *does an address written through this GPU VA
/// receive our magic?* — read back through an **independent** CPU mapping of the object
/// ([`HostRmBackend::read_words_independently`]). The bytes are either there or they are
/// not. What changes is only which engine verb carries them:
///
/// | arm | pushbuffer methods | served by the Mode-2 emulator? |
/// |---|---|---|
/// | [`W381Probe::SemRelease`] | `SEM_ADDR_LO/HI/PAYLOAD/EXECUTE` (host FIFO) | **no**, by design |
/// | [`W381Probe::LaunchDma`] | `SET_OBJECT`, `OFFSET_IN/OUT`, `LINE_*`, `SET_SEMAPHORE_A/B/PAYLOAD`, `LAUNCH_DMA` | **yes** — `execute_ours_spans` moves the bytes |
///
/// ⚠ The `LaunchDma` arm writes **four bytes copied from the channel's own ring**, not a
/// literal the engine synthesises. That is strictly more evidence than a release: a copy
/// that lands proves the destination VA resolved *and* that a source VA resolved *and*
/// that the engine ran, where a release proves only the first and the third.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum W381Probe {
    /// The w379 primitive. **Native only** — see the type's own docs.
    SemRelease,
    /// ★★★★★ The w381 primitive: a four-byte `LAUNCH_DMA` into the address under test.
    /// Servable on **both** sides of the differential.
    LaunchDma,
}

impl W381Probe {
    /// The printed form, so a log line, a grader and a human agree on the vocabulary.
    fn as_str(self) -> &'static str {
        match self {
            W381Probe::SemRelease => "sem-release",
            W381Probe::LaunchDma => "launch-dma",
        }
    }
}

/// ★★ **A scheduled channel and the work-submit token its doorbell needs**, as ONE value.
///
/// ⊘ Not a tidying. The two are minted together by
/// [`HostRmBackend::alloc_channel_at`] and are meaningless apart: a token rung on the wrong
/// channel's doorbell is a submission into somebody else's ring, and the rungs most exposed
/// to that are exactly the ones that hold **two** channels live at once (R3's victim and
/// bystander, R5b's two clients). Carrying them together makes the mispairing
/// unexpressible-by-accident.
///
/// ⚠ It is also what keeps [`w379_release_through`] inside clippy's seven-argument bound
/// after w381 added the probe selector — but that is the *occasion* for the type, not its
/// reason. A bare `#[allow]` would have bought the same silence and none of the safety.
#[derive(Debug, Clone, Copy)]
struct W381Chan {
    /// The channel object itself.
    h: kayfabe_isolate::HostHandle,
    /// The work-submit token `alloc_channel_at` returned **with** `h`, and only with it.
    token: u64,
}

/// The payload the w381 copy probe releases on the channel's **own** semaphore, purely as a
/// retirement **qualifier**.
///
/// ⊘⊘ **IT IS NOT THE VERDICT AND MUST NEVER BECOME ONE.** The primary observable is the
/// destination word; this one only ever qualifies a red. `GP_GET` has no writer anywhere in
/// this workspace and an emulated device has no PBDMA, so a rung that graded on a cursor
/// produced a confident, plausible, always-wrong `NeverFetched` — and a channel semaphore
/// is the same class of secondary evidence.
const W381_RETIRE_PAYLOAD: u32 = 0x8138_1381;

/// What the **host driver** — not our bookkeeping — says about one VA in one address space.
///
/// ⊘ The point of this type is that [`W379HostVa::Unmeasured`] is a first-class value.
/// `NV0080_CTRL_CMD_DMA_GET_PTE_INFO` answers `NV_ERR_TEST_ONLY_CODE_NOT_ENABLED` on every
/// release driver, and a rung that read that refusal as *"the VA does not resolve"* would
/// manufacture the exact finding it is looking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum W379HostVa {
    /// A page table covers the VA. ⚠ **PDE granularity**: this says a table exists, never
    /// that the leaf PTE is valid. See [`HostRmBackend::pde_info`].
    PdeCovers,
    /// The control answered, and said no page table covers the VA.
    PdeAbsent,
    /// ⊘ The control refused. **Not an answer.**
    Unmeasured,
}

impl W379HostVa {
    /// The printed form, so a log line and a grader agree on the vocabulary.
    fn as_str(self) -> &'static str {
        match self {
            W379HostVa::PdeCovers => "PDE_COVERS",
            W379HostVa::PdeAbsent => "PDE_ABSENT",
            W379HostVa::Unmeasured => "UNMEASURED",
        }
    }
}

/// Ask the host driver whether `va` resolves in `vas`, through `NV0080_CTRL_CMD_DMA_GET_
/// PDE_INFO`.
///
/// ★ **The oracle deliberately does not share our allocator or our tables.** It is RM's own
/// answer about RM's own page tables, which is what makes it usable to check a publication
/// claim that our own census also makes — an instrument that shares the thing it measures
/// is not an observer.
fn w379_host_va(rm: &mut HostRmBackend, vas: kayfabe_isolate::HostHandle, va: u64) -> W379HostVa {
    match rm.pde_info(vas, va) {
        Ok((Some(_), _)) => W379HostVa::PdeCovers,
        Ok((None, _)) => W379HostVa::PdeAbsent,
        Err(_) => W379HostVa::Unmeasured,
    }
}

/// Submit one `SEM_RELEASE(target_va + byte_off, payload)` on `chan` and wait for it to
/// land at `byte_off` inside `mem`.
///
/// ⊘ **No fence and no `SEM_ACQUIRE`.** [`HostRmBackend::submit_release_at`] is used rather
/// than `submit_fenced_release` because an acquire is a second thing that can fail in a way
/// that looks exactly like the first — and the question every w379 rung asks is about **one**
/// address resolving.
///
/// ⊘ The read-back is [`HostRmBackend::read_words_independently`]: a fresh device node, a
/// fresh mapping, a kernel-chosen address. Reading through the mapping the sentinel was
/// written through would prove the page is writable and nothing else.
fn w379_release_through(
    rm: &mut HostRmBackend,
    probe: W381Probe,
    ch: W381Chan,
    mem: kayfabe_isolate::HostHandle,
    target_va: u64,
    byte_off: u64,
    payload: u32,
) -> W379Release {
    // ── SUBMIT, by whichever verb this run selected. ─────────────────────────────────────
    //
    // ★★★ w381 — the two arms differ HERE and nowhere else. Everything below this block is
    // byte-identical for both, deliberately: the read-back, the deadline and the three
    // verdicts are the rung's, not the primitive's, so a `LaunchDma` result and a
    // `SemRelease` result are the same measurement of the same address and can be put side
    // by side. ⊘ If the arms diverged in how they GRADE, the differential this exists to
    // produce would be comparing two different questions.
    let submitted = match probe {
        W381Probe::SemRelease => rm
            .submit_release_at(ch.h, ch.token, target_va + byte_off, payload)
            .is_ok(),
        W381Probe::LaunchDma => {
            let (src_off, _sem_off) = kayfabe_isolate_host::rm::HostRmBackend::copy_probe_offsets();
            // ⊘ The magic goes into the channel's OWN ring first — a CPU store into memory
            // that is mapped before the doorbell by construction. A failure to place the
            // source is a refusal, never a `Lost`: the engine was never asked.
            rm.ring_store_u32(ch.h, src_off, payload).is_ok()
                && rm
                    .submit_copy_at(
                        ch.h,
                        ch.token,
                        src_off,
                        target_va + byte_off,
                        4,
                        W381_RETIRE_PAYLOAD,
                    )
                    .is_ok()
        }
    };
    if !submitted {
        return W379Release::Refused;
    }
    // ── OBSERVE. The PRIMARY observable, and it is the only one that decides. ────────────
    let deadline = std::time::Instant::now() + W379_LAND_TIMEOUT;
    let mut saw;
    loop {
        match rm.read_words_independently(mem, W379_BYTES, &[byte_off]) {
            Ok(w) => saw = w[0],
            Err(_) => return W379Release::Refused,
        }
        if saw == payload {
            return W379Release::Landed;
        }
        if std::time::Instant::now() >= deadline {
            return W379Release::Lost { saw };
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// ★★ The **qualifier** for a w381 copy probe: did the channel's own semaphore retire?
///
/// ⊘⊘ **ORDER THE OBSERVABLES.** This is read only when the primary one is already a red,
/// and it may only ever say *which kind* of red — never turn one into a green or a green
/// into one. `GP_GET` produced a confident, plausible, always-wrong `NeverFetched` because
/// a rung graded on a cursor that no writer in this workspace ever advances; a channel
/// semaphore is the same class of evidence and gets the same treatment.
///
/// Returns the printed form directly, because a caller that got a `u32` back would have to
/// re-derive the comparison and could get it wrong in a second place.
fn w381_retired(rm: &HostRmBackend, chan: kayfabe_isolate::HostHandle) -> &'static str {
    let (_src_off, sem_off) = kayfabe_isolate_host::rm::HostRmBackend::copy_probe_offsets();
    match rm.ring_load_u32(chan, sem_off) {
        Ok(v) if v == W381_RETIRE_PAYLOAD => "RETIRED",
        Ok(0) => "NOT-RETIRED(sem still 0)",
        Ok(_) => "NOT-RETIRED(sem holds something else)",
        Err(_) => "UNMEASURED(ring read refused)",
    }
}

/// ★★★★★ **w379 R1′ — CAN ONE ALLOCATION BE LIVE AT TWO GPU VAs AT THE SAME TIME?**
///
/// The w377 correction (2026-09-06) named the LLM wall as **FB-join aliasing**: the join
/// store is keyed by physical frame alone (`install_join(phys, region)` /
/// `release_join(phys)`), so one framebuffer frame can be host-backed at exactly **one** GPU
/// VA. The guest holds 17 frames aliased at two VAs each, so publishing either VA revokes
/// the other, and the `Xid 31` lands at a VA we un-published ourselves.
///
/// This rung is that hazard in fifteen lines of raw client, with no libcuda anywhere:
///
/// ```text
///   1  allocate ONE device-local object
///   2  map it at VA_A                          -> release through VA_A must land   [CONTROL]
///   3  map the SAME object at VA_B             -> release through VA_B must land
///   4  release through VA_A AGAIN              -> must STILL land                  [THE ASK]
/// ```
///
/// ★★ **Step 4 is the whole rung.** Steps 2 and 3 pass even in a world where the second
/// mapping revokes the first, because each is exercised immediately after it is made. Only
/// coming back to VA_A *after* VA_B exists can see a revoke.
///
/// ★ **And the aliasing itself is proved, not assumed.** The three releases land at three
/// distinct offsets of the **one object** the caller allocated, read back through an
/// independent mapping. A release through VA_B that lands at `mem + 0x40` cannot have
/// reached anything but the object we mapped once — so `Landed` at both offsets is a
/// measurement that the two VAs name the same memory, not an inference from having asked
/// for it.
///
/// ## ★★★ PRE-REGISTERED, BEFORE THE RUN
///
/// - **all three land** ⇒ two VAs over one frame both stay live. On **bare metal** this is
///   the expected answer and is the control for the guest arm. In the **guest** it would
///   mean the aliasing diagnosis is wrong — and that must be said loudly rather than filed
///   as a green.
/// - **A lands, B lands, A2 lost** ⇒ ★★★★★ **THE REPRO.** Mapping the second VA revoked the
///   first. This is the LLM's wall, deterministic, ours, no proprietary runtime in it.
/// - **A lands, B lost** ⇒ the second mapping never took effect at all. A different defect
///   from the revoke, and it must not be reported as one.
/// - **A lost** ⇒ ⊘ **the control failed and the rung is UNINTERPRETABLE.** Prints `NOTRUN`.
/// - **a map is refused / placed elsewhere** ⇒ ⊘ `NOTRUN`. RM declining a fixed placement is
///   a statement about our address choice, not about aliasing.
fn alias_two_vas(rm: &mut HostRmBackend, probe: W381Probe, gpu: u32) -> bool {
    // Ring, VA_A and VA_B are ≥ 4 GiB apart. A VA_B adjacent to VA_A could be covered by a
    // big PTE VA_A's mapping already installed, which would make step 4 pass for a reason
    // that has nothing to do with aliasing.
    const RING_AT: u64 = 0x0000_0006_1100_0000;
    const VA_A: u64 = 0x0000_0007_1100_0000;
    const VA_B: u64 = 0x0000_0008_1100_0000;
    const MAGIC_A: u32 = 0xA11A_5A01;
    const MAGIC_B: u32 = 0xA11A_5B01;
    const MAGIC_A2: u32 = 0xA11A_5A02;

    println!(
        "info  R1' alias two VAs   = GPU {gpu}, euid {} — ONE object, TWO GPU VAs, and a \
         release through the FIRST one AFTER the second is mapped",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R1' the bar         = A must land (control), then B, then A AGAIN. ⊘ Steps \
         A and B pass even where the second map revokes the first; only the RE-release \
         through VA_A can see a revoke"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  R1' engine          = COPY0 is not expressible");
        println!("RUNGCTL_alias_two_vas=FAIL");
        println!("RUNG_alias_two_vas=NOTRUN");
        return false;
    };

    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R1' vaspace         = the rung needs its own address space");
        println!("RUNGCTL_alias_two_vas=FAIL");
        println!("RUNG_alias_two_vas=NOTRUN");
        return false;
    };

    let mut mapped: Vec<u64> = Vec::new();
    let mut chan_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut mem_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut control_ok = false;

    let mut go = || -> bool {
        let Ok((chan, token)) = rm.alloc_channel_at(vas, engine_type, Some(GpuVa(RING_AT))) else {
            println!("??    R1' channel         = refused at {RING_AT:#018x} — NOT a result");
            return false;
        };
        chan_h = Some(chan);
        if rm.channel_ring_va(chan) != Some(RING_AT) {
            println!("??    R1' ring placement   = RM did not place the ring where asked");
            return false;
        }
        if rm.schedule(chan).is_err() {
            println!("??    R1' schedule         = refused");
            return false;
        }

        let Ok(mem) = rm.alloc_probe_local(W379_BYTES) else {
            println!("??    R1' object          = device-local allocation refused");
            return false;
        };
        mem_h = Some(mem);
        if rm.fill_words(mem, W379_BYTES, W379_SENTINEL, 0).is_err() {
            println!("??    R1' sentinel        = could not be written");
            return false;
        }

        // ── step 2 — VA_A ────────────────────────────────────────────────────────────────
        match rm.map_local_at(vas, mem, W379_BYTES, Some(VA_A)) {
            Ok(got) if got == VA_A => mapped.push(got),
            Ok(got) => {
                println!("??    R1' VA_A placement   = asked {VA_A:#018x}, RM chose {got:#018x}");
                mapped.push(got);
                return false;
            }
            Err(e) => {
                println!("??    R1' VA_A map        = refused {e:?}");
                return false;
            }
        }
        let host_a1 = w379_host_va(rm, vas, VA_A);
        println!(
            "info  R1' host says A     = {} at {VA_A:#018x} (after mapping A)",
            host_a1.as_str()
        );

        let a = w379_release_through(
            rm,
            probe,
            W381Chan { h: chan, token },
            mem,
            VA_A,
            W379_OFF_A,
            MAGIC_A,
        );
        println!("info  R1' release via A   = {a:?}");
        if !a.landed() {
            println!(
                "??    R1' CONTROL FAILED  = a release through the ONLY mapping did not \
                 land. ⊘ Everything below is UNINTERPRETABLE — this is the channel, the \
                 ring or the release, not aliasing"
            );
            return false;
        }
        control_ok = true;
        println!("ok    R1' control         = MAGIC_A landed through the single mapping");

        // ── step 3 — VA_B over the SAME object ───────────────────────────────────────────
        match rm.map_local_at(vas, mem, W379_BYTES, Some(VA_B)) {
            Ok(got) if got == VA_B => mapped.push(got),
            Ok(got) => {
                println!("??    R1' VA_B placement   = asked {VA_B:#018x}, RM chose {got:#018x}");
                mapped.push(got);
                return false;
            }
            Err(e) => {
                println!(
                    "FAIL  R1' VA_B map        = refused {e:?}. ⊘ RM DECLINED to alias one \
                     allocation at a second VA — if this is bare metal, the aliasing the \
                     guest is claimed to do is not legal and the diagnosis needs revisiting"
                );
                return false;
            }
        }
        let host_a2 = w379_host_va(rm, vas, VA_A);
        let host_b2 = w379_host_va(rm, vas, VA_B);
        println!(
            "info  R1' host says A/B   = A {} / B {} (after mapping BOTH)",
            host_a2.as_str(),
            host_b2.as_str()
        );

        let b = w379_release_through(
            rm,
            probe,
            W381Chan { h: chan, token },
            mem,
            VA_B,
            W379_OFF_B,
            MAGIC_B,
        );
        println!("info  R1' release via B   = {b:?}");
        if !b.landed() {
            println!(
                "FAIL  R1' SECOND VA DEAD  = the second mapping never became live. ⊘ This \
                 is NOT the revoke: VA_A was never re-tested. A different defect"
            );
            return false;
        }
        println!(
            "★     R1' ALIAS PROVED    = MAGIC_B landed at offset {W379_OFF_B:#x} of the \
             ONE object this rung allocated — VA_A and VA_B name the same memory, measured \
             rather than asked for"
        );

        // ── step 4 — VA_A AGAIN, now that VA_B exists ────────────────────────────────────
        let a2 = w379_release_through(
            rm,
            probe,
            W381Chan { h: chan, token },
            mem,
            VA_A,
            W379_OFF_A2,
            MAGIC_A2,
        );
        println!("info  R1' release via A#2 = {a2:?}");
        match a2 {
            W379Release::Landed => {
                println!(
                    "★     R1' BOTH VAs LIVE   = a release through VA_A landed AFTER VA_B \
                     was mapped over the same frame. ⇒ One allocation IS live at two GPU \
                     VAs at once. On bare metal this is the control. IN THE GUEST this \
                     REFUTES the FB-join aliasing diagnosis and must be said loudly"
                );
                true
            }
            W379Release::Lost { saw } => {
                println!(
                    "FAIL  R1' FIRST VA REVOKED = VA_A landed before VA_B was mapped and is \
                     SILENT after ({saw:#010x}, want {MAGIC_A2:#010x}). ⇒ ★★★★★ THE REPRO — \
                     mapping the second VA revoked the first. Look for `Xid 31 … FAULT_PDE` \
                     at {VA_A:#018x} + {W379_OFF_A2:#x}"
                );
                false
            }
            W379Release::Refused => {
                println!("??    R1' release via A#2  = the submission was refused, not lost");
                false
            }
        }
    };

    let ok = go();

    for va in mapped {
        let _ = rm.unmap_local(vas, va);
    }
    if let Some(h) = mem_h {
        let _ = rm.free(h);
    }
    if let Some(h) = chan_h {
        let _ = rm.free(h);
    }
    let _ = rm.free(vas);

    println!(
        "RUNGCTL_alias_two_vas={}",
        if control_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "RUNG_alias_two_vas={}",
        if !control_ok {
            "NOTRUN"
        } else if ok {
            "PASS"
        } else {
            "FAIL"
        }
    );
    ok
}

/// ★★★★★ **w379 R1″ — THE DISCRIMINATOR. Does unmapping ONE of two aliased VAs disturb the
/// other, and does the unmap become observable at all?**
///
/// The w377 correction leaves two models alive, and they select **different fixes**:
///
/// - **(a)** the guest genuinely holds both VAs live ⇒ fix = allow N VAs per frame;
/// - **(b)** one VA is stale in our decode and we never learned to drop it ⇒ fix = observe
///   the unmap.
///
/// Both fit every number in the boot, because the compute path shows no TLB invalidates.
/// ⚠ And the line our own log prints — *"The old VA is still DESCRIBED by the guest"* — is
/// an **unconditional string literal**, not a check, so it cannot discriminate either.
///
/// This rung emits the sequence that does, and states the ground truth for it:
///
/// ```text
///   map VA_A, map VA_B          -> both live (asserted, as in R1')
///   UNMAP VA_A                  -> [ALIAS_MARK=unmap_a] on its own line
///   ask the host driver about VA_A and VA_B
///   release through VA_B        -> MUST still land   <- the property under test
/// ```
///
/// ★★★ **The bar is `VA_B` surviving.** An unmap of one alias that takes the other down with
/// it is the same bug as the publish-side revoke, seen from the other end — and it is the
/// one an `(a)`-shaped fix (N VAs per frame) has to get right.
///
/// ## ⊘ WHAT THIS RUNG CANNOT DO, STATED SO NOBODY READS IT AS DOING IT
///
/// It cannot tell you whether **our device** noticed the unmap: that is a fact about the
/// Mode-2 shim's decode, and this binary is a client of the driver, not of the shim. What it
/// does is emit `ALIAS_MARK=` lines at unambiguous instants so a guest-side log can be
/// **joined** to them, and establish natively that the unmap really happened — which is the
/// precondition for reading any guest-side silence as *"we missed it"* rather than
/// *"nothing to miss"*.
///
/// ## ⚠ THE ORACLE'S OWN LIMIT, PRE-REGISTERED
///
/// `pde_info` answers at **PDE** granularity. RM is free to leave a page *table* standing
/// after the last leaf in it is unmapped, so `PDE_COVERS` at VA_A **after** the unmap is
/// **not** evidence the mapping survived, and this rung does not grade on it. The graded
/// facts are `probe_va` (RM's allocator saying the VA is free again) and the VA_B release.
fn alias_unmap_observe(rm: &mut HostRmBackend, probe: W381Probe, gpu: u32) -> bool {
    const RING_AT: u64 = 0x0000_0006_3100_0000;
    const VA_A: u64 = 0x0000_0007_3100_0000;
    const VA_B: u64 = 0x0000_0008_3100_0000;
    const MAGIC_A: u32 = 0xA11A_5A11;
    const MAGIC_B: u32 = 0xA11A_5B11;
    const MAGIC_B2: u32 = 0xA11A_5B12;

    println!(
        "info  R1\" unmap observe   = GPU {gpu}, euid {} — alias one object at two VAs, \
         UNMAP the first, and ask whether the second survived",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R1\" the bar         = both VAs live (control), then VA_A unmapped, then a \
         release through VA_B must STILL land. ⊘ `pde_info` after an unmap is NOT graded — \
         RM may leave the page table standing"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  R1\" engine          = COPY0 is not expressible");
        println!("RUNGCTL_alias_unmap=FAIL");
        println!("RUNG_alias_unmap=NOTRUN");
        return false;
    };
    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R1\" vaspace         = the rung needs its own address space");
        println!("RUNGCTL_alias_unmap=FAIL");
        println!("RUNG_alias_unmap=NOTRUN");
        return false;
    };

    let mut live: Vec<u64> = Vec::new();
    let mut chan_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut mem_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut control_ok = false;

    let mut go = || -> bool {
        let Ok((chan, token)) = rm.alloc_channel_at(vas, engine_type, Some(GpuVa(RING_AT))) else {
            println!("??    R1\" channel         = refused at {RING_AT:#018x}");
            return false;
        };
        chan_h = Some(chan);
        if rm.schedule(chan).is_err() {
            println!("??    R1\" schedule         = refused");
            return false;
        }
        let Ok(mem) = rm.alloc_probe_local(W379_BYTES) else {
            println!("??    R1\" object          = device-local allocation refused");
            return false;
        };
        mem_h = Some(mem);
        if rm.fill_words(mem, W379_BYTES, W379_SENTINEL, 0).is_err() {
            println!("??    R1\" sentinel        = could not be written");
            return false;
        }
        for (name, at) in [("A", VA_A), ("B", VA_B)] {
            match rm.map_local_at(vas, mem, W379_BYTES, Some(at)) {
                Ok(got) if got == at => live.push(got),
                Ok(got) => {
                    println!("??    R1\" VA_{name} placement = asked {at:#018x}, got {got:#018x}");
                    live.push(got);
                    return false;
                }
                Err(e) => {
                    println!("??    R1\" VA_{name} map      = refused {e:?}");
                    return false;
                }
            }
        }
        println!("ALIAS_MARK=both_mapped va_a={VA_A:#018x} va_b={VA_B:#018x}");

        // ── the control: BOTH aliases live before anything is torn down ─────────────────
        let a = w379_release_through(
            rm,
            probe,
            W381Chan { h: chan, token },
            mem,
            VA_A,
            W379_OFF_A,
            MAGIC_A,
        );
        let b = w379_release_through(
            rm,
            probe,
            W381Chan { h: chan, token },
            mem,
            VA_B,
            W379_OFF_B,
            MAGIC_B,
        );
        println!("info  R1\" control A/B     = {a:?} / {b:?}");
        if !a.landed() || !b.landed() {
            println!(
                "??    R1\" CONTROL FAILED  = the two aliases were not both live BEFORE the \
                 unmap. ⊘ UNINTERPRETABLE — this is R1'`s question, not this rung's"
            );
            return false;
        }
        control_ok = true;
        println!("ok    R1\" control         = both aliases landed before the unmap");

        // ── the event ───────────────────────────────────────────────────────────────────
        println!("ALIAS_MARK=unmap_a_begin va={VA_A:#018x}");
        let unmapped = rm.unmap_local(vas, VA_A);
        println!("ALIAS_MARK=unmap_a_end ok={}", unmapped.is_ok());
        if unmapped.is_err() {
            println!("??    R1\" unmap           = RM refused the unmap: {unmapped:?}");
            return false;
        }
        live.retain(|v| *v != VA_A);

        // ── what the host driver says, ungraded but recorded ────────────────────────────
        let pde_a = w379_host_va(rm, vas, VA_A);
        let pde_b = w379_host_va(rm, vas, VA_B);
        println!(
            "info  R1\" host after unmap = A {} / B {} ⊘ UNGRADED — PDE granularity cannot \
             see a leaf go away",
            pde_a.as_str(),
            pde_b.as_str()
        );
        let free_a = rm.probe_va(vas.raw() as u32, VA_A, W379_BYTES);
        println!("info  R1\" allocator says A = {free_a:?} (want Free — the VA is reusable again)");

        // ── the graded fact ─────────────────────────────────────────────────────────────
        let b2 = w379_release_through(
            rm,
            probe,
            W381Chan { h: chan, token },
            mem,
            VA_B,
            W379_OFF_A2,
            MAGIC_B2,
        );
        println!("info  R1\" release via B#2 = {b2:?}");
        match b2 {
            W379Release::Landed => {
                println!(
                    "★     R1\" SIBLING SURVIVED = unmapping VA_A left VA_B live. ⇒ RM \
                     tracks the two aliases INDEPENDENTLY, and any fix that keys host \
                     backing by frame alone is weaker than the driver it stands in for"
                );
                true
            }
            W379Release::Lost { saw } => {
                println!(
                    "FAIL  R1\" SIBLING KILLED  = unmapping VA_A silenced VA_B \
                     ({saw:#010x}, want {MAGIC_B2:#010x}). ⇒ the revoke is symmetric and \
                     the unmap side needs the same (phys, va) key the publish side does"
                );
                false
            }
            W379Release::Refused => {
                println!("??    R1\" release via B#2  = refused, not lost");
                false
            }
        }
    };

    let ok = go();

    for va in live {
        let _ = rm.unmap_local(vas, va);
    }
    if let Some(h) = mem_h {
        let _ = rm.free(h);
    }
    if let Some(h) = chan_h {
        let _ = rm.free(h);
    }
    let _ = rm.free(vas);

    println!(
        "RUNGCTL_alias_unmap={}",
        if control_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "RUNG_alias_unmap={}",
        if !control_ok {
            "NOTRUN"
        } else if ok {
            "PASS"
        } else {
            "FAIL"
        }
    );
    ok
}

/// ★★★★★ **w379 R2 — ALLOCATION PROPAGATION: does a mapping at a DICTATED VA actually land
/// in the host's page tables?**
///
/// Allocate, map at a VA we chose, and then ask the **host driver** — not our own
/// bookkeeping — whether that VA resolves. ★ The independence is the point: this repo's
/// census has been wrong before, and *"a probe that shares the allocator is not an
/// observer"* is one of its paid-for lessons.
///
/// Five arms, and the **negative** ones are what make the positive one mean anything:
///
/// ```text
///   N1  a VA we never mapped        -> probe_va must say Free       [NEGATIVE CONTROL]
///   N2  the same VA, via pde_info   -> recorded, ungraded
///   P1  map at the dictated VA      -> RM must place it exactly there
///   P2  probe_va at the mapped VA   -> must NOT say Free
///   P3  pde_info at the mapped VA   -> PDE_COVERS, or UNMEASURED and said so
///   P4  unmap, then probe_va again  -> must be Free once more
/// ```
///
/// ⊘ **N1 is not decoration.** `probe_va` answers by trying to place a fresh object at the
/// address; if it could never refuse, `Free` would be free and P2 would be vacuous. An arm
/// whose refusing branch is unreachable proves nothing — the same shape `executor_vas_probe`
/// already documents one object over.
///
/// ⊘ **P3 may be UNMEASURED and that is not a failure.**
/// `NV0080_CTRL_CMD_DMA_GET_PTE_INFO` answers `NV_ERR_TEST_ONLY_CODE_NOT_ENABLED` on a
/// release driver — [`HostRmBackend::pde_info`]'s docs record why the PDE sibling exists at
/// all — so a refusal here is the driver declining to answer, never the VA failing to
/// resolve. Grading it as a red would manufacture the finding.
fn map_propagation(rm: &mut HostRmBackend, gpu: u32) -> bool {
    const VA_MAPPED: u64 = 0x0000_0009_1100_0000;
    const VA_NEVER: u64 = 0x0000_0009_5100_0000;

    println!(
        "info  R2 propagation      = GPU {gpu}, euid {} — map at a VA we dictate, then ask \
         the HOST DRIVER whether it resolves",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R2 the bar          = an unmapped VA must probe FREE (negative control), the \
         mapped one must NOT, and the VA must be FREE again after the unmap"
    );

    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R2 vaspace          = the rung needs its own address space");
        println!("RUNGCTL_map_propagation=FAIL");
        println!("RUNG_map_propagation=NOTRUN");
        return false;
    };
    let space = vas.raw() as u32;

    // ── N1 — the negative control, FIRST, before anything is mapped ─────────────────────
    let n1 = rm.probe_va(space, VA_NEVER, W379_BYTES);
    let n1_free = matches!(n1, Ok(kayfabe_isolate_host::rm::VaProbe::Free));
    println!("info  R2 N1 unmapped VA   = {n1:?} at {VA_NEVER:#018x}");
    let n2 = w379_host_va(rm, vas, VA_NEVER);
    println!(
        "info  R2 N2 pde(unmapped) = {} ⊘ recorded, UNGRADED",
        n2.as_str()
    );
    if !n1_free {
        println!(
            "??    R2 CONTROL FAILED   = a VA this rung never mapped did not probe Free. \
             ⊘ The instrument cannot distinguish mapped from unmapped, so every arm below \
             is UNINTERPRETABLE"
        );
        let _ = rm.free(vas);
        println!("RUNGCTL_map_propagation=FAIL");
        println!("RUNG_map_propagation=NOTRUN");
        return false;
    }
    println!("ok    R2 control          = the unmapped VA probes Free — the instrument can say no");

    let mut ok = true;
    let mut mem_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut mapped_at: Option<u64> = None;

    if let Ok(mem) = rm.alloc_probe_local(W379_BYTES) {
        mem_h = Some(mem);
        // ── P1 — the dictated placement ─────────────────────────────────────────────────
        match rm.map_local_at(vas, mem, W379_BYTES, Some(VA_MAPPED)) {
            Ok(got) if got == VA_MAPPED => {
                mapped_at = Some(got);
                println!("ok    R2 P1 placed as asked = {got:#018x}");
            }
            Ok(got) => {
                mapped_at = Some(got);
                ok = false;
                println!(
                    "FAIL  R2 P1 placement     = asked {VA_MAPPED:#018x}, RM chose {got:#018x}"
                );
            }
            Err(e) => {
                ok = false;
                println!("FAIL  R2 P1 map           = refused {e:?}");
            }
        }

        if mapped_at == Some(VA_MAPPED) {
            // ── P2 — the allocator's answer ─────────────────────────────────────────────
            let p2 = rm.probe_va(space, VA_MAPPED, W379_BYTES);
            let p2_free = matches!(p2, Ok(kayfabe_isolate_host::rm::VaProbe::Free));
            println!("info  R2 P2 mapped VA     = {p2:?}");
            if p2_free {
                ok = false;
                println!(
                    "FAIL  R2 P2               = the allocator says a VA WE JUST MAPPED is \
                     free. The mapping did not land in the space we asked about"
                );
            } else {
                println!("ok    R2 P2               = the allocator refuses the mapped VA");
            }

            // ── P3 — the page-table answer ──────────────────────────────────────────────
            let p3 = w379_host_va(rm, vas, VA_MAPPED);
            match p3 {
                W379HostVa::PdeCovers => {
                    println!("★     R2 P3 host tables   = PDE_COVERS at {VA_MAPPED:#018x}")
                }
                W379HostVa::PdeAbsent => {
                    ok = false;
                    println!(
                        "FAIL  R2 P3 host tables   = PDE_ABSENT at a VA we mapped — the \
                         publication did not reach the host page tables"
                    );
                }
                W379HostVa::Unmeasured => println!(
                    "⊘     R2 P3 host tables   = UNMEASURED (the control refused). NOT a \
                     failure value: `GET_PDE_INFO`/`GET_PTE_INFO` are declined by release \
                     drivers, and a refusal is not an absent mapping"
                ),
            }

            // ── P4 — and the VA comes back ──────────────────────────────────────────────
            if rm.unmap_local(vas, VA_MAPPED).is_ok() {
                mapped_at = None;
                let p4 = rm.probe_va(space, VA_MAPPED, W379_BYTES);
                let p4_free = matches!(p4, Ok(kayfabe_isolate_host::rm::VaProbe::Free));
                println!("info  R2 P4 after unmap   = {p4:?}");
                if p4_free {
                    println!("ok    R2 P4               = the VA is reusable again");
                } else {
                    ok = false;
                    println!(
                        "FAIL  R2 P4               = the VA did not come back after the \
                         unmap — a leak that a stress rung would hit as exhaustion"
                    );
                }
            } else {
                ok = false;
                println!("FAIL  R2 P4 unmap         = refused");
            }
        }
    } else {
        ok = false;
        println!("FAIL  R2 object           = device-local allocation refused");
    }

    if let Some(at) = mapped_at {
        let _ = rm.unmap_local(vas, at);
    }
    if let Some(h) = mem_h {
        let _ = rm.free(h);
    }
    let _ = rm.free(vas);

    println!("RUNGCTL_map_propagation=PASS");
    println!("RUNG_map_propagation={}", if ok { "PASS" } else { "FAIL" });
    ok
}

/// ★★★★★ **w379 R3 — A MISSING PAGE, ON PURPOSE: is the fault CONTAINED, and is it NAMED?**
///
/// Every fault this campaign has met, it met by accident. This rung makes one deliberately
/// and asserts the two properties a fault has to have before anything can be built on top of
/// it:
///
/// ```text
///   1  a BYSTANDER channel, in its OWN address space, releases -> must land   [CONTROL]
///   2  a VICTIM channel releases into a VA that was NEVER mapped -> must NOT land
///   3  the victim's ERROR NOTIFIER must have fired, with a NAME
///   4  the bystander releases AGAIN                            -> must STILL land
/// ```
///
/// ★★ **Step 4 is containment**, and step 1 is what makes it readable: without a bystander
/// that was already working, *"the bystander stopped"* and *"the bystander never worked"*
/// are the same observation. The two channels live in **separate address spaces** so the
/// fault cannot reach the bystander through shared page tables.
///
/// ★ **Step 3 is "named, not silent."** `ErrorNotifierRead::fired()` keys on `status`, the
/// field RM writes last, and `except_type` carries `ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`
/// (`0x1f`) — the same number a host kernel log prints as **`Xid 31`**. A fault that kills a
/// channel without writing that record is a *silent* fault, and this rung fails on it even
/// though containment held.
///
/// ⚠ **This rung deliberately provokes a real `Xid 31` and kills its victim channel.** That
/// is the measurement, not a side effect.
///
/// ⊘ Pre-registered: a run where the victim's release **lands** means the VA resolved after
/// all — the address was not as unmapped as we thought — and is reported as `NOTRUN`, not as
/// a pass. An unmapped-address rung whose address turned out to be mapped measured nothing.
fn missing_page_fault(rm: &mut HostRmBackend, probe: W381Probe, gpu: u32) -> bool {
    const BYST_RING_AT: u64 = 0x0000_0006_5100_0000;
    const BYST_TARGET: u64 = 0x0000_0007_5100_0000;
    const VICT_RING_AT: u64 = 0x0000_0006_7100_0000;
    /// Never mapped by anything in this program, in a space that holds only the victim's
    /// own ring — so a resolution here would be RM's, not ours.
    const VICT_NEVER_MAPPED: u64 = 0x0000_000A_1100_0000;
    const MAGIC_LIVE: u32 = 0xB0DE_0001;
    const MAGIC_LIVE2: u32 = 0xB0DE_0002;
    const MAGIC_DEAD: u32 = 0xDEAD_BEEF;

    println!(
        "info  R3 missing page     = GPU {gpu}, euid {} — fault a channel on a VA that was \
         never mapped, with a BYSTANDER running in its own address space",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R3 the bar          = (a) CONTAINED: the bystander lands before AND after; \
         (b) NAMED: the victim's error notifier fires with an exception type"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  R3 engine           = COPY0 is not expressible");
        println!("RUNGCTL_missing_page=FAIL");
        println!("RUNG_missing_page=NOTRUN");
        return false;
    };
    let (Ok(byst_vas), Ok(vict_vas)) = (rm.alloc_vaspace(), rm.alloc_vaspace()) else {
        println!("FAIL  R3 vaspaces         = the rung needs TWO address spaces");
        println!("RUNGCTL_missing_page=FAIL");
        println!("RUNG_missing_page=NOTRUN");
        return false;
    };

    let mut control_ok = false;
    let mut byst_map: Option<u64> = None;
    let mut handles: Vec<kayfabe_isolate::HostHandle> = Vec::new();

    let mut go = || -> Option<bool> {
        // ── the bystander ───────────────────────────────────────────────────────────────
        let (bchan, btok) = rm
            .alloc_channel_at(byst_vas, engine_type, Some(GpuVa(BYST_RING_AT)))
            .ok()?;
        handles.push(bchan);
        rm.schedule(bchan).ok()?;
        let bmem = rm.alloc_probe_local(W379_BYTES).ok()?;
        handles.push(bmem);
        rm.fill_words(bmem, W379_BYTES, W379_SENTINEL, 0).ok()?;
        let at = rm
            .map_local_at(byst_vas, bmem, W379_BYTES, Some(BYST_TARGET))
            .ok()?;
        byst_map = Some(at);
        if at != BYST_TARGET {
            println!("??    R3 bystander map     = placed at {at:#018x}, not as asked");
            return None;
        }
        let before = w379_release_through(
            rm,
            probe,
            W381Chan {
                h: bchan,
                token: btok,
            },
            bmem,
            BYST_TARGET,
            W379_OFF_A,
            MAGIC_LIVE,
        );
        println!("info  R3 bystander before = {before:?}");
        if !before.landed() {
            println!(
                "??    R3 CONTROL FAILED   = the bystander never worked, so *\"it kept \
                 working\"* is unobservable. ⊘ UNINTERPRETABLE"
            );
            return None;
        }
        control_ok = true;
        println!("ok    R3 control          = the bystander lands BEFORE the fault");

        // ── the victim, with a notifier so the fault can be NAMED ────────────────────────
        let notifier = rm.alloc_sysmem(0x1000).ok()?;
        handles.push(notifier);
        let (vchan, vtok) = rm
            .alloc_channel_at_with_error_notifier(
                vict_vas,
                engine_type,
                Some(GpuVa(VICT_RING_AT)),
                notifier,
            )
            .ok()?;
        handles.push(vchan);
        rm.schedule(vchan).ok()?;
        // A scratch object ONLY so the poll has something to read; it is deliberately NOT
        // mapped at `VICT_NEVER_MAPPED`, so the sentinel it holds can never be overwritten.
        let vmem = rm.alloc_probe_local(W379_BYTES).ok()?;
        handles.push(vmem);
        rm.fill_words(vmem, W379_BYTES, W379_SENTINEL, 0).ok()?;

        println!("FAULT_MARK=victim_release va={VICT_NEVER_MAPPED:#018x}");
        let victim = w379_release_through(
            rm,
            probe,
            W381Chan {
                h: vchan,
                token: vtok,
            },
            vmem,
            VICT_NEVER_MAPPED,
            W379_OFF_A,
            MAGIC_DEAD,
        );
        println!("info  R3 victim           = {victim:?}");
        if victim.landed() {
            println!(
                "??    R3 NOTHING FAULTED  = the release into a VA this rung never mapped \
                 LANDED. ⊘ The address was not unmapped, so no fault was provoked and \
                 nothing here is a result"
            );
            return None;
        }

        // ── (b) NAMED ───────────────────────────────────────────────────────────────────
        let named = match rm
            .read_error_notifier(notifier, kayfabe_isolate_host::rm::NotifierAperture::Sysmem)
        {
            Ok(n) => {
                println!(
                    "info  R3 notifier         = fired={} status={:#06x} except_type={:#x} \
                     engine={:#06x}",
                    n.fired(),
                    n.status,
                    n.except_type,
                    n.engine_type
                );
                if n.fired() {
                    println!(
                        "★     R3 NAMED            = the driver wrote a robust-channel \
                         record. except_type {:#x} is what a host kernel log prints as \
                         `Xid {}`",
                        n.except_type, n.except_type
                    );
                    true
                } else {
                    println!(
                        "FAIL  R3 SILENT           = the channel stopped and the notifier is \
                         quiet. ⊘ A fault nobody names cannot be handled, and `status == 0` \
                         is also what an unwired notifier reads as — both are failures of \
                         this bar"
                    );
                    false
                }
            }
            Err(e) => {
                println!("FAIL  R3 notifier         = unreadable {e:?}");
                false
            }
        };

        // ── (a) CONTAINED ───────────────────────────────────────────────────────────────
        let after = w379_release_through(
            rm,
            probe,
            W381Chan {
                h: bchan,
                token: btok,
            },
            bmem,
            BYST_TARGET,
            W379_OFF_A2,
            MAGIC_LIVE2,
        );
        println!("info  R3 bystander after  = {after:?}");
        let contained = after.landed();
        if contained {
            println!(
                "★     R3 CONTAINED        = a channel in another address space kept \
                 landing across the fault"
            );
        } else {
            println!(
                "FAIL  R3 NOT CONTAINED    = the bystander landed before the fault and is \
                 silent after it. The fault took a channel it does not own"
            );
        }
        Some(named && contained)
    };

    let out = go();

    if let Some(at) = byst_map {
        let _ = rm.unmap_local(byst_vas, at);
    }
    for h in handles.into_iter().rev() {
        let _ = rm.free(h);
    }
    let _ = rm.free(byst_vas);
    let _ = rm.free(vict_vas);

    println!(
        "RUNGCTL_missing_page={}",
        if control_ok { "PASS" } else { "FAIL" }
    );
    match out {
        Some(true) => {
            println!("RUNG_missing_page=PASS");
            true
        }
        Some(false) => {
            println!("RUNG_missing_page=FAIL");
            false
        }
        None => {
            println!("RUNG_missing_page=NOTRUN");
            false
        }
    }
}

/// ★★★★★ **w379 R5 — THE STRESS RUNG: many alloc/map/free cycles, INTERLEAVED, with a
/// rolling window of live mappings.**
///
/// The owner's ask was a *"mean stress test"*, and the four things it has to be able to see
/// are named up front so the rung can be checked against its own claims:
///
/// ```text
///   handle recycling collisions  -> a recycled object must read as its OWN sentinel, never
///                                   as the magic the previous tenant of that VA released
///   extents drifting             -> every cycle maps the same length at the same VA and
///                                   asserts the placement is EXACT, every time
///   rows leaking after free      -> after each unmap the VA must probe Free again
///   interference between live
///   mappings                     -> every live slot is re-released EVERY cycle, so a
///                                   mapping broken by a LATER one is caught at the cycle
///                                   that broke it rather than at teardown
/// ```
///
/// ★★ **The window is what makes this a stress rung rather than a loop.** Four slots are
/// live at once and the allocations are recycled out from under each other, so allocate,
/// map, free and unmap are **interleaved** instead of strictly nested. A strictly nested
/// loop exercises one object at a time and cannot see any of the four failures above.
///
/// ⊘ **WHAT THIS RUNG DOES NOT COVER, STATED SO IT IS NOT READ AS COVERAGE.**
/// **Cross-client leakage is NOT tested here.** *"Two guest processes must not see each
/// other's mappings"* is a standing security requirement and it needs a **second RM client**
/// — a second `HostRmBackend` over a second connection — which this binary builds exactly
/// one of. A rung that ran one client and reported on two would be worse than no rung.
/// ⚠ Named as the follow-up, not left implied.
fn map_stress(rm: &mut HostRmBackend, probe: W381Probe, gpu: u32) -> bool {
    /// Live mappings held at once. Four, so allocate/free interleave rather than nest.
    const SLOTS: usize = 4;
    /// Cycles. Each recycles ONE slot and re-releases through ALL of them.
    const CYCLES: usize = 48;
    const RING_AT: u64 = 0x0000_0006_9100_0000;
    /// The slot VAs: 4 GiB apart, so no slot's mapping can be covered by a neighbour's big
    /// PTE — the confound `alias_two_vas` avoids for the same reason.
    const SLOT_BASE: u64 = 0x0000_000B_1100_0000;
    const SLOT_STRIDE: u64 = 0x0000_0001_0000_0000;

    println!(
        "info  R5 stress           = GPU {gpu}, euid {} — {CYCLES} cycles over {SLOTS} \
         interleaved live mappings, recycling one slot per cycle",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R5 the bar          = every release lands, every placement is EXACT, every \
         freed VA probes Free again, and a recycled object reads as its OWN sentinel"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  R5 engine           = COPY0 is not expressible");
        println!("RUNGCTL_map_stress=FAIL");
        println!("RUNG_map_stress=NOTRUN");
        return false;
    };
    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R5 vaspace          = the rung needs its own address space");
        println!("RUNGCTL_map_stress=FAIL");
        println!("RUNG_map_stress=NOTRUN");
        return false;
    };
    let space = vas.raw() as u32;

    // ⊘ EXACT COUNTS over a fixed denominator, and the failure lists are CAPPED while the
    // counts are not — a sample that hid the total would be the *"a capped list is not a
    // census"* failure this repo has paid for.
    const SHOW: usize = 5;
    let mut cycles_run = 0usize;
    let mut rel_tried = 0usize;
    let mut rel_landed = 0usize;
    let mut placed_exact = 0usize;
    let mut placed_tried = 0usize;
    let mut free_ok = 0usize;
    let mut free_tried = 0usize;
    let mut stale_reads = 0usize;
    let mut first_failures: Vec<String> = Vec::new();
    let mut control_ok = false;

    let mut chan_h: Option<kayfabe_isolate::HostHandle> = None;
    // (object, va, the magic its last release wrote)
    let mut slots: Vec<Option<(kayfabe_isolate::HostHandle, u64, u32)>> = vec![None; SLOTS];

    let mut go = || -> bool {
        let Ok((chan, token)) = rm.alloc_channel_at(vas, engine_type, Some(GpuVa(RING_AT))) else {
            println!("??    R5 channel          = refused at {RING_AT:#018x}");
            return false;
        };
        chan_h = Some(chan);
        if rm.schedule(chan).is_err() {
            println!("??    R5 schedule         = refused");
            return false;
        }

        for cycle in 0..CYCLES {
            let s = cycle % SLOTS;
            let va = SLOT_BASE + (s as u64) * SLOT_STRIDE;

            // ── retire the slot's current tenant, and assert the VA comes back ───────────
            if let Some((old_mem, old_va, _)) = slots[s].take() {
                let un = rm.unmap_local(vas, old_va).is_ok();
                let _ = rm.free(old_mem);
                free_tried += 1;
                let probe = rm.probe_va(space, old_va, W379_BYTES);
                if un && matches!(probe, Ok(kayfabe_isolate_host::rm::VaProbe::Free)) {
                    free_ok += 1;
                } else if first_failures.len() < SHOW {
                    first_failures.push(format!(
                        "cycle {cycle}: VA {old_va:#018x} did not come back after free \
                         (unmap_ok={un}, probe={probe:?})"
                    ));
                }
            }

            // ── a fresh tenant, sentinel-filled BEFORE it is mapped ─────────────────────
            let Ok(mem) = rm.alloc_probe_local(W379_BYTES) else {
                if first_failures.len() < SHOW {
                    first_failures.push(format!("cycle {cycle}: allocation refused"));
                }
                continue;
            };
            if rm.fill_words(mem, W379_BYTES, W379_SENTINEL, 0).is_err() {
                let _ = rm.free(mem);
                continue;
            }
            placed_tried += 1;
            let got = match rm.map_local_at(vas, mem, W379_BYTES, Some(va)) {
                Ok(g) => g,
                Err(e) => {
                    if first_failures.len() < SHOW {
                        first_failures
                            .push(format!("cycle {cycle}: map at {va:#018x} refused {e:?}"));
                    }
                    let _ = rm.free(mem);
                    continue;
                }
            };
            if got == va {
                placed_exact += 1;
            } else if first_failures.len() < SHOW {
                first_failures.push(format!(
                    "cycle {cycle}: EXTENT/PLACEMENT DRIFT — asked {va:#018x}, got {got:#018x}"
                ));
            }

            // ★ HANDLE RECYCLING: the fresh object must read as its OWN sentinel. If RM
            // handed back the previous tenant's storage, or if our mapping still names it,
            // the word here is the magic that tenant released — a value this new object has
            // no other way to hold.
            if let Ok(w) = rm.read_words_independently(mem, W379_BYTES, &[W379_OFF_A])
                && w[0] != W379_SENTINEL
            {
                stale_reads += 1;
                if first_failures.len() < SHOW {
                    first_failures.push(format!(
                        "cycle {cycle}: STALE READ at {va:#018x} — fresh object holds \
                         {:#010x}, not the sentinel {W379_SENTINEL:#010x}",
                        w[0]
                    ));
                }
            }
            slots[s] = Some((mem, got, 0));

            // ── re-release through EVERY live slot, so a mapping broken by a LATER one is
            //    caught at the cycle that broke it ────────────────────────────────────────
            for (i, slot) in slots.iter_mut().enumerate() {
                let Some((smem, sva, last)) = slot else {
                    continue;
                };
                // A payload unique to (cycle, slot): a stale word from the previous release
                // can never satisfy it.
                let payload = 0x5715_0000u32 | ((cycle as u32) << 4) | (i as u32);
                rel_tried += 1;
                let out = w379_release_through(
                    rm,
                    probe,
                    W381Chan { h: chan, token },
                    *smem,
                    *sva,
                    W379_OFF_A,
                    payload,
                );
                if out.landed() {
                    rel_landed += 1;
                    *last = payload;
                    if !control_ok {
                        control_ok = true;
                        println!(
                            "ok    R5 control          = the first release of the first \
                             cycle landed — the channel and the ring work, so a later red \
                             is the mapping plane and not the harness"
                        );
                    }
                } else if first_failures.len() < SHOW {
                    first_failures.push(format!(
                        "cycle {cycle} slot {i}: release through {sva:#018x} did not land \
                         ({out:?}, last good {last:#010x})"
                    ));
                }
            }
            cycles_run += 1;
        }
        true
    };

    let ran = go();

    for slot in slots.iter().flatten() {
        let _ = rm.unmap_local(vas, slot.1);
        let _ = rm.free(slot.0);
    }
    if let Some(h) = chan_h {
        let _ = rm.free(h);
    }
    let _ = rm.free(vas);

    println!(
        "info  R5 census           = cycles {cycles_run}/{CYCLES}  releases \
         {rel_landed}/{rel_tried}  placements exact {placed_exact}/{placed_tried}  \
         VAs recovered {free_ok}/{free_tried}  stale reads {stale_reads}"
    );
    if !first_failures.is_empty() {
        println!(
            "⚠     R5 failures         = showing {} of {} recorded",
            first_failures.len().min(SHOW),
            first_failures.len()
        );
        for f in first_failures.iter().take(SHOW) {
            println!("        {f}");
        }
    }

    let clean = ran
        && cycles_run == CYCLES
        && rel_tried > 0
        && rel_landed == rel_tried
        && placed_exact == placed_tried
        && free_ok == free_tried
        && stale_reads == 0;
    if clean {
        println!(
            "★     R5 STRESS CLEAN     = {rel_landed} releases over {cycles_run} \
             interleaved cycles, every placement exact, every freed VA recovered, no stale \
             read. ⊘ Single-client only — cross-client leakage is NOT covered here"
        );
    }

    println!(
        "RUNGCTL_map_stress={}",
        if control_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "RUNG_map_stress={}",
        if !control_ok {
            "NOTRUN"
        } else if clean {
            "PASS"
        } else {
            "FAIL"
        }
    );
    clean
}

/// ★★★ **w381 — WRITE one magic into `obj_va + off` WITH THE ENGINE**, by whichever probe
/// this run selected.
///
/// ⊘ Split out of [`w379_release_through`] because that function's read-back is a **CPU**
/// load through [`HostRmBackend::read_words_independently`], and half of R4's objects have no
/// CPU view at all. The submit half is aperture-blind; only the observe half is not.
///
/// Returns whether the submission was **accepted**, never whether it landed. The landing is
/// the caller's to observe, and conflating the two is how a refused submission comes to read
/// as a dead mapping.
fn w381_engine_write(
    rm: &mut HostRmBackend,
    probe: W381Probe,
    chan: kayfabe_isolate::HostHandle,
    token: u64,
    obj_va: u64,
    off: u64,
    magic: u32,
) -> bool {
    match probe {
        W381Probe::SemRelease => rm
            .submit_release_at(chan, token, obj_va + off, magic)
            .is_ok(),
        W381Probe::LaunchDma => {
            let (src_off, _) = kayfabe_isolate_host::rm::HostRmBackend::copy_probe_offsets();
            rm.ring_store_u32(chan, src_off, magic).is_ok()
                && rm
                    .submit_copy_at(chan, token, src_off, obj_va + off, 4, W381_RETIRE_PAYLOAD)
                    .is_ok()
        }
    }
}

/// ★★★★★ **w381 — READ one word back THROUGH THE ENGINE**, into a scratch object the CPU
/// *can* see.
///
/// This is always a `LAUNCH_DMA`, on both probe arms: it is the only verb that can move bytes
/// at all, and a `SemRelease` cannot read anything. ⚠ The scratch slot must be poisoned
/// first — see [`HostRmBackend::submit_copy_va`].
fn w381_engine_readback(
    rm: &mut HostRmBackend,
    chan: kayfabe_isolate::HostHandle,
    token: u64,
    src_va: u64,
    scratch_va: u64,
    slot: usize,
) -> bool {
    rm.submit_copy_va(
        chan,
        token,
        src_va,
        scratch_va + (slot as u64) * 4,
        4,
        W381_RETIRE_PAYLOAD,
    )
    .is_ok()
}

/// ★★★★★ **w381 R4 — RPC-MIXED ALLOCATIONS: does ORDERING and IDENTITY survive the mix?**
///
/// Owner 2026-09-06, naming the gap the w379 lane left open by name: *"mixing rpc with
/// normal allocs"*. Nothing in this tree tested it, and the shape of the defect it looks for
/// is the one this campaign has met most often — **two objects that come to share one
/// answer**.
///
/// ```text
///   1  N VIDMEM and N SYSMEM objects, ALLOCATED INTERLEAVED with a device CONTROL
///      between every pair, each mapped at a VA this rung dictates
///   2  a DISTINCT magic written into every one of them BY THE ENGINE, then every one of
///      them read back BY THE ENGINE into a poisoned scratch the CPU can see
///        -> each must hold ITS OWN magic and no other object's            [IDENTITY]
///        -> object #0 of each family is that family's                     [CONTROL]
///   3  the whole SYSMEM family torn down; the VIDMEM family re-written and re-read
///        -> every survivor must still land                                [ORDERING]
/// ```
///
/// # ★★★★★ THE READ-BACK IS THE ENGINE'S, AND THAT IS FORCED — NOT A PREFERENCE
///
/// The obvious readback, [`HostRmBackend::read_words_independently`], is a **CPU** load, and
/// `[measured 2026-09-06 on this bench, and predicted by `alloc_probe_local`'s own docs]`
/// **every sysmem object in this rung refuses a CPU mapping**: `RmBackend::alloc_sysmem`
/// passes `NVOS02_FLAGS_MAPPING_NO_MAP`, so `NV_ESC_RM_MAP_MEMORY` on the result is refused —
/// *"a published backing is opaque to the CPU in both directions, by design"*. The first
/// version of this rung wrote the sentinel with `fill_words` and died there, six times, with
/// `RUNGCTL_rpc_mixed=FAIL`.
///
/// ⇒ Every object is written **and read** by the copy engine, and only a scratch VIDMEM
/// object is ever touched by the CPU. ★ That is strictly **more** evidence than the CPU path
/// would have been: it proves the engine can *read* each address under test as well as write
/// it, which a write-only probe cannot say.
///
/// # ⊘ WHAT "RPC-MIXED" MEANS HERE, STATED HONESTLY BECAUSE THE NAME OVERPROMISES
///
/// This rung interleaves **three** request families that are known to take different paths
/// inside RM:
///
/// - `NV01_MEMORY_LOCAL_USER` — device-local vidmem. Under Mode 2 its storage is the
///   **emulated framebuffer**, and `cpu_ce`'s router reaches it through `CpuPlane::Fb`.
/// - `NV01_MEMORY_SYSTEM` — pages RM pins out of system memory. Under Mode 2 its storage is
///   **guest RAM**, reached through `CpuPlane::GuestRam`. ★ The two planes are *"two number
///   spaces that collide freely"* by `ceutils`'s own account, and serving one out of the
///   other's store is the silent-wrong-bytes failure the address plane exists to refuse.
///   That is exactly what the identity phase is aimed at.
/// - `NV0080_CTRL_CMD_DMA_GET_PDE_INFO` — a device **control**, issued between every
///   allocation, so no two allocations of the same family are ever adjacent.
///
/// ⊘ **Which of those crosses to the emulated GSP is RM's routing decision, and this rung
/// does NOT measure it.** Calling the mix "RPC-mixed" is a statement about the request
/// families being different, not a claim that a particular one produced a `GSP_RM_ALLOC`. A
/// rung that asserted the crossing would need an instrument inside the device, and one that
/// *claimed* it without one would be the `pde_info` mistake in a new place.
///
/// ⊘ The interleaved control's **result is recorded and UNGRADED**, for the same reason R2
/// does not grade `pde_info`: `GET_PDE_INFO` answers `PDE_COVERS` at addresses the run never
/// mapped, and a refusal is not an absent mapping. It is here as a **perturbation**, not as
/// an oracle.
///
/// ## ★★★ PRE-REGISTERED, BEFORE THE RUN
///
/// - **both controls land, identity clean, ordering clean** ⇒ PASS. The mix is inert.
/// - **a CROSSTALK** — an object holding *another object's* magic ⇒ ★★★★★ FAIL, and it is the
///   strong red: two allocations resolved to one backing. Reported separately from *"never
///   written"*, which is a dead mapping and a different defect, and separately again from
///   *"still the scratch poison"*, which is a readback that never ran.
/// - **an ORDERING failure** — a surviving vidmem object stops answering once the sysmem
///   family is freed ⇒ FAIL. A teardown reached across families.
/// - **either family's object #0 fails** ⇒ ⊘ `NOTRUN`. A family that could not be written
///   even once makes every statement about the mix uninterpretable. ⚠ Including the sysmem
///   one: a rung that quietly graded on the vidmem half alone would be *"every row verified"*
///   over half the rows.
fn rpc_mixed_allocs(rm: &mut HostRmBackend, probe: W381Probe, gpu: u32) -> bool {
    /// Objects **per family**. Small enough that the whole census fits on one screen and
    /// every failure can be printed rather than sampled.
    const N: usize = 6;
    const RING_AT: u64 = 0x0000_0006_9100_0000;
    /// 4 GiB apart, so no object's mapping can be covered by a neighbour's big PTE — the
    /// confound `alias_two_vas` and `map_stress` both avoid for the same reason.
    const STRIDE: u64 = 0x0000_0001_0000_0000;
    const VA_VID_BASE: u64 = 0x0000_000C_1100_0000;
    const VA_SYS_BASE: u64 = 0x0000_0014_1100_0000;
    /// The one object the CPU ever touches. Clear of both families' spans.
    const VA_SCRATCH: u64 = 0x0000_001A_1100_0000;
    /// How long the engine is given to retire a whole pass before the scratch is read.
    const SETTLE: std::time::Duration = std::time::Duration::from_millis(400);
    /// How many failures are printed. A capped list is not a census, so the **count** is
    /// printed beside the cap on every run.
    const SHOW: usize = 8;

    /// The magic for one object. Distinct per (pass, family, index), so a word left over from
    /// an earlier pass can never satisfy a later one, and a word belonging to a *different*
    /// object is recognisable as that object's rather than merely wrong.
    fn magic(pass: u32, vid: bool, i: usize) -> u32 {
        0x8410_0000 | (pass << 12) | (u32::from(vid) << 8) | (i as u32)
    }
    fn fam(vid: bool) -> &'static str {
        if vid { "VIDMEM" } else { "SYSMEM" }
    }

    println!(
        "info  R4 rpc-mixed        = GPU {gpu}, euid {} — {N} VIDMEM + {N} SYSMEM \
         allocations, interleaved, with a device control between every one",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R4 the bar          = every object holds ITS OWN magic (identity), and \
         tearing down the whole SYSMEM family leaves every VIDMEM mapping still writable \
         (ordering). ⊘ Both halves are read BY THE ENGINE: a sysmem object has no CPU view"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  R4 engine           = COPY0 is not expressible");
        println!("RUNGCTL_rpc_mixed=FAIL");
        println!("RUNG_rpc_mixed=NOTRUN");
        return false;
    };
    let Ok(vas) = rm.alloc_vaspace() else {
        println!("FAIL  R4 vaspace          = the rung needs its own address space");
        println!("RUNGCTL_rpc_mixed=FAIL");
        println!("RUNG_rpc_mixed=NOTRUN");
        return false;
    };

    // `(handle, va, is_vidmem, index)` for every object this rung has live.
    let mut live: Vec<(kayfabe_isolate::HostHandle, u64, bool, usize)> = Vec::new();
    let mut chan_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut scratch_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut control_vid = false;
    let mut control_sys = false;
    let mut failures: Vec<String> = Vec::new();
    // Exact counts over fixed denominators. ⊘ Never a sample.
    let mut alloc_ok = 0usize;
    let mut placed_exact = 0usize;
    let mut placed_tried = 0usize;
    let mut ident_ok = 0usize;
    let mut ident_tried = 0usize;
    let mut crosstalk = 0usize;
    let mut never_read = 0usize;
    let mut order_ok = 0usize;
    let mut order_tried = 0usize;
    let mut ctrl_answered = 0usize;
    let mut ctrl_tried = 0usize;

    let mut go = || -> bool {
        let Ok((chan, token)) = rm.alloc_channel_at(vas, engine_type, Some(GpuVa(RING_AT))) else {
            println!("??    R4 channel          = refused at {RING_AT:#018x} — NOT a result");
            return false;
        };
        chan_h = Some(chan);
        if rm.channel_ring_va(chan) != Some(RING_AT) {
            println!("??    R4 ring placement   = RM did not place the ring where asked");
            return false;
        }
        if rm.schedule(chan).is_err() {
            println!("??    R4 schedule         = refused");
            return false;
        }

        // ── the SCRATCH — the one object the CPU ever touches ───────────────────────────
        let Ok(scratch) = rm.alloc_probe_local(W379_BYTES) else {
            println!("??    R4 scratch          = device-local allocation refused");
            return false;
        };
        scratch_h = Some(scratch);
        match rm.map_local_at(vas, scratch, W379_BYTES, Some(VA_SCRATCH)) {
            Ok(got) if got == VA_SCRATCH => {}
            Ok(got) => {
                println!("??    R4 scratch place    = asked {VA_SCRATCH:#018x}, got {got:#018x}");
                return false;
            }
            Err(e) => {
                println!("??    R4 scratch map      = refused {e:?}");
                return false;
            }
        }

        // ── ALLOCATE AND MAP, interleaved, a control after every allocation ─────────────
        //
        // ⚠ Index 0 of each family is that family's CONTROL and is allocated first, in the
        // same loop and by the same code, so a control that passes and a mixed object that
        // fails cannot differ in how they were made.
        for i in 0..N {
            for vid in [true, false] {
                let alloc = if vid {
                    rm.alloc_probe_local(W379_BYTES)
                } else {
                    rm.alloc_sysmem(W379_BYTES)
                };
                let want = if vid { VA_VID_BASE } else { VA_SYS_BASE } + (i as u64) * STRIDE;
                match alloc {
                    Ok(mem) => {
                        alloc_ok += 1;
                        placed_tried += 1;
                        match rm.map_local_at(vas, mem, W379_BYTES, Some(want)) {
                            Ok(got) if got == want => {
                                placed_exact += 1;
                                live.push((mem, got, vid, i));
                            }
                            Ok(got) => {
                                failures.push(format!(
                                    "{} #{i}: PLACEMENT DRIFT — asked {want:#018x}, RM chose \
                                     {got:#018x}",
                                    fam(vid)
                                ));
                                live.push((mem, got, vid, i));
                            }
                            Err(e) => {
                                failures.push(format!(
                                    "{} #{i}: map at {want:#018x} refused {e:?}",
                                    fam(vid)
                                ));
                                let _ = rm.free(mem);
                            }
                        }
                    }
                    Err(e) => failures.push(format!("alloc {} #{i} refused {e:?}", fam(vid))),
                }
                // ⊘ THE PERTURBATION. Recorded, UNGRADED — see the rung's docs for why
                // `GET_PDE_INFO` cannot be an oracle here.
                ctrl_tried += 1;
                if w379_host_va(rm, vas, want) != W379HostVa::Unmeasured {
                    ctrl_answered += 1;
                }
            }
        }

        // ── IDENTITY. Write every object, then read EVERY object. ──────────────────────
        //
        // ★ The two loops are SEPARATE on purpose. Writing and immediately reading one object
        // cannot see a later object landing on top of it, which is precisely the crosstalk
        // this phase exists to find — the same reason `alias_two_vas`'s step 4 is the whole
        // of that rung.
        // ⊘ `failures` is a PARAMETER, not a capture: the caller records into the same list
        // around every call, and a closure that captured it would own the borrow for its
        // whole lifetime.
        let identity = |pass: u32,
                        objs: &[(kayfabe_isolate::HostHandle, u64, bool, usize)],
                        rm: &mut HostRmBackend,
                        failures: &mut Vec<String>|
         -> Vec<Option<u32>> {
            // ⚠ POISON FIRST. A readback that never ran leaves whatever was in the slot, and
            // without a sentinel underneath it that is indistinguishable from a landed copy.
            if rm
                .fill_words(scratch, W379_BYTES, W379_SENTINEL, 0)
                .is_err()
            {
                return vec![None; objs.len()];
            }
            for &(_, va, vid, i) in objs {
                if !w381_engine_write(rm, probe, chan, token, va, W379_OFF_B, magic(pass, vid, i))
                    && failures.len() < 64
                {
                    failures.push(format!(
                        "pass {pass}: WRITE to {} #{i} at {va:#018x} was REFUSED, not lost",
                        fam(vid)
                    ));
                }
            }
            for (slot, &(_, va, vid, i)) in objs.iter().enumerate() {
                if !w381_engine_readback(rm, chan, token, va + W379_OFF_B, VA_SCRATCH, slot)
                    && failures.len() < 64
                {
                    failures.push(format!(
                        "pass {pass}: READBACK of {} #{i} at {va:#018x} was REFUSED",
                        fam(vid)
                    ));
                }
            }
            std::thread::sleep(SETTLE);
            let offs: Vec<u64> = (0..objs.len() as u64).map(|s| s * 4).collect();
            match rm.read_words_independently(scratch, W379_BYTES, &offs) {
                Ok(w) => w.into_iter().map(Some).collect(),
                Err(_) => vec![None; objs.len()],
            }
        };

        let seen = identity(1, &live, rm, &mut failures);
        for (idx, &(_, va, vid, i)) in live.iter().enumerate() {
            ident_tried += 1;
            let want = magic(1, vid, i);
            match seen.get(idx).copied().flatten() {
                Some(v) if v == want => {
                    ident_ok += 1;
                    if i == 0 {
                        if vid {
                            control_vid = true;
                        } else {
                            control_sys = true;
                        }
                    }
                }
                Some(v) if v == W379_SENTINEL => {
                    never_read += 1;
                    if failures.len() < 64 {
                        failures.push(format!(
                            "{} #{i} at {va:#018x}: the scratch slot is STILL THE POISON — the \
                             engine never wrote it back, so this says nothing about the object",
                            fam(vid)
                        ));
                    }
                }
                Some(v) => {
                    // ★★★★★ Is the word ANOTHER object's? That is a much stronger — and much
                    // worse — finding than "nothing arrived", and the two must never be
                    // reported as one number.
                    let owner = live
                        .iter()
                        .find(|(_, _, ov, oj)| magic(1, *ov, *oj) == v)
                        .map(|(_, ova, ov, oj)| format!("{} #{oj} at {ova:#018x}", fam(*ov)));
                    if let Some(who) = owner {
                        crosstalk += 1;
                        if failures.len() < 64 {
                            failures.push(format!(
                                "★★★★★ CROSSTALK — {} #{i} at {va:#018x} holds {v:#010x}, which \
                                 is {who}'s magic",
                                fam(vid)
                            ));
                        }
                    } else if failures.len() < 64 {
                        failures.push(format!(
                            "{} #{i} at {va:#018x}: holds {v:#010x}, want {want:#010x} (nobody's \
                             magic — a dead mapping, not crosstalk)",
                            fam(vid)
                        ));
                    }
                }
                None => {
                    if failures.len() < 64 {
                        failures.push(format!("{} #{i}: the scratch could not be read", fam(vid)));
                    }
                }
            }
        }
        if !(control_vid && control_sys) {
            println!(
                "??    R4 CONTROL FAILED   = vidmem #0={control_vid} sysmem #0={control_sys}. ⊘ \
                 A family whose FIRST object could not be written and read back even once \
                 makes every statement about the MIX uninterpretable, and grading on the half \
                 that worked would be \"every row verified\" over half the rows. Retirement \
                 qualifier: {}",
                w381_retired(rm, chan)
            );
            return false;
        }
        println!(
            "ok    R4 controls         = object #0 of BOTH families round-tripped through the \
             engine — a later red is the MIX and not the harness"
        );

        // ── ORDERING. Tear down ONE family; the other must survive it. ──────────────────
        let sys: Vec<_> = live.iter().filter(|(_, _, v, _)| !*v).copied().collect();
        live.retain(|(_, _, v, _)| *v);
        for (mem, va, _, _) in sys {
            let _ = rm.unmap_local(vas, va);
            let _ = rm.free(mem);
        }
        let after = identity(2, &live, rm, &mut failures);
        for (idx, &(_, va, vid, i)) in live.iter().enumerate() {
            order_tried += 1;
            let want = magic(2, vid, i);
            if after.get(idx).copied().flatten() == Some(want) {
                order_ok += 1;
            } else if failures.len() < 64 {
                failures.push(format!(
                    "ordering: VIDMEM #{i} at {va:#018x} stopped round-tripping after the whole \
                     SYSMEM family was freed (saw {:?}, want {want:#010x})",
                    after.get(idx).copied().flatten()
                ));
            }
        }
        true
    };

    let ran = go();

    for (mem, va, _, _) in &live {
        let _ = rm.unmap_local(vas, *va);
        let _ = rm.free(*mem);
    }
    if let Some(h) = scratch_h {
        let _ = rm.unmap_local(vas, VA_SCRATCH);
        let _ = rm.free(h);
    }
    if let Some(h) = chan_h {
        let _ = rm.free(h);
    }
    let _ = rm.free(vas);

    println!(
        "info  R4 census           = allocated {alloc_ok}/{}  placed exact \
         {placed_exact}/{placed_tried}  identity {ident_ok}/{ident_tried}  CROSSTALK \
         {crosstalk}  never-read {never_read}  ordering {order_ok}/{order_tried}  controls \
         answered {ctrl_answered}/{ctrl_tried} (ungraded)",
        2 * N
    );
    if !failures.is_empty() {
        println!(
            "⚠     R4 failures         = showing {} of {} recorded",
            failures.len().min(SHOW),
            failures.len()
        );
        for f in failures.iter().take(SHOW) {
            println!("        {f}");
        }
    }

    let control_ok = control_vid && control_sys;
    // ⊘ `ident_tried == 2 * N` is part of the bar, not an afterthought: a run that mapped four
    // objects and verified all four would otherwise print a clean identity line while having
    // measured a third of the mix.
    let clean = ran
        && control_ok
        && alloc_ok == 2 * N
        && ident_tried == 2 * N
        && ident_ok == ident_tried
        && crosstalk == 0
        && never_read == 0
        && placed_exact == placed_tried
        && order_tried == N
        && order_ok == order_tried;
    if clean {
        println!(
            "★     R4 MIX IS INERT     = {ident_ok} objects across TWO request families, each \
             holding its own magic under an engine read-back, and every VIDMEM mapping still \
             round-tripping after the whole SYSMEM family was torn down"
        );
    }
    println!(
        "RUNGCTL_rpc_mixed={}",
        if control_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "RUNG_rpc_mixed={}",
        if !control_ok {
            "NOTRUN"
        } else if clean {
            "PASS"
        } else {
            "FAIL"
        }
    );
    clean
}

/// ★★★★★ **w381 R5b — CROSS-CLIENT LEAKAGE. Two RM clients must not see each other's
/// memory.**
///
/// `map_stress` builds **one** RM client and its own docs say so: *"single-client only —
/// cross-client leakage is NOT covered here"*. That gap is not a nice-to-have. **Two guest
/// processes must not see each other's memory** is a standing requirement of this project,
/// and until this rung existed nothing in the tree tested it at all.
///
/// ```text
///   0  client A: own root, own VAS, own channel, object MA at VA_SHARED, magic  [CONTROL]
///      client B: a SECOND /dev/nvidiactl fd and a SECOND NV01_ROOT, own VAS, own
///                channel, object MB at THE SAME VA NUMBER, its own magic        [CONTROL]
///   1  before B maps: B's allocator must say VA_SHARED is FREE, while A holds it   [N1]
///   2  after both are live and both were written: A must still read A's magic     [LEAK]
///                                                  B must still read B's magic
///   3  A writes AGAIN through VA_SHARED; it must land, and B must not see it   [ORDERING]
///   4  the two clients' raw handle numbers are printed and compared             [IDENTITY]
/// ```
///
/// # ★★★ WHY THE SHARED VA NUMBER IS THE WHOLE RUNG
///
/// A GPU VA means nothing outside the address space it is resolved in. Two clients naming
/// the identical 64-bit number is therefore the sharpest available test of that claim: if
/// the number alone were enough to reach memory, this is the arrangement in which it would
/// show, and it would show as A reading **B's** magic — a value A has no other way to hold.
///
/// ★ **And the handle comparison in step 4 is not decoration.** Both connections mint object
/// handles from their own counters, so the two clients very often end up holding the *same
/// raw handle number* for different memory. When they do, this rung has measured
/// *"identical handle, identical VA, different bytes"* in one run, which is the strongest
/// form of the property. When they do not, the run says so rather than claiming it.
///
/// # ★★★ THE FOREIGN-HANDLE ARM REACHES RM — I ASSUMED IT COULD NOT, AND I WAS WRONG
///
/// This rung was first written with the handle arm **ungraded**, on the reading that
/// `HostRmBackend::narrow` refuses a foreign [`HostHandle`] before any ioctl is issued, so
/// the arm would measure our own bookkeeping. ⊘ **That reading was false and the source says
/// so in three lines**: `narrow` is `u32::try_from(h.raw())` and **nothing else** — it does
/// not look at the isolate id and it does not consult any table. So `map_local_at` on a
/// handle minted by the *other* client puts A's raw number into
/// `NV_ESC_RM_MAP_MEMORY_DMA` on **B's** file descriptor, and RM answers.
///
/// `[measured 2026-09-06, RTX 3060, 580.159.04]` it answers **`0x57`
/// (`NV_ERR_OBJECT_NOT_FOUND`)**: the handle does not exist *under B's client*, even though
/// it names live memory under A's. ⇒ **RM scopes object handles per client**, and this rung
/// now grades on it. ⚠ Recorded because the mistake is the interesting half: *suspecting* the
/// instrument sent me the wrong way, and the fix was to read `narrow` rather than to reason
/// about it.
///
/// ## ★★★ PRE-REGISTERED, BEFORE THE RUN
///
/// - **both controls land, no leak, ordering holds** ⇒ PASS. Clients are isolated.
/// - **A reads B's magic (or B reads A's)** ⇒ ★★★★★ FAIL, and it is a security result, not a
///   correctness one. Print which direction.
/// - **B's allocator says VA_SHARED is OCCUPIED before B ever mapped it** ⇒ FAIL: one
///   client's mapping is visible in another's address space.
/// - **either client cannot be brought up** ⇒ ⊘ `NOTRUN`. A second client that never opened
///   measured nothing, and must not read as isolation.
fn cross_client_leak(rm: &mut HostRmBackend, probe: W381Probe, gpu: u32) -> bool {
    const RING_A: u64 = 0x0000_0006_D100_0000;
    const RING_B: u64 = 0x0000_0006_F100_0000;
    /// ★ **The same number in both address spaces.** See the rung's docs.
    const VA_SHARED: u64 = 0x0000_001C_1100_0000;
    const MAGIC_A: u32 = 0xC5C0_00A1;
    const MAGIC_B: u32 = 0xC5C0_00B1;
    const MAGIC_A2: u32 = 0xC5C0_00A2;

    println!(
        "info  R5b cross-client    = GPU {gpu}, euid {} — TWO RM clients, each with its own \
         root, its own address space and its own channel, both naming {VA_SHARED:#018x}",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R5b the bar         = neither client's object ever holds the OTHER's magic, \
         and B's allocator does not see A's mapping"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  R5b engine          = COPY0 is not expressible");
        println!("RUNGCTL_cross_client=FAIL");
        println!("RUNG_cross_client=NOTRUN");
        return false;
    };

    // ── CLIENT B — a second connection, from scratch. ───────────────────────────────────
    //
    // ⊘ `DevDir::open` again rather than a clone of A's: the point is a second *client*, and
    // a second `NV01_ROOT` is what `RmConnection::open` mints. Sharing A's directory handle
    // would still be two roots, but it would also be one fewer difference than a real second
    // process has, and this rung stands in for two guest processes.
    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            println!("??    R5b client B        = open(/dev) refused: {e}");
            println!("RUNGCTL_cross_client=FAIL");
            println!("RUNG_cross_client=NOTRUN");
            return false;
        }
    };
    let conn_b = match RmConnection::open(&dev, GpuId(gpu), kayfabe_chips::pinned_host_classes()) {
        Ok(c) => c,
        Err(e) => {
            println!(
                "??    R5b client B        = a SECOND RM client could not be opened: {e}. ⊘ \
                 NOT an isolation result — the experiment never ran"
            );
            println!("RUNGCTL_cross_client=FAIL");
            println!("RUNG_cross_client=NOTRUN");
            return false;
        }
    };
    let client_a_root = rm.host_client();
    let client_b_root = conn_b.client();
    println!("info  R5b hClient A/B     = {client_a_root:#010x} / {client_b_root:#010x}");
    if client_a_root == client_b_root {
        println!(
            "??    R5b SAME ROOT       = the two connections returned the SAME hClient, so \
             they are not two clients and nothing below would be a cross-client statement"
        );
        println!("RUNGCTL_cross_client=FAIL");
        println!("RUNG_cross_client=NOTRUN");
        return false;
    }
    let id_b = IsolateId::new(1, GpuId(gpu));
    let mut rm_b = HostRmBackend::new(
        id_b,
        Arc::new(conn_b),
        Arc::new(kayfabe_isolate_host::ChildExports::new()),
    );

    let (Ok(vas_a), Ok(vas_b)) = (rm.alloc_vaspace(), rm_b.alloc_vaspace()) else {
        println!("FAIL  R5b vaspaces        = the rung needs one address space per client");
        println!("RUNGCTL_cross_client=FAIL");
        println!("RUNG_cross_client=NOTRUN");
        return false;
    };
    let space_b = vas_b.raw() as u32;

    let mut a_chan: Option<kayfabe_isolate::HostHandle> = None;
    let mut b_chan: Option<kayfabe_isolate::HostHandle> = None;
    let mut a_mem: Option<(kayfabe_isolate::HostHandle, u64)> = None;
    let mut b_mem: Option<(kayfabe_isolate::HostHandle, u64)> = None;
    let mut control_a = false;
    let mut control_b = false;
    let mut n1_free = false;
    let mut leak_a_saw_b = false;
    let mut leak_b_saw_a = false;
    let mut ordering_ok = false;
    let mut foreign_refused = false;

    let mut go = || -> bool {
        // ── A comes up FIRST and is written, so B is brought up into a world where A's
        //    mapping already exists. The reverse order could not see a leak caused by the
        //    second mapping, which is `alias_two_vas`'s step-4 lesson applied across clients.
        let Ok((ca, ta)) = rm.alloc_channel_at(vas_a, engine_type, Some(GpuVa(RING_A))) else {
            println!("??    R5b A channel       = refused at {RING_A:#018x} — NOT a result");
            return false;
        };
        a_chan = Some(ca);
        if rm.schedule(ca).is_err() {
            println!("??    R5b A schedule      = refused");
            return false;
        }
        let Ok(ma) = rm.alloc_probe_local(W379_BYTES) else {
            println!("??    R5b A object        = device-local allocation refused");
            return false;
        };
        if rm.fill_words(ma, W379_BYTES, W379_SENTINEL, 0).is_err() {
            println!("??    R5b A sentinel      = could not be written");
            let _ = rm.free(ma);
            return false;
        }
        match rm.map_local_at(vas_a, ma, W379_BYTES, Some(VA_SHARED)) {
            Ok(got) if got == VA_SHARED => a_mem = Some((ma, got)),
            Ok(got) => {
                println!("??    R5b A placement     = asked {VA_SHARED:#018x}, got {got:#018x}");
                a_mem = Some((ma, got));
                return false;
            }
            Err(e) => {
                println!("??    R5b A map           = refused {e:?}");
                let _ = rm.free(ma);
                return false;
            }
        }
        let a1 = w379_release_through(
            rm,
            probe,
            W381Chan { h: ca, token: ta },
            ma,
            VA_SHARED,
            W379_OFF_A,
            MAGIC_A,
        );
        println!("info  R5b control A       = {a1:?}");
        if !a1.landed() {
            println!(
                "??    R5b CONTROL A FAILED = client A could not write its own object. ⊘ \
                 Everything below is UNINTERPRETABLE. Retirement qualifier: {}",
                w381_retired(rm, ca)
            );
            return false;
        }
        control_a = true;

        // ── N1 — B's allocator, BEFORE B maps anything, at the VA A is holding ──────────
        //
        // ★ `probe_va` allocates and fixed-maps a throwaway object to ask the question, so
        // the answer is RM's [OUT] `dmaOffset` and not our argument echoed back. It is
        // already calibrated on this hardware by `dictated_ring_negative`.
        let n1 = rm_b.probe_va(space_b, VA_SHARED, W379_BYTES);
        n1_free = matches!(n1, Ok(kayfabe_isolate_host::rm::VaProbe::Free));
        println!(
            "info  R5b N1 B sees A's VA = {n1:?} at {VA_SHARED:#018x} (A HAS IT MAPPED). \
             Free ⇒ the two address spaces are separate"
        );

        // ── B comes up, at the same VA number, in its own space ────────────────────────
        let Ok((cb, tb)) = rm_b.alloc_channel_at(vas_b, engine_type, Some(GpuVa(RING_B))) else {
            println!("??    R5b B channel       = refused at {RING_B:#018x} — NOT a result");
            return false;
        };
        b_chan = Some(cb);
        if rm_b.schedule(cb).is_err() {
            println!("??    R5b B schedule      = refused");
            return false;
        }
        let Ok(mb) = rm_b.alloc_probe_local(W379_BYTES) else {
            println!("??    R5b B object        = device-local allocation refused");
            return false;
        };
        if rm_b.fill_words(mb, W379_BYTES, W379_SENTINEL, 0).is_err() {
            println!("??    R5b B sentinel      = could not be written");
            let _ = rm_b.free(mb);
            return false;
        }
        match rm_b.map_local_at(vas_b, mb, W379_BYTES, Some(VA_SHARED)) {
            Ok(got) if got == VA_SHARED => b_mem = Some((mb, got)),
            Ok(got) => {
                println!("??    R5b B placement     = asked {VA_SHARED:#018x}, got {got:#018x}");
                b_mem = Some((mb, got));
                return false;
            }
            Err(e) => {
                println!(
                    "??    R5b B map           = refused {e:?}. ⊘ If RM declined the SAME VA \
                     to a second client, that is itself a finding — but it is not the leak \
                     test, which needs both mappings live"
                );
                let _ = rm_b.free(mb);
                return false;
            }
        }
        // ★ IDENTITY, step 4 — printed whether or not they collide.
        println!(
            "info  R5b raw handles     = A object {:#010x}, B object {:#010x}{}",
            ma.raw(),
            mb.raw(),
            if ma.raw() == mb.raw() {
                " ★ IDENTICAL — this run measures \"same handle, same VA, different bytes\""
            } else {
                " (distinct; the VA collision alone carries the rung)"
            }
        );
        let b1 = w379_release_through(
            &mut rm_b,
            probe,
            W381Chan { h: cb, token: tb },
            mb,
            VA_SHARED,
            W379_OFF_A,
            MAGIC_B,
        );
        println!("info  R5b control B       = {b1:?}");
        if !b1.landed() {
            println!(
                "??    R5b CONTROL B FAILED = client B could not write its own object. ⊘ \
                 Everything below is UNINTERPRETABLE. Retirement qualifier: {}",
                w381_retired(&rm_b, cb)
            );
            return false;
        }
        control_b = true;
        println!(
            "ok    R5b controls        = BOTH clients wrote their OWN object through the \
             SAME VA number — so a later red is leakage and not a dead channel"
        );

        // ── LEAK — read both objects back, each through its own independent mapping ─────
        match rm.read_words_independently(ma, W379_BYTES, &[W379_OFF_A]) {
            Ok(w) if w[0] == MAGIC_A => {
                println!("ok    R5b A after B       = A still holds MAGIC_A {MAGIC_A:#010x}")
            }
            Ok(w) if w[0] == MAGIC_B => {
                leak_a_saw_b = true;
                println!(
                    "FAIL  R5b ★★★★★ LEAK A<-B = client A's object holds client B's magic \
                     {MAGIC_B:#010x}. B's write through ITS {VA_SHARED:#018x} reached A's \
                     memory — a value A has no other way to hold"
                );
            }
            Ok(w) => println!(
                "FAIL  R5b A after B       = A holds {:#010x}, neither magic — A's own \
                 mapping died while B was brought up",
                w[0]
            ),
            Err(e) => println!("??    R5b A after B       = read refused {e:?}"),
        }
        match rm_b.read_words_independently(mb, W379_BYTES, &[W379_OFF_A]) {
            Ok(w) if w[0] == MAGIC_B => {
                println!("ok    R5b B after B       = B holds MAGIC_B {MAGIC_B:#010x}")
            }
            Ok(w) if w[0] == MAGIC_A => {
                leak_b_saw_a = true;
                println!(
                    "FAIL  R5b ★★★★★ LEAK B<-A = client B's object holds client A's magic \
                     {MAGIC_A:#010x}"
                );
            }
            Ok(w) => println!(
                "FAIL  R5b B after B       = B holds {:#010x}, neither magic",
                w[0]
            ),
            Err(e) => println!("??    R5b B after B       = read refused {e:?}"),
        }

        // ── ORDERING — A writes again, with both clients live. ──────────────────────────
        let a2 = w379_release_through(
            rm,
            probe,
            W381Chan { h: ca, token: ta },
            ma,
            VA_SHARED,
            W379_OFF_A2,
            MAGIC_A2,
        );
        println!("info  R5b A writes again  = {a2:?}");
        let b_unmoved = matches!(
            rm_b.read_words_independently(mb, W379_BYTES, &[W379_OFF_A2]),
            Ok(w) if w[0] == W379_SENTINEL
        );
        ordering_ok = a2.landed() && b_unmoved;
        if !b_unmoved {
            println!(
                "FAIL  R5b B saw A's 2nd   = offset {W379_OFF_A2:#x} of B's object is no \
                 longer the sentinel after A wrote through ITS OWN {VA_SHARED:#018x}"
            );
        }

        // ★★★ THE HANDLE ARM, AND IT IS GRADED — see the rung's docs for why the first
        // version of this comment was wrong. `narrow` is a `u32::try_from` and nothing else,
        // so A's raw number really does reach RM on B's descriptor.
        let forged = kayfabe_isolate::HostHandle::new(id_b, ma.raw());
        let foreign = rm_b.map_local_at(vas_b, forged, W379_BYTES, None);
        foreign_refused = foreign.is_err();
        println!(
            "{:<5} R5b foreign handle  = client B mapping A's raw object {:#010x} into \
             B's OWN address space: {}",
            if foreign_refused { "ok" } else { "FAIL" },
            ma.raw(),
            match &foreign {
                Ok(va) => format!(
                    "★★★★★ MAPPED at {va:#018x} — RM let client B name an object minted by \
                     client A. A handle is not scoped to its client"
                ),
                Err(e) => format!(
                    "refused {e:?} ⇒ RM scopes object handles PER CLIENT. `Other(87)` is \
                     `NV_ERR_OBJECT_NOT_FOUND` (`0x57`) — the handle names live memory under \
                     A and does not exist under B"
                ),
            }
        );
        if let Ok(va) = foreign {
            let _ = rm_b.unmap_local(vas_b, va);
        }
        true
    };

    let ran = go();

    if let Some((mem, va)) = a_mem {
        let _ = rm.unmap_local(vas_a, va);
        let _ = rm.free(mem);
    }
    if let Some((mem, va)) = b_mem {
        let _ = rm_b.unmap_local(vas_b, va);
        let _ = rm_b.free(mem);
    }
    if let Some(h) = a_chan {
        let _ = rm.free(h);
    }
    if let Some(h) = b_chan {
        let _ = rm_b.free(h);
    }
    let _ = rm.free(vas_a);
    let _ = rm_b.free(vas_b);

    let control_ok = control_a && control_b;
    let clean = ran
        && control_ok
        && n1_free
        && !leak_a_saw_b
        && !leak_b_saw_a
        && ordering_ok
        && foreign_refused;
    println!(
        "info  R5b census          = controls A={control_a} B={control_b}  \
         B-sees-A's-VA-free={n1_free}  leak A<-B={leak_a_saw_b}  leak B<-A={leak_b_saw_a}  \
         ordering={ordering_ok}  foreign handle refused={foreign_refused}"
    );
    if clean {
        println!(
            "★     R5b ISOLATED        = two RM clients drove the SAME GPU VA in their own \
             address spaces, neither ever held the other's magic, and neither could name \
             the other's object by its raw handle"
        );
    }
    println!(
        "RUNGCTL_cross_client={}",
        if control_ok { "PASS" } else { "FAIL" }
    );
    println!(
        "RUNG_cross_client={}",
        if !control_ok {
            "NOTRUN"
        } else if clean {
            "PASS"
        } else {
            "FAIL"
        }
    );
    clean
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ w385 — `--concurrent-fuzz`: THE MULTI-THREADED RAW CLIENT, WITH DELIBERATE FUZZY
// INTERLEAVING.
//
// ## ⊘ THE CLASS THAT WAS UNCOVERED, STATED BEFORE THE CODE
//
// Every rung above this one is **single-threaded and sequential**. `map_stress` (R5) does
// 186 releases and does them **one at a time**; `cross_client_leak` (R5b) holds two RM
// clients and never lets them run **at the same time**. So nothing in this binary — and
// nothing in the suite — has ever reached:
//
//   * `RmConnection`'s two host-side mutexes under real contention. They are `objects`
//     (the handle table and the monotonic `mint()` counter) and `rings` (the CPU mappings
//     of every channel's pushbuffer/GPFIFO/semaphore), and their separation is a **stated
//     rank discipline** — *"Two locks, each held for one kind of thing, is the R3 lock-rank
//     discipline rather than a convenience"* (`rm.rs`, the `rings` field). A discipline no
//     two threads ever tested is a comment.
//   * the address table under **concurrent** map / unmap / probe into ONE address space.
//   * handle and VA **recycling** with another thread allocating into the hole.
//   * ordering assumptions that hold only because nothing else was running.
//
// ## ★★★ SEEDED, OR IT IS NOT AN INSTRUMENT — and the honest half of that claim
//
// Every random choice comes from one `--seed`, printed on every run, and `--seed=N` replays
// it. ⊘ **What replays is the DECISION SEQUENCE, not the SCHEDULE.** Thread `t`'s op at
// iteration `i` is a pure function of `(seed, t, i)`, so a red names a reproducible
// *program*; the OS interleaving that made it red is not ours to reproduce. That is stated
// here rather than discovered later, because *"reproducible fuzz"* over threads is a claim
// this tree cannot make and should not imply. In practice a red replays because the op
// sequence and the jitter pattern are the same; it is a strong tendency, not a guarantee.
//
// ## THE ORACLE — five invariants, each refused BY NAME
//
// A stress test with no invariant only finds crashes. These are checked continuously, and
// every violation carries the seed, the thread, the iteration and the address:
//
//   1. `VA_DOUBLE_BOUND`   — no VA is ever bound to two different memories at once. Every
//                            thread writes a **thread-unique, client-tagged magic**
//                            ([`w385_magic`]), so a word that arrives from somewhere else
//                            is *identifiable* rather than merely wrong.
//   2. `MAP_NOT_ATOMIC`    — a mapping is all-or-nothing. A VA that `probe_va` calls `Free`
//                            immediately after a map RM said it accepted is a mapping that
//                            was observable half-installed.
//   3. `ALIAS_MISMATCH` / `ALIAS_REVOKED` — every value written through one alias is
//                            readable through the others (w380's property), and mapping or
//                            unmapping the second alias does not silently revoke the first.
//   4. `CROSS_CLIENT_LEAK` — no client ever observes another client's magic. The two
//                            clients deliberately name **the same VA numbers**, so this is
//                            an isolation statement and not an accident of layout.
//   5. `FREED_HANDLE_RESOLVES` / `VA_NOT_RECOVERED` / `STALE_READ` — every release is
//                            eventually observed; a freed handle is not resolvable
//                            afterwards; a VA comes back Free; a recycled object reads as
//                            its OWN sentinel and never as its predecessor's magic.
//
// ## ★★ A DEADLOCK MUST FAIL BY NAME, NOT HANG
//
// This tree has wedged CI on nontermination more than once, and a hang once held three
// binaries for 23 hours. There is a watchdog on the **whole rung** and a stall check on
// **each worker**; on expiry it prints every worker's last op, prints
// `RUNG_concurrent_fuzz=FAIL` with `FUZZ_REASON=DEADLOCK/WATCHDOG`, and exits non-zero.
// ⊘ It exits via `std::process::exit`, so RM objects are NOT torn down — a watchdog kill
// leaks the run's channels and that is the correct trade: the alternative is the hang.
//
// ## CONTROLS — the rung is these, not the loop
//
//   * **The positive control runs FIRST**: the same operations at `T=1` with **no jitter**.
//     If that fails the rung is broken, not the system, and everything after it prints
//     `NOTRUN`.
//   * **`FUZZ_OVERLAP_PAIRS` is a second control, and it can veto a green.** It counts pairs
//     of RM-verb intervals from *different threads* that actually intersected. A fuzz run
//     that sampled **zero** overlap sampled no concurrency at all, and grading it PASS would
//     be reporting a finding never measured — so zero overlap prints `NOTRUN`.
//   * ⚠ **A green proves less than it looks.** The run states `FUZZ_OPS` and
//     `FUZZ_OVERLAP_PAIRS` so the budget it exhausted is a number. Absence of a red is not
//     absence of a race.
//
// ⊘ **`GP_GET` is never read here** — it has no writer anywhere in this workspace, so on an
// emulated device it is a constant. ⊘ **`pde_info` is not used as a publication oracle**: it
// answers at page-*table* granularity. The graded observables are `probe_va` and the
// engine read-back through an independent CPU mapping, exactly as w381 established.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// Bytes of every object the fuzz allocates — the same `FB_LEAF_GRANULE` the w379/w381
/// rungs use, so a placement result here is comparable with theirs. See [`W379_BYTES`].
const W385_BYTES: u64 = W379_BYTES;

/// What every fresh object holds before any engine touches it. Distinct from
/// [`W379_SENTINEL`] so a stale word can be attributed to the rung that wrote it, and its
/// top byte is deliberately **not** [`W385_MAGIC_TAG`] so [`w385_owner`] cannot claim it.
const W385_SENTINEL: u32 = 0xDEAD_0385;

/// The top byte that marks a word as *"a fuzz worker wrote this"*. Everything below it is
/// the writer's identity, which is what makes a cross-thread read **identifiable** rather
/// than merely unexpected.
const W385_MAGIC_TAG: u32 = 0xF5;

/// Where the fuzz's address windows start. Far above every other rung's constants so a
/// concurrent run cannot collide with one of them.
const W385_VA_BASE: u64 = 0x0000_0020_0000_0000;

/// One worker's private window: 32 GiB, which is 8 slots' worth of the stride below and
/// therefore holds this rung's ring plus `2 * W385_SLOTS` mappable VAs with room to spare.
///
/// ⊘⊘ **IT WAS 64 GiB AND THAT WAS A DEFECT.** `alloc_vaspace` asks for `vaSize = 0`, which
/// takes RM's default `FERMI_VASPACE_A` limit — **1 TiB** on this part. At 64 GiB per lane,
/// lane 14's window begins at `0x100_0000_0000`, i.e. exactly one byte-range past the end of
/// the address space, so every mapping in it was refused. Measured 2026-09-06 at 32 threads:
/// 22 fabricated `ALIAS_REVOKED`s per phase, all of them from tids 28/29 and all of them at
/// VAs in that one window. ⇒ Halving the stride buys twice the lanes inside the same limit,
/// and [`W385_MAX_LANE`] refuses the rest **by name** instead of letting them look like a
/// finding.
const W385_WORKER_STRIDE: u64 = 0x0000_0008_0000_0000;

/// Distance between two mappable VAs. 4 GiB, for the reason `alias_two_vas` names: an
/// adjacent VA could be covered by a big PTE a neighbour's mapping already installed, and
/// the rung would then pass for a reason that has nothing to do with what it asked.
const W385_SLOT_STRIDE: u64 = 0x0000_0001_0000_0000;

/// The three word offsets inside an object at which magics land. Distinct, so a write
/// through one alias cannot be mistaken for a write through another.
const W385_OFFS: [u64; 3] = [0x0000, 0x0040, 0x0080];

/// ★★★★★ **HOW A PHASE PLACES ITS WORKERS ON CORES — and why a stress rung has an opinion
/// about that at all.**
///
/// Owner ruling, 2026-09-06: *"pinning to vcpu cores is very useful to test concurrency in
/// kayfabe as each vcpu is a host thread."* In kayfabe **each guest vCPU IS a host thread**,
/// so the concurrency the product must survive is *those* threads contending: a small, fixed
/// set, preempting each other. A fuzz whose threads land wherever the scheduler puts them is
/// sampling a **different topology** — on a 19-core box they spread across idle cores, run
/// past one another, and essentially never interleave **inside** a critical section.
///
/// ⊘ That is exactly where a lock-order or ranked-lock bug hides, so an unpinned run can be
/// green for the whole of it. ⚠ Pinning here **removes** parallelism on purpose; it is not a
/// performance knob.
///
/// ★★★ The three modes exist as a **paired differential**, never as a replacement:
/// `Unpinned` is kept as the control at the SAME width as each pinned arm, so *"pinning
/// changed the answer"* is itself a measurement rather than an assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum W385Pin {
    /// ⊘ THE CONTROL. Threads land wherever the scheduler puts them — what every previous
    /// version of this rung did, and what an ordinary `cargo test` does.
    Unpinned,
    /// ★ ONE WORKER PER CORE over the first `cores` cores — the guest's own topology, with
    /// `cores` defaulting to the bench guest's `-smp`.
    PerCore {
        /// How many cores the arm spreads over.
        cores: usize,
    },
    /// ★★★ **OVER-SUBSCRIBED**: more workers than cores, every one of them admitted to the
    /// same `cores`-wide set. This is the arm that forces **preemption inside a held lock**,
    /// which un-pinned parallelism tends never to produce.
    Crowd {
        /// The width of the shared set every worker is confined to.
        cores: usize,
    },
}

impl W385Pin {
    /// The cores worker `tid` may run on, or `None` for the unpinned control.
    fn cores_for(self, tid: usize) -> Option<Vec<usize>> {
        match self {
            W385Pin::Unpinned => None,
            W385Pin::PerCore { cores } => Some(vec![tid % cores.max(1)]),
            W385Pin::Crowd { cores } => Some((0..cores.max(1)).collect()),
        }
    }

    /// The printed form, so a log line, a grader and a human agree on the vocabulary.
    fn as_str(self) -> &'static str {
        match self {
            W385Pin::Unpinned => "unpinned",
            W385Pin::PerCore { .. } => "percore",
            W385Pin::Crowd { .. } => "crowd",
        }
    }
}

/// Live objects one worker juggles. Three, so allocate/map/free interleave rather than
/// nest, and so a `Free` always has a live neighbour whose mapping it might disturb.
const W385_SLOTS: usize = 3;

/// RM's default `FERMI_VASPACE_A` limit on this part — the ceiling every window must sit
/// under. ⊘ A literal, and deliberately not derived from the geometry it bounds: a threshold
/// computed from the thing it checks moves silently when that thing moves.
const W385_VAS_LIMIT: u64 = 0x0000_0100_0000_0000;

/// The highest lane whose whole window — ring, and `2 * W385_SLOTS` mappable VAs — fits under
/// [`W385_VAS_LIMIT`]. ★ A worker above this is refused before it runs, by name.
const W385_MAX_LANE: usize = (((W385_VAS_LIMIT - W385_VA_BASE)
    - (2 * W385_SLOTS as u64 + 1) * W385_SLOT_STRIDE)
    / W385_WORKER_STRIDE) as usize;

/// How many violation details are printed. ⊘ The **counts** are never capped — a capped
/// list beside an uncapped census is fine; a capped census is not a census.
const W385_SHOW: usize = 12;

/// A thread-unique, client-tagged word.
///
/// ★ The identity is IN the value. A read-back that returns someone else's magic tells you
/// **whose** it is, which is the difference between *"invariant 1 fired"* and *"invariant 1
/// fired, client 1 thread 3 wrote it, here is the seed"*.
fn w385_magic(client: u32, tid: u32, seq: u32) -> u32 {
    (W385_MAGIC_TAG << 24) | ((client & 0xF) << 20) | ((tid & 0xFF) << 12) | (seq & 0xFFF)
}

/// Who wrote a word, or `None` if no fuzz worker did.
fn w385_owner(word: u32) -> Option<(u32, u32)> {
    if (word >> 24) == W385_MAGIC_TAG {
        Some(((word >> 20) & 0xF, (word >> 12) & 0xFF))
    } else {
        None
    }
}

/// SplitMix64 — the seeded stream behind every choice this rung makes.
///
/// ⊘ Deliberately not a library RNG and deliberately not `SystemTime`-driven per draw: a
/// `Math.random`-shaped jitter makes a red un-actionable, and this tree has burned days on
/// non-reproducible failures. Every worker derives its own stream from the ONE printed seed.
struct W385Rng(u64);

impl W385Rng {
    fn new(seed: u64) -> Self {
        W385Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n`. `n == 0` answers `0` rather than dividing by zero.
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

/// The verbs a worker draws from — the ones the existing rungs already do, nothing invented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum W385Op {
    /// Allocate a device-local object into a free slot and fill it with the sentinel.
    Alloc,
    /// Map a slot's object at its **first** VA.
    MapA,
    /// Map the SAME object at a **second** VA — the w380 alias case.
    MapB,
    /// Drive the engine to write this worker's magic at a live VA, and read it back.
    Write,
    /// Write through A, then through B, then through A **again** — the revoke detector.
    AliasProp,
    /// Unmap one alias and assert the other survived.
    UnmapB,
    /// Ask RM's allocator what it thinks of a VA we believe is mapped.
    Probe,
    /// Unmap everything, free the object, and assert the handle and the VA both went away.
    Free,
    /// Free **and immediately re-allocate** into the same slot, to force handle/VA reuse.
    Recycle,
    /// The slot state did not admit the drawn verb. Counted, never hidden.
    Idle,
}

impl W385Op {
    fn as_str(self) -> &'static str {
        match self {
            W385Op::Alloc => "alloc",
            W385Op::MapA => "map_a",
            W385Op::MapB => "map_b",
            W385Op::Write => "write",
            W385Op::AliasProp => "alias_prop",
            W385Op::UnmapB => "unmap_b",
            W385Op::Probe => "probe",
            W385Op::Free => "free",
            W385Op::Recycle => "recycle",
            W385Op::Idle => "idle",
        }
    }

    /// The draw. Weighted so the plane under test — mapping and writing — dominates, and so
    /// free/recycle happen often enough that another thread is usually mid-allocation while
    /// one is mid-teardown.
    fn draw(rng: &mut W385Rng) -> W385Op {
        // ⊘ The weights were REBALANCED after the first measured run, and the reason is on
        // the record: at the original 16-entry table `alias_prop` drew **zero** times over
        // 360 ops, so invariant 3 — the w380 alias property, the single most valuable thing
        // this rung can check — was never exercised in an arm that reported PASS. A verb's
        // weight is part of the oracle, not a taste.
        const TABLE: [W385Op; 20] = [
            W385Op::Alloc,
            W385Op::Alloc,
            W385Op::Alloc,
            W385Op::MapA,
            W385Op::MapA,
            W385Op::MapA,
            W385Op::MapB,
            W385Op::MapB,
            W385Op::MapB,
            W385Op::Write,
            W385Op::Write,
            W385Op::Write,
            W385Op::Write,
            W385Op::AliasProp,
            W385Op::AliasProp,
            W385Op::AliasProp,
            W385Op::UnmapB,
            W385Op::Probe,
            W385Op::Free,
            W385Op::Recycle,
        ];
        TABLE[(rng.below(TABLE.len() as u64)) as usize]
    }

    /// The breadcrumb code the watchdog dumps. `u32` because it lives in an `AtomicU32`
    /// that a wedged worker leaves behind.
    fn code(self) -> u32 {
        match self {
            W385Op::Alloc => 1,
            W385Op::MapA => 2,
            W385Op::MapB => 3,
            W385Op::Write => 4,
            W385Op::AliasProp => 5,
            W385Op::UnmapB => 6,
            W385Op::Probe => 7,
            W385Op::Free => 8,
            W385Op::Recycle => 9,
            W385Op::Idle => 10,
        }
    }

    fn from_code(c: u32) -> &'static str {
        match c {
            0 => "(not started)",
            1 => "alloc",
            2 => "map_a",
            3 => "map_b",
            4 => "write",
            5 => "alias_prop",
            6 => "unmap_b",
            7 => "probe",
            8 => "free",
            9 => "recycle",
            10 => "idle",
            _ => "(unknown)",
        }
    }

    /// ★★★ Does slot `sl` admit this verb?
    ///
    /// ⊘ **This exists because the first version of the rung was nearly vacuous and said so
    /// in its own census.** It drew a verb and a slot INDEPENDENTLY, so most draws landed on
    /// a slot in the wrong state: measured at `I=12`, `idle=85` of 108 ops, `write=0`,
    /// `alias_prop=0`, `map_b=0` — three arms went green having **never run the engine at
    /// all**. A stress rung whose ops mostly do nothing is a timer, not a test.
    ///
    /// ⇒ Draw the VERB from the seeded stream, then choose uniformly among the slots that
    /// can actually take it. The draw stays a pure function of the stream and the state, and
    /// `Idle` now means *"no slot could take this verb"* rather than *"I picked badly"*.
    fn admits(self, sl: Option<&W385Slot>) -> bool {
        match (self, sl) {
            (W385Op::Alloc, None) => true,
            (W385Op::MapA, Some(s)) => s.va_a.is_none(),
            (W385Op::MapB, Some(s)) => s.va_a.is_some() && s.va_b.is_none(),
            (W385Op::Write, Some(s)) => s.va_a.is_some() || s.va_b.is_some(),
            (W385Op::AliasProp, Some(s)) => s.va_a.is_some() && s.va_b.is_some(),
            (W385Op::UnmapB, Some(s)) => s.va_b.is_some(),
            (W385Op::Probe, Some(s)) => s.va_a.is_some(),
            (W385Op::Free | W385Op::Recycle, Some(_)) => true,
            _ => false,
        }
    }

    /// The verbs that make the **engine** run and are therefore what invariants 1, 3 and 4
    /// are actually tested by. ★ An arm that drew none of them sampled nothing, however
    /// many allocations it did — see the coverage veto in [`concurrent_fuzz`].
    const ENGINE: [W385Op; 3] = [W385Op::Write, W385Op::AliasProp, W385Op::UnmapB];

    /// ★★★ The one verb that tests **invariant 3 end to end** — write through A, through B,
    /// then through A again with B still mapped. Counted separately in the arm line because
    /// an arm that never drew it is green about the alias property specifically, and the
    /// aggregate `engine_ops` would hide that.
    const ALIAS: W385Op = W385Op::AliasProp;

    /// Every verb this rung can draw, for the per-op census.
    const ALL: [W385Op; 10] = [
        W385Op::Alloc,
        W385Op::MapA,
        W385Op::MapB,
        W385Op::Write,
        W385Op::AliasProp,
        W385Op::UnmapB,
        W385Op::Probe,
        W385Op::Free,
        W385Op::Recycle,
        W385Op::Idle,
    ];
}

/// What every worker publishes so a watchdog can say **what it was doing** rather than
/// merely that it stopped.
///
/// ⊘ Atomics rather than a mutex, deliberately: the watchdog must be readable while a worker
/// is wedged, and a mutex the wedged worker holds is exactly the thing it cannot be.
struct W385Beat {
    /// Milliseconds since the rung's origin at which worker `i` last finished an op.
    last_ms: Vec<std::sync::atomic::AtomicU64>,
    /// [`W385Op::code`] of the verb worker `i` is inside.
    op: Vec<std::sync::atomic::AtomicU32>,
    /// Which iteration worker `i` is on.
    iter: Vec<std::sync::atomic::AtomicU64>,
    /// Whether worker `i` finished its loop.
    done: Vec<std::sync::atomic::AtomicBool>,
    /// ★★ Set by the phase once every worker has been **joined**.
    ///
    /// ⊘ Not redundant with `done`, and the difference is a bug this nearly shipped: a
    /// worker that returns EARLY (no channel) or **panics** never sets its own `done`, so a
    /// watchdog keyed on `done` alone would keep counting and eventually kill a process
    /// whose workers had all been collected. `done` is the worker's statement; `stop` is
    /// the phase's.
    stop: std::sync::atomic::AtomicBool,
}

impl W385Beat {
    fn new(n: usize) -> Self {
        W385Beat {
            last_ms: (0..n)
                .map(|_| std::sync::atomic::AtomicU64::new(0))
                .collect(),
            op: (0..n)
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect(),
            iter: (0..n)
                .map(|_| std::sync::atomic::AtomicU64::new(0))
                .collect(),
            done: (0..n)
                .map(|_| std::sync::atomic::AtomicBool::new(false))
                .collect(),
            stop: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn note(&self, t: usize, op: W385Op, iter: u64, ms: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        self.op[t].store(op.code(), Relaxed);
        self.iter[t].store(iter, Relaxed);
        self.last_ms[t].store(ms, Relaxed);
    }

    /// The dump. Printed by the watchdog, and also on a clean finish when anything is odd.
    fn dump(&self, now_ms: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        for t in 0..self.op.len() {
            println!(
                "        worker {t:2}  last op = {:<12} iter {:<6} last beat {} ms ago  \
                 done={}",
                W385Op::from_code(self.op[t].load(Relaxed)),
                self.iter[t].load(Relaxed),
                now_ms.saturating_sub(self.last_ms[t].load(Relaxed)),
                self.done[t].load(Relaxed)
            );
        }
    }
}

/// One worker's tunables and identity, as ONE value — clippy's argument bound is the
/// occasion, but the reason is that a worker mis-paired with another worker's lane would
/// write into a window it does not own and manufacture invariant 1.
#[derive(Debug, Clone, Copy)]
struct W385Worker {
    /// Global worker index — the `tid` inside every magic.
    tid: usize,
    /// Which RM client this worker belongs to.
    client: usize,
    /// Index **within the client**. ★ The VA window is keyed on this and NOT on `tid`, so
    /// the two clients deliberately name **the same VA numbers** — which is what makes
    /// invariant 4 an isolation statement instead of an accident of layout.
    lane: usize,
    /// Iterations to run.
    iters: u64,
    /// Upper bound on one jitter sleep, in microseconds. `0` = the positive control.
    jitter_us: u64,
    /// This worker's PRNG seed, derived from the run's one printed seed.
    seed: u64,
    /// How this worker is placed on cores. See [`W385Pin`].
    pin: W385Pin,
}

impl W385Worker {
    /// The base of this worker's private VA window.
    fn window(&self) -> u64 {
        W385_VA_BASE + (self.lane as u64) * W385_WORKER_STRIDE
    }

    /// Where this worker's channel ring lives.
    fn ring_at(&self) -> u64 {
        self.window()
    }

    /// The two VAs of slot `s`. `A` at odd multiples of the stride, `B` at even, so no
    /// slot's alias can be covered by a neighbour's big PTE.
    fn slot_vas(&self, s: usize) -> (u64, u64) {
        let base = self.window();
        (
            base + (1 + 2 * s as u64) * W385_SLOT_STRIDE,
            base + (2 + 2 * s as u64) * W385_SLOT_STRIDE,
        )
    }

    /// Is `va` inside this worker's own window? A mapping that lands outside it is a
    /// `WINDOW_ESCAPE` and is graded, because it is how one worker comes to scribble on
    /// another's invariant.
    fn owns(&self, va: u64) -> bool {
        let b = self.window();
        va >= b && va < b + W385_WORKER_STRIDE
    }
}

/// One live object a worker is juggling.
struct W385Slot {
    mem: kayfabe_isolate::HostHandle,
    va_a: Option<u64>,
    va_b: Option<u64>,
}

/// What one worker brings back.
#[derive(Default)]
struct W385Report {
    /// `(invariant name, the detail line)`. The names are the vocabulary; the details are
    /// what makes a red actionable.
    violations: Vec<(&'static str, String)>,
    /// Per-verb attempt counts, indexed by [`W385Op::code`].
    ops: [u64; 11],
    /// Every RM-verb interval this worker occupied, `(tid, start_ns, end_ns)`.
    spans: Vec<(usize, u128, u128)>,
    /// Verbs whose *submission* RM refused. ⊘ Not a violation: a refusal is the system
    /// saying no, and folding it into a red would report our own ask as the system's fault.
    refused: u64,
    /// Whether the worker reached the end of its loop.
    finished: bool,
    /// ★ Whether this worker's affinity call was ACCEPTED. ⊘ Not whether it was *asked for*:
    /// a default you never see exercised is a default you do not have, and a pinned arm that
    /// silently ran unpinned would be reported as *"pinning changed nothing"*.
    pinned: bool,
    /// The cores the kernel says this worker may run on, read back AFTER the call.
    observed_cores: Vec<usize>,
    /// ★★★ Every raw RM handle this worker's allocations came back with.
    ///
    /// `RmConnection::mint` is `let h = o.next; o.next += 1` under the `objects` mutex, and
    /// two workers minting at once is the most direct thing this rung can aim at that lock.
    /// ⊘ The uniqueness is checked by the PHASE, across workers — a per-worker check could
    /// never see the collision, which is exactly the shape of the defect.
    minted: Vec<u64>,
}

impl W385Report {
    fn violate(&mut self, name: &'static str, detail: String) {
        self.violations.push((name, detail));
    }
}

/// ★★★ ONE WORKER. Everything above is scaffolding; this is the loop.
///
/// It builds its **own** [`HostRmBackend`] from the client's shared `Arc<RmConnection>`,
/// which is the point: `HostRmBackend`'s mutating verbs take `&mut self` and its `slots`
/// map is per-worker, so a per-thread backend is the only shape that is not itself a bug —
/// while the `Arc<RmConnection>` underneath, with its `objects` and `rings` mutexes, is
/// **shared**, and is the host-side lock this rung exists to hammer.
#[allow(clippy::too_many_lines)]
fn w385_run_worker(
    w: W385Worker,
    conn: std::sync::Arc<RmConnection>,
    vas_raw: u64,
    engine_type: u32,
    probe: W381Probe,
    origin: std::time::Instant,
    beat: std::sync::Arc<W385Beat>,
) -> W385Report {
    let mut rep = W385Report::default();
    // ⊘ THE GEOMETRY CHECK, BEFORE ANYTHING ELSE. A window past the end of the address space
    // does not produce a subtle result — it produces a CONFIDENT one, because every mapping
    // in it is refused and every downstream probe then reads as a revoke. Refuse it by its
    // own name so the run says *"the rung asked for more address space than exists"* instead
    // of *"the alias property broke"*.
    if w.lane > W385_MAX_LANE {
        rep.violate(
            "WINDOW_ABOVE_VAS_LIMIT",
            format!(
                "tid {} lane {} would place a window at {:#018x}, past the address space's \
                 {:#018x} limit. ⊘ HARNESS FAULT, not a system result: raise --fuzz-clients \
                 or lower --fuzz-threads (max {} lanes per client)",
                w.tid,
                w.lane,
                w.window(),
                W385_VAS_LIMIT,
                W385_MAX_LANE + 1
            ),
        );
        beat.done[w.tid].store(true, std::sync::atomic::Ordering::Relaxed);
        return rep;
    }
    // ★★★ PIN FIRST, BEFORE THE CHANNEL AND BEFORE THE FIRST VERB. Everything this worker
    // does — including the RM ioctls whose locks are the point — must happen on the cores the
    // arm claims, or the arm is measuring the previous placement.
    // ⊘ The placement is READ BACK rather than assumed: `pin_current_thread` returning true
    // is the kernel accepting the mask, and `current_cores` is the kernel describing what it
    // then applied. A rung that printed the mask it ASKED for could report a pinned arm that
    // ran unpinned.
    if let Some(cores) = w.pin.cores_for(w.tid) {
        rep.pinned = kayfabe_linux_raw::affinity::pin_current_thread(&cores);
    }
    rep.observed_cores = kayfabe_linux_raw::affinity::current_cores().unwrap_or_default();
    let id = IsolateId::new(w.client as u32, GpuId(0));
    let vas = kayfabe_isolate::HostHandle::new(id, vas_raw);
    let space = vas_raw as u32;
    let mut rm = HostRmBackend::new(
        id,
        conn,
        std::sync::Arc::new(kayfabe_isolate_host::ChildExports::new()),
    );
    let mut rng = W385Rng::new(w.seed);

    // The channel is this worker's alone: `submit_*` takes the next GPFIFO slot out of the
    // backend's OWN `slots` map, so two workers sharing one channel would overwrite each
    // other's entries and produce a red that is the harness's, not the system's.
    let chan_res = rm.alloc_channel_at(vas, engine_type, Some(GpuVa(w.ring_at())));
    if let Ok((c, _)) = &chan_res {
        rep.minted.push(c.raw());
    }
    let Ok((chan, token)) = chan_res else {
        rep.violate(
            "WORKER_NO_CHANNEL",
            format!(
                "tid {} could not place a channel ring at {:#018x}",
                w.tid,
                w.ring_at()
            ),
        );
        beat.done[w.tid].store(true, std::sync::atomic::Ordering::Relaxed);
        return rep;
    };
    if rm.schedule(chan).is_err() {
        rep.violate(
            "WORKER_NO_SCHEDULE",
            format!("tid {} channel would not schedule", w.tid),
        );
        let _ = rm.free(chan);
        beat.done[w.tid].store(true, std::sync::atomic::Ordering::Relaxed);
        return rep;
    }

    let mut slots: Vec<Option<W385Slot>> = (0..W385_SLOTS).map(|_| None).collect();
    let mut seq: u32 = 0;
    let ch = W381Chan { h: chan, token };

    // ── the jitter, seeded. Both a sleep and a spin, chosen by the same stream: a pure
    //    sleep parks the thread and stops contending, and a race that only shows up while
    //    two threads are actually ON CPU together would never be sampled by one.
    let jitter = |rng: &mut W385Rng| {
        if w.jitter_us == 0 {
            return;
        }
        let d = rng.below(w.jitter_us);
        if rng.below(2) == 0 {
            std::thread::sleep(std::time::Duration::from_micros(d));
        } else {
            let until = std::time::Instant::now() + std::time::Duration::from_micros(d);
            while std::time::Instant::now() < until {
                std::hint::spin_loop();
            }
        }
    };

    for it in 0..w.iters {
        jitter(&mut rng);
        let op = W385Op::draw(&mut rng);
        // ★ Choose among the slots that ADMIT the drawn verb — see [`W385Op::admits`] for
        // the measurement that made this necessary.
        let cand: Vec<usize> = (0..W385_SLOTS)
            .filter(|&i| op.admits(slots[i].as_ref()))
            .collect();
        let (op, s) = if cand.is_empty() {
            (W385Op::Idle, 0)
        } else {
            (op, cand[rng.below(cand.len() as u64) as usize])
        };
        let start = origin.elapsed().as_nanos();
        beat.note(w.tid, op, it, origin.elapsed().as_millis() as u64);

        let mut did = op;
        match op {
            // ── ALLOCATE ────────────────────────────────────────────────────────────────
            W385Op::Alloc => {
                if slots[s].is_some() {
                    did = W385Op::Idle;
                } else {
                    match rm.alloc_probe_local(W385_BYTES) {
                        Ok(mem) => {
                            rep.minted.push(mem.raw());
                            if rm.fill_words(mem, W385_BYTES, W385_SENTINEL, 0).is_err() {
                                let _ = rm.free(mem);
                                rep.refused += 1;
                            } else {
                                // ★ INVARIANT 5, the recycling half: a FRESH object must
                                // read as its own sentinel. If it holds a magic, RM handed
                                // back storage somebody is still writing to.
                                w385_check_fresh(&rm, &w, &mut rep, mem, it);
                                slots[s] = Some(W385Slot {
                                    mem,
                                    va_a: None,
                                    va_b: None,
                                });
                            }
                        }
                        Err(_) => rep.refused += 1,
                    }
                }
            }

            // ── MAP, first VA ───────────────────────────────────────────────────────────
            W385Op::MapA => {
                let (va_a, _) = w.slot_vas(s);
                match slots[s].as_mut() {
                    Some(sl) if sl.va_a.is_none() => {
                        let mem = sl.mem;
                        match rm.map_local_at(vas, mem, W385_BYTES, Some(va_a)) {
                            Ok(got) => {
                                w385_check_placement(&w, &mut rep, va_a, got, it);
                                sl.va_a = Some(got);
                                jitter(&mut rng);
                                // ★ INVARIANT 2 — all-or-nothing. RM said it mapped; its
                                // own allocator must not still call the VA free.
                                w385_check_installed(&mut rm, &w, &mut rep, space, got, it);
                            }
                            Err(_) => rep.refused += 1,
                        }
                    }
                    _ => did = W385Op::Idle,
                }
            }

            // ── MAP, second VA over the SAME memory — the w380 alias case ───────────────
            W385Op::MapB => {
                let (_, va_b) = w.slot_vas(s);
                match slots[s].as_mut() {
                    Some(sl) if sl.va_a.is_some() && sl.va_b.is_none() => {
                        let mem = sl.mem;
                        match rm.map_local_at(vas, mem, W385_BYTES, Some(va_b)) {
                            Ok(got) => {
                                w385_check_placement(&w, &mut rep, va_b, got, it);
                                sl.va_b = Some(got);
                            }
                            Err(_) => rep.refused += 1,
                        }
                    }
                    _ => did = W385Op::Idle,
                }
            }

            // ── WRITE one magic through one live VA, and read it back ───────────────────
            W385Op::Write => {
                let live: Vec<u64> = slots[s]
                    .as_ref()
                    .map(|sl| sl.va_a.iter().chain(sl.va_b.iter()).copied().collect())
                    .unwrap_or_default();
                if live.is_empty() {
                    did = W385Op::Idle;
                } else {
                    let va = live[rng.below(live.len() as u64) as usize];
                    let off = W385_OFFS[rng.below(W385_OFFS.len() as u64) as usize];
                    seq = seq.wrapping_add(1);
                    let magic = w385_magic(w.client as u32, w.tid as u32, seq);
                    let mem = slots[s].as_ref().expect("slot").mem;
                    jitter(&mut rng);
                    let out = w379_release_through(&mut rm, probe, ch, mem, va, off, magic);
                    w385_grade_write(&w, &mut rep, out, magic, va, off, it);
                }
            }

            // ── THE ALIAS PROPERTY, in one op: A, then B, then A AGAIN ──────────────────
            W385Op::AliasProp => match slots[s].as_ref().map(|sl| (sl.mem, sl.va_a, sl.va_b)) {
                Some((mem, Some(va_a), Some(va_b))) => {
                    seq = seq.wrapping_add(1);
                    let m1 = w385_magic(w.client as u32, w.tid as u32, seq);
                    seq = seq.wrapping_add(1);
                    let m2 = w385_magic(w.client as u32, w.tid as u32, seq);
                    seq = seq.wrapping_add(1);
                    let m3 = w385_magic(w.client as u32, w.tid as u32, seq);

                    let a1 = w379_release_through(&mut rm, probe, ch, mem, va_a, W385_OFFS[0], m1);
                    w385_grade_write(&w, &mut rep, a1, m1, va_a, W385_OFFS[0], it);
                    jitter(&mut rng);
                    let b1 = w379_release_through(&mut rm, probe, ch, mem, va_b, W385_OFFS[1], m2);
                    w385_grade_write(&w, &mut rep, b1, m2, va_b, W385_OFFS[1], it);
                    jitter(&mut rng);
                    // ★★★ INVARIANT 3 — the whole op. A landed before B existed; it must
                    // still land now that B does. A `Lost` here IS the revoke.
                    let a2 = w379_release_through(&mut rm, probe, ch, mem, va_a, W385_OFFS[2], m3);
                    // ⊘⊘ **`Lost`, NEVER `!landed()`.** `W379Release::Refused` means the
                    // submission was declined and the engine was never asked — folding it in
                    // here reports OUR OWN ask as the system's red. Measured 2026-09-06:
                    // before this line said `Lost`, an arm whose VA windows ran past the
                    // address space's 1 TiB limit produced **22 fabricated ALIAS_REVOKEDs**
                    // per phase, and the only thing that distinguished them from a real
                    // revoke was the `(Refused)` printed in their own message.
                    if a1.landed() && b1.landed() && matches!(a2, W379Release::Lost { .. }) {
                        rep.violate(
                            "ALIAS_REVOKED",
                            format!(
                                "tid {} it {it}: VA_A {va_a:#018x} landed before VA_B \
                                 {va_b:#018x} was mapped and is SILENT after ({a2:?})",
                                w.tid
                            ),
                        );
                    } else {
                        w385_grade_write(&w, &mut rep, a2, m3, va_a, W385_OFFS[2], it);
                    }
                    // ★ INVARIANT 3, the other half: what B wrote must be readable through
                    // the ONE object both VAs name.
                    if b1.landed()
                        && let Ok(words) =
                            rm.read_words_independently(mem, W385_BYTES, &[W385_OFFS[1]])
                        && words[0] != m2
                    {
                        rep.violate(
                            "ALIAS_MISMATCH",
                            format!(
                                "tid {} it {it}: wrote {m2:#010x} through VA_B \
                                 {va_b:#018x}, the object reads {:#010x}",
                                w.tid, words[0]
                            ),
                        );
                    }
                }
                _ => did = W385Op::Idle,
            },

            // ── UNMAP the alias, and assert the survivor survived ───────────────────────
            W385Op::UnmapB => match slots[s].as_ref().map(|sl| (sl.mem, sl.va_a, sl.va_b)) {
                Some((mem, va_a, Some(va_b))) => {
                    let un = rm.unmap_local(vas, va_b).is_ok();
                    if let Some(sl) = slots[s].as_mut() {
                        sl.va_b = None;
                    }
                    jitter(&mut rng);
                    if un {
                        // ★ INVARIANT 5 — the unmap must become observable.
                        match rm.probe_va(space, va_b, W385_BYTES) {
                            Ok(kayfabe_isolate_host::rm::VaProbe::Free) => {}
                            other => rep.violate(
                                "VA_NOT_RECOVERED",
                                format!(
                                    "tid {} it {it}: {va_b:#018x} did not come back after \
                                     an unmap RM accepted (probe={other:?})",
                                    w.tid
                                ),
                            ),
                        }
                    } else {
                        rep.refused += 1;
                    }
                    // ★★★ INVARIANT 3 — unmapping one alias must not take the other down.
                    if let Some(va_a) = va_a {
                        seq = seq.wrapping_add(1);
                        let m = w385_magic(w.client as u32, w.tid as u32, seq);
                        let out =
                            w379_release_through(&mut rm, probe, ch, mem, va_a, W385_OFFS[0], m);
                        // ⊘ `Lost`, never `!landed()` — see the same note in `AliasProp`.
                        if matches!(out, W379Release::Lost { .. }) {
                            rep.violate(
                                "ALIAS_REVOKED",
                                format!(
                                    "tid {} it {it}: unmapping the alias {va_b:#018x} \
                                     silenced the survivor {va_a:#018x} ({out:?})",
                                    w.tid
                                ),
                            );
                        } else {
                            w385_grade_write(&w, &mut rep, out, m, va_a, W385_OFFS[0], it);
                        }
                    }
                }
                _ => did = W385Op::Idle,
            },

            // ── ASK THE ALLOCATOR about a VA we believe is live ─────────────────────────
            W385Op::Probe => match slots[s].as_ref().and_then(|sl| sl.va_a) {
                Some(va) => {
                    w385_check_installed(&mut rm, &w, &mut rep, space, va, it);
                }
                None => did = W385Op::Idle,
            },

            // ── FREE, and assert both the handle and the VA went away ───────────────────
            W385Op::Free => match slots[s].take() {
                Some(sl) => w385_retire(&mut rm, &w, &mut rep, vas, space, sl, it),
                None => did = W385Op::Idle,
            },

            // ── RECYCLE — free and re-allocate into the SAME slot, immediately ──────────
            W385Op::Recycle => match slots[s].take() {
                Some(sl) => {
                    w385_retire(&mut rm, &w, &mut rep, vas, space, sl, it);
                    jitter(&mut rng);
                    match rm.alloc_probe_local(W385_BYTES) {
                        Ok(mem) => {
                            rep.minted.push(mem.raw());
                            // ★★ INVARIANT 5 — read the fresh object BEFORE writing the
                            // sentinel over it. Filling first would erase the exact
                            // evidence: a recycled handle still holding a magic.
                            w385_check_fresh(&rm, &w, &mut rep, mem, it);
                            if rm.fill_words(mem, W385_BYTES, W385_SENTINEL, 0).is_err() {
                                let _ = rm.free(mem);
                                rep.refused += 1;
                            } else {
                                slots[s] = Some(W385Slot {
                                    mem,
                                    va_a: None,
                                    va_b: None,
                                });
                            }
                        }
                        Err(_) => rep.refused += 1,
                    }
                }
                None => did = W385Op::Idle,
            },

            W385Op::Idle => {}
        }

        let end = origin.elapsed().as_nanos();
        rep.ops[did.code() as usize] += 1;
        if did != W385Op::Idle {
            rep.spans.push((w.tid, start, end));
        }
        beat.note(w.tid, did, it, origin.elapsed().as_millis() as u64);
    }

    // ── teardown. Every object this worker made, disposed by this worker. ───────────────
    for sl in slots.into_iter().flatten() {
        if let Some(va) = sl.va_b {
            let _ = rm.unmap_local(vas, va);
        }
        if let Some(va) = sl.va_a {
            let _ = rm.unmap_local(vas, va);
        }
        let _ = rm.free(sl.mem);
    }
    let _ = rm.free(chan);
    rep.finished = true;
    beat.done[w.tid].store(true, std::sync::atomic::Ordering::Relaxed);
    rep
}

/// ★ INVARIANT 5 — a **fresh** object must read as nothing, never as somebody's magic.
///
/// ⊘ A fresh object legitimately holds whatever the driver last left there, so the only
/// value that is a *finding* is one this rung's writers produced: a word carrying
/// [`W385_MAGIC_TAG`]. Anything else is uninitialised memory and is not graded.
fn w385_check_fresh(
    rm: &HostRmBackend,
    w: &W385Worker,
    rep: &mut W385Report,
    mem: kayfabe_isolate::HostHandle,
    it: u64,
) {
    let Ok(words) = rm.read_words_independently(mem, W385_BYTES, &W385_OFFS) else {
        return;
    };
    for (i, &word) in words.iter().enumerate() {
        if let Some((c, t)) = w385_owner(word) {
            let name = if c == w.client as u32 {
                "STALE_READ"
            } else {
                "CROSS_CLIENT_LEAK"
            };
            rep.violate(
                name,
                format!(
                    "tid {} it {it}: a FRESH object reads {word:#010x} at +{:#x} — client \
                     {c} thread {t} wrote it, so this storage is still somebody's",
                    w.tid, W385_OFFS[i]
                ),
            );
        }
    }
}

/// ★ INVARIANT 2 — a VA RM said it mapped must not read as `Free` to RM's own allocator.
///
/// ⊘ `Relocated` is **not** graded as a violation here: RM treating a fixed ask as a hint is
/// a statement about our address choice, and folding it in would manufacture reds out of a
/// legal allocator behaviour. Only `Free` — *"nothing is there"* over a live mapping — is.
fn w385_check_installed(
    rm: &mut HostRmBackend,
    w: &W385Worker,
    rep: &mut W385Report,
    space: u32,
    va: u64,
    it: u64,
) {
    if matches!(
        rm.probe_va(space, va, W385_BYTES),
        Ok(kayfabe_isolate_host::rm::VaProbe::Free)
    ) {
        rep.violate(
            "MAP_NOT_ATOMIC",
            format!(
                "tid {} it {it}: {va:#018x} is mapped and RM's allocator answers Free — a \
                 mapping observable half-installed",
                w.tid
            ),
        );
    }
}

/// Grade one placement. A drift is counted; a drift **out of this worker's window** is a red,
/// because that is how one worker comes to scribble on another's invariant.
fn w385_check_placement(w: &W385Worker, rep: &mut W385Report, asked: u64, got: u64, it: u64) {
    if got != asked && !w.owns(got) {
        rep.violate(
            "WINDOW_ESCAPE",
            format!(
                "tid {} it {it}: asked {asked:#018x}, RM placed {got:#018x} — OUTSIDE this \
                 worker's window {:#018x}..",
                w.tid,
                w.window()
            ),
        );
    }
}

/// ★★★ INVARIANTS 1 and 4 — grade what actually arrived at an address this worker wrote.
///
/// The magic carries its writer's identity, so the three outcomes are distinguishable:
/// our own magic (fine), **somebody else's** magic (the finding), or nothing (a lost write).
fn w385_grade_write(
    w: &W385Worker,
    rep: &mut W385Report,
    out: W379Release,
    magic: u32,
    va: u64,
    off: u64,
    it: u64,
) {
    match out {
        W379Release::Landed => {}
        W379Release::Refused => rep.refused += 1,
        W379Release::Lost { saw } => match w385_owner(saw) {
            Some((c, t)) if c != w.client as u32 => rep.violate(
                "CROSS_CLIENT_LEAK",
                format!(
                    "tid {} it {it}: wrote {magic:#010x} at {va:#018x}+{off:#x}; the object \
                     holds {saw:#010x}, written by CLIENT {c} thread {t}",
                    w.tid
                ),
            ),
            Some((c, t)) if t != w.tid as u32 => rep.violate(
                "VA_DOUBLE_BOUND",
                format!(
                    "tid {} it {it}: wrote {magic:#010x} at {va:#018x}+{off:#x}; the object \
                     holds {saw:#010x}, written by client {c} THREAD {t} — this VA or this \
                     memory is bound twice",
                    w.tid
                ),
            ),
            _ => rep.violate(
                "RELEASE_LOST",
                format!(
                    "tid {} it {it}: {magic:#010x} never reached {va:#018x}+{off:#x} \
                     (object holds {saw:#010x})",
                    w.tid
                ),
            ),
        },
    }
}

/// ★ INVARIANT 5 — retire one slot and assert both halves of *"it went away"*: the freed
/// handle must stop resolving, and the VA must come back.
///
/// ⊘ The handle check is only sound because [`RmConnection::mint`] is a **monotonic**
/// counter — no handle value is reissued inside one run — so a `map_cpu` that succeeds on a
/// freed handle cannot be explained by reuse.
fn w385_retire(
    rm: &mut HostRmBackend,
    w: &W385Worker,
    rep: &mut W385Report,
    vas: kayfabe_isolate::HostHandle,
    space: u32,
    sl: W385Slot,
    it: u64,
) {
    let vas_had = (sl.va_a, sl.va_b);
    if let Some(va) = sl.va_b {
        let _ = rm.unmap_local(vas, va);
    }
    if let Some(va) = sl.va_a {
        let _ = rm.unmap_local(vas, va);
    }
    let freed = rm.free(sl.mem).is_ok();
    if !freed {
        rep.refused += 1;
        return;
    }
    if rm
        .read_words_independently(sl.mem, W385_BYTES, &[0])
        .is_ok()
    {
        rep.violate(
            "FREED_HANDLE_RESOLVES",
            format!(
                "tid {} it {it}: handle {:#x} was freed and a fresh CPU mapping of it still \
                 succeeded",
                w.tid,
                sl.mem.raw()
            ),
        );
    }
    for va in [vas_had.0, vas_had.1].into_iter().flatten() {
        match rm.probe_va(space, va, W385_BYTES) {
            Ok(kayfabe_isolate_host::rm::VaProbe::Free) => {}
            other => rep.violate(
                "VA_NOT_RECOVERED",
                format!(
                    "tid {} it {it}: {va:#018x} did not come back after its object was \
                     freed (probe={other:?})",
                    w.tid
                ),
            ),
        }
    }
}

/// The knobs, so the rung can be dialled up until it reds or the budget is stated.
#[derive(Debug, Clone, Copy)]
struct W385Cfg {
    threads: usize,
    iters: u64,
    clients: usize,
    jitter_us: u64,
    seed: u64,
    /// Whole-rung watchdog, seconds.
    deadline_s: u64,
    /// A worker silent for this long is called wedged, even if the whole rung has time left.
    stall_s: u64,
    /// How this phase places its workers on cores. See [`W385Pin`].
    pin: W385Pin,
    /// ★ How many cores the pinned arms use. Defaults to the **bench guest's `-smp`**
    /// (`scripts/bench/boot_nvkvm.sh`), because the topology worth reproducing is the one
    /// the product actually presents — not a round number.
    cores: usize,
}

/// ★★ THE WATCHDOG. A deadlock must FAIL BY NAME, and it must fail *while* it is deadlocked
/// — after the fact there is nothing to dump.
///
/// ⊘ It ends the process rather than unwinding, and that is deliberate: a worker wedged in
/// an RM ioctl cannot be joined, cancelled or timed out from here, so the only honest
/// choices are *"print what we know and die"* or *"hang"*. It prints the verdict line first,
/// so a grader reading printed lines gets a `FAIL` and never a silence.
fn w385_watchdog(
    cfg: W385Cfg,
    beat: std::sync::Arc<W385Beat>,
    origin: std::time::Instant,
    phase: &'static str,
) {
    use std::sync::atomic::Ordering::Relaxed;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let now = origin.elapsed();
        let now_ms = now.as_millis() as u64;
        if beat.stop.load(Relaxed) || beat.done.iter().all(|d| d.load(Relaxed)) {
            return;
        }
        let expired = now.as_secs() >= cfg.deadline_s;
        let stalled: Vec<usize> = (0..beat.op.len())
            .filter(|&t| {
                !beat.done[t].load(Relaxed)
                    && beat.op[t].load(Relaxed) != 0
                    && now_ms.saturating_sub(beat.last_ms[t].load(Relaxed)) >= cfg.stall_s * 1000
            })
            .collect();
        if !expired && stalled.is_empty() {
            continue;
        }
        println!(
            "FAIL  W385 WATCHDOG       = phase {phase}: {} after {}s. ⊘ A nontermination \
             that WEDGED instead of failing is the shape this exists to refuse",
            if expired {
                format!("the {}s rung deadline expired", cfg.deadline_s)
            } else {
                format!("workers {stalled:?} silent for >= {}s", cfg.stall_s)
            },
            now.as_secs()
        );
        println!("⚠     W385 last known    = what each worker was doing when time ran out:");
        beat.dump(now_ms);
        println!("FUZZ_SEED={:#018x}", cfg.seed);
        println!("FUZZ_REASON=DEADLOCK/WATCHDOG");
        println!("RUNGCTL_concurrent_fuzz=FAIL");
        println!("RUNG_concurrent_fuzz=FAIL");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        // ⊘ Objects leak. That is the correct trade against a hang, and it is stated rather
        // than hidden: this process is a diagnostic, and the run is over either way.
        std::process::exit(3);
    }
}

/// What one phase (the control, or the fuzz) measured.
struct W385Phase {
    /// How this phase placed its workers.
    pin: W385Pin,
    /// How many workers the kernel actually accepted a mask for. ⊘ `0` on an `Unpinned`
    /// arm by construction; `< workers` on a pinned arm is an UNMEASURED pinning, not a
    /// successful one.
    pinned: usize,
    /// The distinct core sets the workers observed, as printed strings — the attributable
    /// record of where this arm actually ran.
    core_sets: std::collections::BTreeSet<String>,
    /// `(tid, raw handle)` for every allocation any worker made. See [`W385Report::minted`].
    minted: Vec<(usize, u64)>,
    violations: Vec<(&'static str, String)>,
    ops: [u64; 11],
    refused: u64,
    finished: usize,
    workers: usize,
    /// Pairs of RM-verb intervals from **different** threads that actually intersected.
    /// ★ This is the concurrency *control*: zero of it means nothing was sampled.
    overlap_pairs: u64,
    /// How many spans the overlap count was computed over — see [`W385_SPAN_CAP`].
    spans_used: usize,
    total_spans: usize,
    /// Op durations in microseconds, sorted. Reported as a distribution, never a mean.
    durations: Vec<u64>,
    wall_ms: u128,
}

/// Spans the overlap census runs over. The census is O(n²) and the whole point is a
/// **positive** count, so a cap is honest as long as it is reported — which it is.
const W385_SPAN_CAP: usize = 6000;

/// Run one phase: spawn `cfg.threads` workers across `cfg.clients` RM clients, join them,
/// and fold their reports into one.
fn w385_phase(
    cfg: W385Cfg,
    conns: &[std::sync::Arc<RmConnection>],
    vas_raws: &[u64],
    engine_type: u32,
    probe: W381Probe,
    phase: &'static str,
) -> W385Phase {
    let origin = std::time::Instant::now();
    let beat = std::sync::Arc::new(W385Beat::new(cfg.threads));
    let wd_beat = std::sync::Arc::clone(&beat);
    std::thread::spawn(move || w385_watchdog(cfg, wd_beat, origin, phase));

    let mut handles = Vec::new();
    // Per-client lane counters, so lane N of client 0 and lane N of client 1 name the SAME
    // VA window — the collision invariant 4 is about.
    let mut lanes = vec![0usize; conns.len()];
    for tid in 0..cfg.threads {
        let client = tid % conns.len();
        let lane = lanes[client];
        lanes[client] += 1;
        let w = W385Worker {
            tid,
            client,
            lane,
            iters: cfg.iters,
            jitter_us: cfg.jitter_us,
            // ★ Each stream is a pure function of the ONE printed seed and the worker's
            // identity, so `--seed=N` reproduces every worker's decision sequence.
            seed: cfg
                .seed
                .wrapping_mul(0x2545_F491_4F6C_DD1D)
                .wrapping_add((tid as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            pin: cfg.pin,
        };
        let conn = std::sync::Arc::clone(&conns[client]);
        let vas_raw = vas_raws[client];
        let beat = std::sync::Arc::clone(&beat);
        handles.push(std::thread::spawn(move || {
            w385_run_worker(w, conn, vas_raw, engine_type, probe, origin, beat)
        }));
    }

    let mut out = W385Phase {
        pin: cfg.pin,
        pinned: 0,
        core_sets: std::collections::BTreeSet::new(),
        minted: Vec::new(),
        violations: Vec::new(),
        ops: [0; 11],
        refused: 0,
        finished: 0,
        workers: cfg.threads,
        overlap_pairs: 0,
        spans_used: 0,
        total_spans: 0,
        durations: Vec::new(),
        wall_ms: 0,
    };
    let mut spans: Vec<(usize, u128, u128)> = Vec::new();
    for (tid, h) in handles.into_iter().enumerate() {
        match h.join() {
            Ok(rep) => {
                if rep.finished {
                    out.finished += 1;
                }
                if rep.pinned {
                    out.pinned += 1;
                }
                out.core_sets.insert(w385_cores_str(&rep.observed_cores));
                for h in rep.minted {
                    out.minted.push((tid, h));
                }
                out.refused += rep.refused;
                for (i, n) in rep.ops.iter().enumerate() {
                    out.ops[i] += n;
                }
                out.violations.extend(rep.violations);
                spans.extend(rep.spans);
            }
            // ★★★ A PANIC IN A WORKER IS A RESULT, and it is the one a lock-rank witness
            // produces. Swallowing it would turn the loudest possible red into a missing row.
            Err(_) => out.violations.push((
                "WORKER_PANIC",
                format!("tid {tid} panicked — a witness assert or an unwrap inside a verb"),
            )),
        }
    }
    // ★★★ HANDLE UNIQUENESS — checked HERE because it is cross-worker by nature.
    //
    // `mint()` is `o.next; o.next += 1` under the `objects` mutex. If that mutex ever failed
    // to serialise, two workers would receive the SAME raw handle, and every downstream
    // symptom — a free that takes somebody else's object, a map onto a stranger's memory —
    // would be attributed to the mapping plane instead. ⊘ Within one connection the counter
    // is monotonic so a repeat is unambiguous; ACROSS connections handles legitimately
    // repeat, which is why the check is scoped to the workers of one client.
    {
        let mut seen: std::collections::HashMap<(usize, u64), usize> =
            std::collections::HashMap::new();
        for &(tid, h) in &out.minted {
            let client = tid % cfg.clients.max(1);
            if let Some(&prev) = seen.get(&(client, h)) {
                out.violations.push((
                    "HANDLE_COLLISION",
                    format!(
                        "client {client}: raw handle {h:#x} was minted for worker {prev} AND \
                         worker {tid} — `RmConnection::mint` did not serialise"
                    ),
                ));
            } else {
                seen.insert((client, h), tid);
            }
        }
    }
    // ★ The watchdog is retired HERE, by the phase, not by the workers — see [`W385Beat`].
    beat.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    out.wall_ms = origin.elapsed().as_millis();
    out.total_spans = spans.len();
    for &(_, s, e) in &spans {
        out.durations.push((e.saturating_sub(s) / 1000) as u64);
    }
    out.durations.sort_unstable();
    spans.sort_unstable_by_key(|s| s.1);
    let used = spans.len().min(W385_SPAN_CAP);
    out.spans_used = used;
    for i in 0..used {
        for j in (i + 1)..used {
            // sorted by start, so once a later span starts after this one ends, none of the
            // remaining ones can overlap it either.
            if spans[j].1 >= spans[i].2 {
                break;
            }
            if spans[j].0 != spans[i].0 {
                out.overlap_pairs += 1;
            }
        }
    }
    out
}

/// Print one phase's census: the **distribution and `n`**, never a summary number.
fn w385_report(phase: &str, p: &W385Phase) {
    let n = p.durations.len();
    let pct = |q: f64| -> u64 {
        if n == 0 {
            0
        } else {
            p.durations[((n as f64 - 1.0) * q) as usize]
        }
    };
    let total: u64 = p.ops.iter().sum();
    println!(
        "info  W385 {phase:<8} census = {total} ops over {} workers in {} ms, {} refused, \
         {}/{} workers finished",
        p.workers, p.wall_ms, p.refused, p.finished, p.workers
    );
    let mut line = String::new();
    for op in W385Op::ALL {
        line.push_str(&format!("{}={} ", op.as_str(), p.ops[op.code() as usize]));
    }
    println!("info  W385 {phase:<8} verbs  = {}", line.trim_end());
    println!(
        "info  W385 {phase:<8} op us   = n={n} min={} p50={} p90={} p99={} max={}  ⊘ a \
         distribution, because one number is not a measurement",
        p.durations.first().copied().unwrap_or(0),
        pct(0.50),
        pct(0.90),
        pct(0.99),
        p.durations.last().copied().unwrap_or(0)
    );
    println!(
        "info  W385 {phase:<8} overlap = {} pairs of RM-verb intervals from DIFFERENT \
         threads intersected, over {}/{} spans",
        p.overlap_pairs, p.spans_used, p.total_spans
    );
    println!(
        "info  W385 {phase:<8} handles = {} minted across {} worker(s), each checked for \
         collision against every other worker OF THE SAME CLIENT",
        p.minted.len(),
        p.workers
    );
    // ★ WHERE IT ACTUALLY RAN, not where it was asked to run.
    println!(
        "info  W385 {phase:<8} pinning = mode {} — {}/{} workers accepted a mask; observed \
         core sets {:?}",
        p.pin.as_str(),
        p.pinned,
        p.workers,
        p.core_sets
    );
}

/// A core set as a short printable string — `"3"`, `"0-2"`, `"0,4,9"`. Used only for the
/// attributable pinning line, which is why it collapses runs rather than listing 19 numbers.
fn w385_cores_str(cores: &[usize]) -> String {
    if cores.is_empty() {
        return "?".into();
    }
    let mut out = String::new();
    let mut i = 0;
    while i < cores.len() {
        let mut j = i;
        while j + 1 < cores.len() && cores[j + 1] == cores[j] + 1 {
            j += 1;
        }
        if !out.is_empty() {
            out.push(',');
        }
        if j > i {
            out.push_str(&format!("{}-{}", cores[i], cores[j]));
        } else {
            out.push_str(&format!("{}", cores[i]));
        }
        i = j + 1;
    }
    out
}

/// ★★★★★ **w385 — THE RUNG.** Positive control first, then the fuzz, then the grade.
#[allow(clippy::too_many_lines)]
fn concurrent_fuzz(
    conn: &std::sync::Arc<RmConnection>,
    probe: W381Probe,
    gpu: u32,
    cfg: W385Cfg,
) -> bool {
    println!(
        "info  W385 fuzz           = GPU {gpu}, euid {} — {} threads x {} iterations over \
         {} RM client(s), jitter <= {} us",
        kayfabe_linux_raw::geteuid(),
        cfg.threads,
        cfg.iters,
        cfg.clients,
        cfg.jitter_us
    );
    // ★★★ THE TOPOLOGY, PRINTED, because it is a claim about the machine and not about us.
    // `available_parallelism` is what the scheduler will actually give an unpinned arm, and
    // `cfg.cores` is the guest's `-smp` we are reproducing; a run where the box has FEWER
    // cores than the guest claims is a run whose `unpinned` control is not a control.
    let host_cores = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    println!(
        "info  W385 topology       = host offers {host_cores} core(s); the pinned arms use \
         {} — the bench guest's `-smp` (scripts/bench/boot_nvkvm.sh). ★ In kayfabe EACH \
         GUEST vCPU IS A HOST THREAD, so this is the contention the product must survive",
        cfg.cores
    );
    if host_cores > 0 && host_cores < cfg.cores {
        println!(
            "⚠     W385 topology       = the host has FEWER cores ({host_cores}) than the \
             pinned arms ask for ({}). The `unpinned` arm is then over-subscribed too and \
             is NOT a clean control for the pinned one",
            cfg.cores
        );
    }
    // ★★★ THE SEED, FIRST AND UNCONDITIONALLY. A fuzz failure you cannot reproduce is an
    // anecdote, and a seed printed only on success is a seed you do not have when it reds.
    println!("FUZZ_SEED={:#018x}", cfg.seed);
    println!(
        "info  W385 replay         = --concurrent-fuzz --seed {} --fuzz-threads {} \
         --fuzz-iters {} --fuzz-clients {}",
        cfg.seed, cfg.threads, cfg.iters, cfg.clients
    );
    println!(
        "info  W385 the bar        = five invariants, each refused BY NAME: \
         VA_DOUBLE_BOUND, MAP_NOT_ATOMIC, ALIAS_MISMATCH/ALIAS_REVOKED, CROSS_CLIENT_LEAK, \
         FREED_HANDLE_RESOLVES/VA_NOT_RECOVERED/STALE_READ"
    );
    println!(
        "⊘     W385 seed scope     = the seed replays the DECISION SEQUENCE, not the OS \
         SCHEDULE. A red names a reproducible program; the interleaving that made it red is \
         not ours to reproduce"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  W385 engine         = COPY0 is not expressible");
        println!("FUZZ_REASON=NO_ENGINE");
        println!("RUNGCTL_concurrent_fuzz=FAIL");
        println!("RUNG_concurrent_fuzz=NOTRUN");
        return false;
    };

    // ── the clients. Client 0 is the caller's; every further one is a fresh root. ───────
    let mut conns: Vec<std::sync::Arc<RmConnection>> = vec![std::sync::Arc::clone(conn)];
    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            println!("??    W385 second client  = open(/dev) refused: {e}");
            println!("FUZZ_REASON=NO_DEVICE");
            println!("RUNGCTL_concurrent_fuzz=FAIL");
            println!("RUNG_concurrent_fuzz=NOTRUN");
            return false;
        }
    };
    for i in 1..cfg.clients {
        match RmConnection::open(&dev, GpuId(gpu), kayfabe_chips::pinned_host_classes()) {
            Ok(c) => conns.push(std::sync::Arc::new(c)),
            Err(e) => {
                println!(
                    "??    W385 client {i}        = a further RM client could not be opened: \
                     {e}. ⊘ NOT an isolation result — the experiment never ran"
                );
                println!("FUZZ_REASON=NO_SECOND_CLIENT");
                println!("RUNGCTL_concurrent_fuzz=FAIL");
                println!("RUNG_concurrent_fuzz=NOTRUN");
                return false;
            }
        }
    }
    let roots: Vec<u32> = conns.iter().map(|c| c.client()).collect();
    println!(
        "info  W385 hClients       = {:?} — {} distinct root(s)",
        roots
            .iter()
            .map(|r| format!("{r:#010x}"))
            .collect::<Vec<_>>(),
        {
            let mut u = roots.clone();
            u.sort_unstable();
            u.dedup();
            u.len()
        }
    );

    // ── one shared VAS per client. ★ SHARED, not per-thread: a per-thread address space
    //    would leave RM's per-VAS page tables uncontended, which is the plane this rung is
    //    named after. The per-worker VA WINDOWS are what keep a violation attributable.
    let mut backends: Vec<HostRmBackend> = Vec::new();
    let mut vas_raws: Vec<u64> = Vec::new();
    let mut vas_handles: Vec<kayfabe_isolate::HostHandle> = Vec::new();
    for (i, c) in conns.iter().enumerate() {
        let mut b = if i == 0 {
            HostRmBackend::new(
                IsolateId::new(0, GpuId(gpu)),
                std::sync::Arc::clone(c),
                std::sync::Arc::new(kayfabe_isolate_host::ChildExports::new()),
            )
        } else {
            HostRmBackend::new(
                IsolateId::new(i as u32, GpuId(gpu)),
                std::sync::Arc::clone(c),
                std::sync::Arc::new(kayfabe_isolate_host::ChildExports::new()),
            )
        };
        match b.alloc_vaspace() {
            Ok(v) => {
                vas_raws.push(v.raw());
                vas_handles.push(v);
                backends.push(b);
            }
            Err(e) => {
                println!("FAIL  W385 vaspace {i}       = refused {e:?}");
                println!("FUZZ_REASON=NO_VASPACE");
                println!("RUNGCTL_concurrent_fuzz=FAIL");
                println!("RUNG_concurrent_fuzz=NOTRUN");
                return false;
            }
        }
    }
    // ── ★ THE POSITIVE CONTROL. Same operations, T=1, NO jitter, one client. If this does
    //    not come back clean the rung is broken and nothing after it is interpretable.
    println!(
        "\ninfo  W385 CONTROL        = the SAME verbs at T=1 with NO jitter. ⊘ If this reds, \
         the RUNG is broken, not the system, and everything below is UNINTERPRETABLE"
    );
    let ctl_cfg = W385Cfg {
        threads: 1,
        iters: cfg.iters.clamp(24, 96),
        clients: 1,
        jitter_us: 0,
        seed: cfg.seed ^ 0x385,
        // ⊘ The control is UNPINNED on purpose: its job is to say the VERBS work, and a
        // control that also changed the placement could not do that for either arm.
        pin: W385Pin::Unpinned,
        ..cfg
    };
    let ctl = w385_phase(
        ctl_cfg,
        &conns[..1],
        &vas_raws[..1],
        engine_type,
        probe,
        "control",
    );
    w385_report("control", &ctl);
    let ctl_engine: u64 = W385Op::ENGINE
        .iter()
        .map(|o| ctl.ops[o.code() as usize])
        .sum();
    // ⊘ The control passes only if it also RAN THE ENGINE. A control that allocated and
    // never wrote proves the allocator works and says nothing about the plane every arm
    // below it grades on.
    let control_ok = ctl.violations.is_empty() && ctl.finished == 1 && ctl_engine > 0;
    if !control_ok {
        println!(
            "⊘     W385 CONTROL FAILED = {} violation(s) with ONE thread and no jitter, \
             {ctl_engine} engine ops, {}/1 workers finished",
            ctl.violations.len(),
            ctl.finished
        );
        for (name, detail) in ctl.violations.iter().take(W385_SHOW) {
            println!("        {name}: {detail}");
        }
    } else {
        println!(
            "ok    W385 control        = {} single-threaded ops of which {ctl_engine} ran \
             the ENGINE, zero violations — the verbs, the channel and the oracle all work",
            ctl.ops.iter().sum::<u64>()
        );
    }

    // ── ★★★ THE FUZZ, AS A PAIRED DIFFERENTIAL OVER CPU PLACEMENT. ────────────────────
    //
    // Owner ruling, 2026-09-06: pin to vCPU cores, because in kayfabe **each guest vCPU IS a
    // host thread**. ⊘ And do NOT silently replace the unpinned arm with a pinned one: run
    // both, at the SAME width, so *"pinning changed the answer"* is a measurement.
    //
    //   unpinned   T = cfg.threads (default = the bench guest's `-smp`)   the control
    //   percore    T = cfg.cores,  one worker per core                    the topology
    //   crowd-free T = 3 x cores,  unpinned                               the crowd's control
    //   crowd      T = 3 x cores,  ALL on the same `cores`-wide set       the over-subscribed arm
    //
    // ★★★ The last one is the one that matters most and the reason it exists is worth
    // stating: un-pinned parallelism on a wide box tends to run threads PAST each other, so a
    // critical section is entered and left before anybody else gets there. Forcing more
    // runnable workers than cores makes the scheduler preempt **inside** a held lock, which
    // is where a lock-order bug actually lives.
    let crowd_t = (cfg.cores * 3).max(cfg.threads);
    let arms: [(&'static str, W385Cfg); 4] = [
        (
            "unpinned",
            W385Cfg {
                pin: W385Pin::Unpinned,
                ..cfg
            },
        ),
        (
            "percore",
            W385Cfg {
                threads: cfg.cores,
                pin: W385Pin::PerCore { cores: cfg.cores },
                ..cfg
            },
        ),
        (
            "crowd-free",
            W385Cfg {
                threads: crowd_t,
                pin: W385Pin::Unpinned,
                ..cfg
            },
        ),
        (
            "crowd",
            W385Cfg {
                threads: crowd_t,
                pin: W385Pin::Crowd { cores: cfg.cores },
                ..cfg
            },
        ),
    ];

    let mut results: Vec<(&'static str, W385Cfg, W385Phase, &'static str)> = Vec::new();
    for (name, acfg) in arms {
        println!(
            "\ninfo  W385 ARM {name:<10} = {} threads across {} client(s), placement {}, \
             seeded jitter <= {} us",
            acfg.threads,
            acfg.clients,
            acfg.pin.as_str(),
            acfg.jitter_us
        );
        let ph = w385_phase(acfg, &conns, &vas_raws, engine_type, probe, "fuzz");
        w385_report(name, &ph);
        for (n, d) in ph.violations.iter().take(W385_SHOW) {
            println!("        {name}/{n}: {d}");
        }
        // ── the per-arm grade. ⊘ ZERO OVERLAP VETOES A GREEN: an arm in which no two
        //    threads' RM verbs ever intersected sampled no concurrency at all, and calling
        //    that PASS reports a finding never measured.
        // ⊘ A PINNED arm whose masks were REFUSED is likewise UNMEASURED, not passed — it
        //    ran as the unpinned arm under a pinned arm's name, which is worse than not
        //    running: it would be read as "pinning changed nothing".
        // ★★★ THE COVERAGE VETO, and it is not a nicety. Invariants 1, 3 and 4 are only
        // tested when the ENGINE runs; an arm that allocated and mapped and never wrote is
        // green about nothing. Measured before this existed: three arms at `I=12` reported
        // PASS with `write=0`.
        let engine: u64 = W385Op::ENGINE
            .iter()
            .map(|o| ph.ops[o.code() as usize])
            .sum();
        // ⊘ EVERY BRANCH CARRIES ITS OWN REASON, and that is not cosmetic: three arms of
        // this chain all answer `NOTRUN`, and *"the arm was unmeasured"* is useless without
        // *"because its pin masks were refused"* / *"because it sampled no overlap"* /
        // *"because it never ran the engine"*. Those are three different repairs. ★ It also
        // happens to be what makes the blocks distinguishable to `clippy::if_same_then_else`,
        // which flagged the collapsed version — the lint was right for the wrong reason.
        let (v, why) = if ph.finished != ph.workers {
            ("FAIL", "WORKER_DID_NOT_FINISH")
        } else if acfg.pin != W385Pin::Unpinned && ph.pinned != ph.workers {
            ("NOTRUN", "PIN_REFUSED")
        } else if ph.overlap_pairs == 0 {
            ("NOTRUN", "NO_CONCURRENCY_OBSERVED")
        } else if engine == 0 {
            ("NOTRUN", "NO_ENGINE_WORK_SAMPLED")
        } else if ph.violations.is_empty() {
            ("PASS", "CLEAN")
        } else {
            ("FAIL", "INVARIANT_VIOLATED")
        };
        let ops: u64 = ph.ops.iter().sum();
        println!(
            "FUZZ_ARM={name} verdict={v} why={why} threads={} pin={} cores={} ops={ops} \
             engine_ops={engine} alias_ops={} overlap={} viol={} refused={} pinned={}/{} \
             finished={}/{}",
            acfg.threads,
            acfg.pin.as_str(),
            cfg.cores,
            ph.ops[W385Op::ALIAS.code() as usize],
            ph.overlap_pairs,
            ph.violations.len(),
            ph.refused,
            ph.pinned,
            ph.workers,
            ph.finished,
            ph.workers
        );
        if v == "NOTRUN" {
            println!(
                "⊘     W385 {name:<10} = UNMEASURED because {why}. finished={}/{}  \
                 pinned={}/{}  overlap={}  engine_ops={engine} — ⊘ NOT a pass and NOT a \
                 failure; the third value exists for exactly this",
                ph.finished, ph.workers, ph.pinned, ph.workers, ph.overlap_pairs
            );
        }
        results.push((name, acfg, ph, v));
    }

    // ── teardown of the shared address spaces. ─────────────────────────────────────────
    for (b, v) in backends.iter_mut().zip(vas_handles.iter()) {
        let _ = b.free(*v);
    }

    // ── the census, by invariant name, ACROSS EVERY ARM. ⊘ Counts uncapped. ────────────
    let mut by_name: std::collections::BTreeMap<(&'static str, &'static str), usize> =
        std::collections::BTreeMap::new();
    for (name, _, ph, _) in &results {
        for (n, _) in &ph.violations {
            *by_name.entry((*name, n)).or_default() += 1;
        }
    }
    for ((arm, name), count) in &by_name {
        println!("FUZZ_VIOLATION={name} arm={arm} n={count}");
    }

    let ops: u64 = results
        .iter()
        .map(|(_, _, p, _)| p.ops.iter().sum::<u64>())
        .sum();
    let overlap: u64 = results.iter().map(|(_, _, p, _)| p.overlap_pairs).sum();
    let viol: usize = results.iter().map(|(_, _, p, _)| p.violations.len()).sum();
    println!("FUZZ_THREADS={}", cfg.threads);
    println!("FUZZ_CORES={}", cfg.cores);
    println!("FUZZ_ITERS={}", cfg.iters);
    println!("FUZZ_CLIENTS={}", cfg.clients);
    println!("FUZZ_ARMS={}", results.len());
    println!("FUZZ_OPS={ops}");
    println!("FUZZ_OVERLAP_PAIRS={overlap}");
    println!("FUZZ_VIOLATIONS={viol}");
    println!(
        "FUZZ_REFUSED={}",
        results.iter().map(|(_, _, p, _)| p.refused).sum::<u64>()
    );

    // ★★★ THE PAIRING RULE, EVALUATED HERE RATHER THAN LEFT TO THE READER. If a pinned arm
    // reds while its same-width unpinned control is green, THAT IS THE FINDING, and it says
    // the unpinned arm was never a test of this.
    let grade = |n: &str| {
        results
            .iter()
            .find(|(a, _, _, _)| *a == n)
            .map(|(_, _, _, v)| *v)
    };
    for (pinned, control) in [("percore", "unpinned"), ("crowd", "crowd-free")] {
        match (grade(pinned), grade(control)) {
            (Some("FAIL"), Some("PASS")) => println!(
                "★★★★★ W385 PINNING FOUND IT = arm `{pinned}` RED while its same-width \
                 unpinned control `{control}` is GREEN. ⇒ the unpinned arm was never a test \
                 of this. FUZZ_PINNING_DELTA={pinned}"
            ),
            (Some(a), Some(b)) => println!(
                "info  W385 pairing        = {pinned}={a} vs {control}={b} — placement did \
                 not change the answer"
            ),
            _ => println!("info  W385 pairing        = {pinned} / {control} incomparable"),
        }
    }

    // ── ★★ THE OVERALL GRADE. Every outcome pre-registered. ───────────────────────────
    let verdict = if !control_ok {
        println!("FUZZ_REASON=CONTROL_FAILED");
        "NOTRUN"
    } else if results.iter().any(|(_, _, _, v)| *v == "FAIL") {
        println!("FUZZ_REASON=INVARIANT_VIOLATED");
        "FAIL"
    } else if results.iter().all(|(_, _, _, v)| *v == "PASS") {
        println!("FUZZ_REASON=CLEAN");
        "PASS"
    } else {
        // ⊘ Some arm was UNMEASURED (no overlap sampled, or a pinned arm whose masks were
        // refused). Not a pass and not a failure — the third value exists for exactly this.
        println!("FUZZ_REASON=ARM_UNMEASURED");
        "NOTRUN"
    };

    if verdict == "PASS" {
        println!(
            "★     W385 FUZZ CLEAN     = {ops} operations over {} arms (up to {crowd_t} \
             threads, {} client(s), {} cores), {overlap} measured cross-thread overlaps, \
             zero invariant violations. ⚠ THAT IS A BUDGET, NOT A PROOF: absence of a red \
             is not absence of a race, and every arm sampled ONE schedule of one seed",
            results.len(),
            cfg.clients,
            cfg.cores
        );
    }
    println!(
        "RUNGCTL_concurrent_fuzz={}",
        if control_ok { "PASS" } else { "FAIL" }
    );
    println!("RUNG_concurrent_fuzz={verdict}");
    verdict == "PASS"
}

/// `cmd[:size]` pairs, comma-separated. Size defaults to 4 — the width of the control
/// that motivated the rung — and is capped so a typo cannot ask RM to fill a huge buffer.
fn parse_ctrl_specs(s: &str) -> Result<Vec<(u32, usize)>, String> {
    const MAX: usize = 4096;
    let mut out = Vec::new();
    for item in s.split(',').filter(|i| !i.is_empty()) {
        let (c, sz) = item.split_once(':').unwrap_or((item, "4"));
        let c = c.trim().trim_start_matches("0x");
        let cmd = u32::from_str_radix(c, 16).map_err(|_| format!("{item}: bad control number"))?;
        let size: usize = sz.trim().parse().map_err(|_| format!("{item}: bad size"))?;
        if size == 0 || size > MAX {
            return Err(format!("{item}: size must be 1..={MAX}"));
        }
        out.push((cmd, size));
    }
    if out.is_empty() {
        return Err("no controls given".into());
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ w384 — WHAT ONE DOORBELL COSTS THE SUBMITTING THREAD
//
// Owner, 2026-09-06: *"the most valuable raw client is one that passes on host and fails in
// guest, then its just iterate unless there is a blocker."* All seven w379/w381 rungs pass
// natively and six of seven pass in the guest, so by that criterion they discriminate very
// little. This rung is built to have the valuable shape.
//
// ## THE DEFECT IT EXISTS TO GATE
//
// `[measured, LLM boot, w383 lane]` publication runs **60–71 ms per doorbell, INLINE on the
// vCPU thread**:
//
//     TRAPWITNESS off_trap_claims=0 inline_exceptions=61865 worst_trap=1750538us
//                 (target: inline_exceptions=0)
//
// Thousands of doorbells at that price is why the LLM rung is killed by a harness timeout
// with doorbells still being served. Nothing in this tree measured it in under twenty
// minutes, and `LLM_TOKENS` — the only feedback the async lane had — needs a full boot plus
// a model load and can come back UNMEASURED for reasons that have nothing to do with the
// change under test.
//
// ## ⊘ WHAT THIS RUNG IS NOT
//
// It is **not a performance target and not a benchmark**. It is a DISCRIMINATOR: it exists
// to answer one binary question — *is per-doorbell cost on the submitting thread within a
// stated multiple of what bare metal charges for the same act by the same binary?* Passing
// it is not a claim that anything is fast.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// Where this rung puts its channel ring. Distinct from every other rung's, so a run that
/// selects more than one cannot have its placements collide.
const DBL_RING_AT: u64 = 0x0000_000B_1000_0000;
/// Where the destination object is mapped. See [`DBL_RING_AT`].
const DBL_TARGET_AT: u64 = 0x0000_000B_5000_0000;
/// The base of the fresh-map arm's VAs. Each iteration takes the next 4 GiB stride, so no
/// two of them can share a page table leaf and be mistaken for one publication.
const DBL_FRESH_BASE: u64 = 0x0000_000C_0000_0000;
/// See [`DBL_FRESH_BASE`].
const DBL_FRESH_STRIDE: u64 = 0x1_0000_0000;

/// What the destination holds before the engine runs. Neither `0` nor `1`: a zero cannot be
/// told from freshly-allocated memory, and *"the magic is not there"* has to mean *"nothing
/// wrote here"* rather than *"we cannot tell"*.
const DBL_SENTINEL: u32 = 0xDEAD_0384;
/// The payload the **opening** control copies, at offset `0x00`.
const DBL_MAGIC_PRE: u32 = 0x0384_5EED;
/// The payload the **closing** control copies, at offset `0x40`.
const DBL_MAGIC_POST: u32 = 0x0384_C105;

/// ★★★ **THE GATE, AND WHY IT IS A MULTIPLE OF A MEASUREMENT RATHER THAN A NUMBER.**
///
/// ⊘ A hard-coded millisecond figure would be a standard nobody agreed to, on a box nobody
/// characterised. The floor here is whatever **bare metal** charges for the identical act —
/// same binary (md5 printed on both arms), same box, same week — and the gate is
/// `native_p50 × DBL_GATE_MULTIPLE`.
///
/// **`1000` is chosen because it is the only decade that separates the two things this rung
/// must tell apart**, and both sides of that band are measured rather than assumed:
///
/// - **The floor a CORRECT emulation cannot go below.** A doorbell inside a Mode-2 guest is
///   a trapped MMIO store: one VM exit, one dispatch through the VMM, one return. That is
///   tens of microseconds on this class of hardware, against a native path this rung
///   measures in single-digit microseconds — order **10–100×**, and an async design that
///   moves publication off the vCPU thread still pays it.
/// - **The DEFECT.** 60–71 ms per doorbell inline on the vCPU thread, against that same
///   single-digit-microsecond native path — order **10⁴×**.
///
/// ⇒ 1000× sits a decade **above** the most expensive honest emulation cost and a decade
/// **below** the defect. A gate outside that band either reddens a working async design or
/// greens the thing it exists to catch, so the multiple is not a taste.
///
/// ⚠ It is a *discrimination* threshold. A run that passes has not been shown to be fast; it
/// has been shown not to be paying the inline-publication price.
const DBL_GATE_MULTIPLE: f64 = 1000.0;

/// ★★ **The NATIVE arm's own bar** — because a calibration run cannot fail a gate derived
/// from itself, and a rung with no reachable red on one arm is a rung that arm cannot use.
///
/// On bare metal a doorbell is `release_fence()` plus one 32-bit store into a mapped
/// write-combining window, wrapped in a handful of stores into device memory. **Nothing in
/// that sequence can legitimately median above a millisecond.** If it does, the box is
/// contended, the window is not mapped where we think, or something else is on the path —
/// and in every one of those cases the number is not usable as the floor a guest gate is
/// multiplied out of. ⇒ a native median above this is a **red**, printed as one.
///
/// ⊘ Deliberately loose (three decades above the expected single-digit microseconds): its
/// job is to catch *"this calibration is not a calibration"*, not to police host jitter.
const DBL_NATIVE_SANITY_CEILING_US: f64 = 1000.0;

/// The fewest samples a distribution may be built from before this rung will grade on it.
///
/// ⊘ **`UNMEASURED`, not `FAIL`, below this.** A median over a dozen samples is a number,
/// not a measurement, and this tree has already paid for *"a count and a total cannot
/// recover a distribution"*. Fifty is enough for a stable median and a meaningful p90, and
/// is reached inside the wall budget even at 70 ms per doorbell (5 s ⇒ ~71 samples) — so the
/// **broken** case still produces a graded red rather than an `UNMEASURED`.
const DBL_MIN_SAMPLES: usize = 50;

/// How many submissions are outstanding before the loop drains the channel.
///
/// ⊘ **Not a tuning knob — a correctness bound.** The GPFIFO this crate allocates has 64
/// entries and the pushbuffer 32 slots, and nothing here waits for retirement inside the
/// timed region. Submitting past the smaller of those without draining would overwrite a
/// slot the engine has not read yet, and the copies the controls check would be landing out
/// of a ring that had been rewritten underneath them. 16 is half the pushbuffer.
///
/// ★ **The drain is OUTSIDE the measured region**, and that is stated in the printed header
/// rather than left to be discovered: the region is exactly one `submit_copy_at`.
const DBL_DRAIN_EVERY: usize = 16;

/// How long one drain window waits for the last submission in it to retire.
///
/// ⊘ **It is an OBSERVABLE, not a timeout to be tuned away.** `[measured 2026-09-06, Mode-2
/// guest]` the drain is where this rung found its result: three runs out of three, the
/// windows ending at submissions 15, 31 and 47 retired and the one ending at **63** did not.
/// A shorter timeout would have produced more samples and hidden the fact; a longer one
/// would have burnt the wall budget. What makes it evidence rather than a confound is that
/// the rung **counts the timeouts and names the submission the first one happened at**.
const DBL_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2000);

/// A latency distribution, in **microseconds**, with the count it was built from.
///
/// ⊘ **Five order statistics and a total, never a mean.** The failure this rung gates on is
/// a tail that dominates an aggregate; a mean is exactly the summary that hides it, and this
/// tree has a recorded instance of a count plus a total being read as a distribution when it
/// cannot recover one.
#[derive(Debug, Clone, Copy, Default)]
struct DblDist {
    /// How many submissions the numbers below are over.
    n: usize,
    /// The fastest sample.
    min_us: f64,
    /// The **median**, and the statistic the verdict is taken on. See [`dbl_verdict_note`].
    p50_us: f64,
    /// The 90th percentile — reported so a tail regression is visible without being the
    /// verdict.
    p90_us: f64,
    /// The slowest sample. ⊘ Never graded: one scheduler preemption would redden a healthy
    /// run, and a rung that flakes is a rung people stop reading.
    max_us: f64,
    /// What the whole loop cost, so *"n × p50"* can be checked against the wall clock.
    total_ms: f64,
    /// ★ Whether the wall budget ended the loop before `n_max` was reached. A truncated run
    /// is still a valid distribution; a run that does not SAY it was truncated is not.
    truncated: bool,
    /// How many submissions in the loop were **refused**. ⊘ A refusal is not a fast
    /// submission and is excluded from the samples, so its count has to be printed beside
    /// them or a channel that died halfway would read as a healthy, quick one.
    refused: usize,
}

impl DblDist {
    /// Build the distribution from raw nanosecond samples. `us` is taken by value because
    /// it is sorted in place, and sorting the caller's vector behind its back is the kind of
    /// surprise that makes a second use of the same samples silently different.
    fn of(mut ns: Vec<u64>, truncated: bool, refused: usize, wall: std::time::Duration) -> Self {
        ns.sort_unstable();
        let n = ns.len();
        if n == 0 {
            return DblDist {
                truncated,
                refused,
                total_ms: wall.as_secs_f64() * 1e3,
                ..DblDist::default()
            };
        }
        // ⊘ Nearest-rank, not interpolated: an interpolated quantile invents a value no
        // submission ever took, and every number this rung prints is meant to be one that
        // actually happened.
        let at = |q: f64| -> f64 {
            let idx = ((q * n as f64).ceil() as usize)
                .saturating_sub(1)
                .min(n - 1);
            ns[idx] as f64 / 1e3
        };
        DblDist {
            n,
            min_us: ns[0] as f64 / 1e3,
            p50_us: at(0.50),
            p90_us: at(0.90),
            max_us: ns[n - 1] as f64 / 1e3,
            total_ms: wall.as_secs_f64() * 1e3,
            truncated,
            refused,
        }
    }

    /// One machine-readable line. ⊘ `key=value` throughout and never a table, because the
    /// grader that reads this runs in `bash` and a column layout is a format a `sed` has to
    /// guess at.
    ///
    /// ⚠ **Three decimals, not one, and that is a measured decision.** `[measured 2026-09-06,
    /// bench kb]` the bare-doorbell arm's whole distribution renders as `0.0` at one decimal —
    /// a native doorbell store is well under 100 ns — and `0.0` reads as *"nothing was
    /// measured"* rather than as *"this is genuinely sub-microsecond"*. A format that cannot
    /// distinguish a real small number from an absent one is the same defect as an empty
    /// capture decoding to zeros.
    fn print(self, tag: &str) {
        println!(
            "DBL_DIST {tag} n={} min_us={:.3} p50_us={:.3} p90_us={:.3} max_us={:.3} \
             total_ms={:.1} truncated={} refused={}",
            self.n,
            self.min_us,
            self.p50_us,
            self.p90_us,
            self.max_us,
            self.total_ms,
            self.truncated,
            self.refused
        );
    }
}

/// How this rung was configured, so the header can print it and a grader can check that the
/// run it is reading is the run it asked for.
#[derive(Debug, Clone, Copy)]
struct DblCfg {
    /// Samples per repetition, before the budget is consulted.
    n_max: usize,
    /// The wall budget per repetition. ⊘ Its purpose is that a **broken** arm still returns
    /// in seconds: at 70 ms a doorbell, `n_max` would take minutes and the gate this rung
    /// exists to be would be as slow as the LLM boot it replaces.
    budget: std::time::Duration,
    /// How many independent repetitions the process runs.
    ///
    /// ⊘ **Within-process repetitions cannot see a per-boot lottery** — `submit_ms` has been
    /// measured at 9.1× across three consecutive boots of ONE build — so these are here for
    /// within-run stability only, and the SCRIPT runs the whole binary more than once. Both
    /// are needed and neither substitutes for the other.
    reps: usize,
    /// ★ The native median this arm is graded against, in microseconds, if the caller gave
    /// one. `None` ⇒ this run is the **calibration** and says so.
    native_p50_us: Option<f64>,
    /// See [`DBL_GATE_MULTIPLE`]. Overridable so a lane can widen or tighten the band with
    /// the number on its own log rather than in a rebuild.
    gate_multiple: f64,
}

impl Default for DblCfg {
    fn default() -> Self {
        DblCfg {
            n_max: 512,
            budget: std::time::Duration::from_millis(1500),
            reps: 3,
            native_p50_us: None,
            gate_multiple: DBL_GATE_MULTIPLE,
        }
    }
}

/// The one sentence that says which statistic decides, printed with the verdict so nobody
/// has to reconstruct it from the numbers.
fn dbl_verdict_note() -> &'static str {
    "⊘ THE VERDICT IS TAKEN ON THE MEDIAN. The failure being gated is an AGGREGATE — \
     thousands of doorbells at the typical price — and the median is the statistic that \
     aggregate is made of. `max` is printed and never graded (one scheduler preemption \
     would redden a healthy run); `p90` is printed so a tail regression is visible without \
     being the verdict"
}

/// ★★★★★ **w384 — THE DOORBELL-LATENCY RUNG.**
///
/// ```text
///   allocate a VAS, a channel at a dictated ring VA, schedule it
///   allocate one device-local object, poison it, map it at a dictated VA
///   CONTROL (open)  — one 4-byte LAUNCH_DMA into it must LAND        [POSITIVE CONTROL]
///   ARM S           — N x (submit_copy_at of 4 bytes), each one TIMED     [GRADED]
///   CONTROL (close) — another 4-byte LAUNCH_DMA must LAND            [POSITIVE CONTROL]
///   ARM D           — N x (bare doorbell, no new entry), TIMED         [PRINTED, UNGRADED]
///   ARM F           — N x (map a FRESH page, then one timed submit)    [PRINTED, UNGRADED]
/// ```
///
/// # ★★★ THE MEASURED REGION, STATED EXACTLY — because timing your own instrument is the
/// # standing trap here
///
/// For arm S the region is **exactly one `HostRmBackend::submit_copy_at` call** and nothing
/// else. Inside it: a handle narrow, one `HashMap` lookup for the channel's parts, the
/// pushbuffer encode (which allocates one small `Vec`), twelve-odd stores into the ring
/// object, two GPFIFO words, the `GP_PUT` store, two release fences and the doorbell store.
/// Outside it: the payload store that seeds the source word, the periodic drain, every
/// `println!`, and all statistics.
///
/// ★ **The `Vec` and the lookup are inside the region on BOTH arms, so they cancel in the
/// ratio the gate is taken on.** That is the reason the gate is a *multiple of a measured
/// native median* and not an absolute number: any cost that is arm-independent divides out.
///
/// ⚠ **`RmConnection::doorbell` prints two lines per store for its first 512 stores**, and
/// those lines are inside the region. This rung calls
/// [`kayfabe_isolate_host::rm::mute_doorbell_witness`] before the first sample and **prints
/// that it did**; without it the native floor would be the cost of a formatted write to
/// stderr, and every gate multiplied out of that floor would be uniformly too loose.
///
/// # ★★★ PRE-REGISTERED, BEFORE THE RUN — every outcome, so none reads as the favourable one
///
/// - **native**, controls pass, median under [`DBL_NATIVE_SANITY_CEILING_US`] ⇒ `PASS`, and
///   the run is the **calibration**: it prints `DBL_CALIBRATION_NATIVE_P50_US` for the guest
///   arm to be graded against. ⊘ A native `PASS` is NOT "native met a gate" — it cannot, the
///   gate would be derived from itself — and the printed verdict says so in words.
/// - **guest**, controls pass, `p50 <= native_p50 × multiple` ⇒ `PASS`. ★★★★★ **AND THAT
///   WOULD BE A FINDING, NOT A GREEN.** It would mean the 60–71 ms inline publication is not
///   on the raw client's doorbell path and the LLM's cost has been mis-attributed. The rung
///   prints that in full rather than filing it as a pass.
/// - **guest**, controls pass, `p50 > gate` ⇒ `FAIL`. The intended shape, and the handle the
///   async lane iterates against.
/// - **either control fails** ⇒ `NOTRUN`. The timed submissions went into a channel that was
///   not carrying work, and a distribution over refused or dead submissions is a number
///   about nothing. ⊘ NOT a failure value.
/// - **fewer than [`DBL_MIN_SAMPLES`] samples** ⇒ `NOTRUN`, for the same reason.
/// - **no native reference and this is not being read as a calibration** ⇒ the gate is
///   printed as `⊘ none` and the verdict falls back to the sanity ceiling, labelled.
fn doorbell_latency(rm: &mut HostRmBackend, gpu: u32, cfg: DblCfg) -> bool {
    const OFF_PRE: u64 = 0x00;
    const OFF_POST: u64 = 0x40;
    const OFF_LOOP: u64 = 0x80;

    println!(
        "info  R6 doorbell latency = GPU {gpu}, euid {} — what ONE doorbell costs the \
         SUBMITTING THREAD",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  R6 the bar          = the median cost of one `submit_copy_at`, against the \
         SAME act measured on bare metal by the SAME binary. ⊘ Not a benchmark: a \
         discriminator"
    );
    println!(
        "DBL_CFG n_max={} budget_ms={} reps={} gate_multiple={:.0} native_p50_us={}",
        cfg.n_max,
        cfg.budget.as_millis(),
        cfg.reps,
        cfg.gate_multiple,
        cfg.native_p50_us.map_or_else(
            || "⊘none(this run is the CALIBRATION)".to_string(),
            |v| format!("{v:.2}")
        ),
    );
    println!(
        "DBL_REGION = exactly one `submit_copy_at` (narrow, parts lookup, one small Vec in \
         the encoder, ~12 ring stores, 2 GPFIFO words, GP_PUT, 2 fences, the doorbell \
         store). OUTSIDE it: the source seed store, the every-{DBL_DRAIN_EVERY} drain, all \
         printing, all statistics"
    );
    // ⚠ Printed, never silent — see the function docs. A muted witness that a reader takes
    // for an unmuted one is the failure this line exists against.
    let already = kayfabe_isolate_host::rm::mute_doorbell_witness();
    println!(
        "DBL_WITNESS_MUTED=yes (the per-store doorbell witness had counted {already} stores; \
         its per-store lines are INSIDE the measured region and would BE the native floor. \
         ⊘ Refusals and the periodic tally still print)"
    );

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("??    R6 engine type      = COPY0 has no engine type on this ABI");
        println!("RUNGCTL_doorbell_latency=FAIL");
        println!("RUNG_doorbell_latency=NOTRUN");
        return false;
    };
    let Ok(vas) = rm.alloc_vaspace() else {
        println!("??    R6 vaspace          = the rung needs its own address space");
        println!("RUNGCTL_doorbell_latency=FAIL");
        println!("RUNG_doorbell_latency=NOTRUN");
        return false;
    };

    let mut chan_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut mem_h: Option<kayfabe_isolate::HostHandle> = None;
    let mut fresh: Vec<(u64, kayfabe_isolate::HostHandle)> = Vec::new();
    let mut mapped_target = false;
    let mut control_ok = false;
    // ★★★★★ **A THIRD STATE, AND ITS ABSENCE WAS A REAL DEFECT — caught by the grader's own
    // negative control, 2026-09-06.** The sample-floor branch printed *"UNMEASURED, and NOT a
    // failure value"* in prose and then returned `false`, which — with the positive control
    // PASSING — reached the `else` arm and emitted `RUNG_doorbell_latency=FAIL`. ⇒ **the
    // anchored machine-readable line said the OPPOSITE of the sentence above it**, and a
    // grader reads the anchored line. `control_ok` alone cannot express this: the control DID
    // pass, and there is still nothing to grade.
    let mut unmeasured = false;

    // ⊘ A closure so every early return still reaches the teardown below it. The rung
    // allocates a channel, an object and up to `n` fresh objects; leaking them would make a
    // LATER rung in the same process fail for this rung's reason.
    let mut go = || -> bool {
        let Ok((chan, token)) = rm.alloc_channel_at(vas, engine_type, Some(GpuVa(DBL_RING_AT)))
        else {
            println!("??    R6 channel          = refused at {DBL_RING_AT:#018x} — NOT a result");
            return false;
        };
        chan_h = Some(chan);
        if rm.channel_ring_va(chan) != Some(DBL_RING_AT) {
            println!("??    R6 ring placement    = RM did not place the ring where asked");
            return false;
        }
        if rm.schedule(chan).is_err() {
            println!("??    R6 schedule          = refused");
            return false;
        }
        let Ok(mem) = rm.alloc_probe_local(W379_BYTES) else {
            println!("??    R6 object           = device-local allocation refused");
            return false;
        };
        mem_h = Some(mem);
        if rm.fill_words(mem, W379_BYTES, DBL_SENTINEL, 0).is_err() {
            println!("??    R6 sentinel         = could not be written");
            return false;
        }
        match rm.map_local_at(vas, mem, W379_BYTES, Some(DBL_TARGET_AT)) {
            Ok(got) if got == DBL_TARGET_AT => mapped_target = true,
            Ok(got) => {
                mapped_target = true;
                println!(
                    "??    R6 placement        = asked {DBL_TARGET_AT:#018x}, RM chose \
                     {got:#018x} — the rung's addresses are not the ones it grades on"
                );
                return false;
            }
            Err(e) => {
                println!("??    R6 map             = refused {e:?}");
                return false;
            }
        }

        // ── THE OPENING CONTROL ──────────────────────────────────────────────────────────
        //
        // ⊘ Before a single sample is taken. A distribution over submissions that were never
        // carrying work is a number about nothing, and *"the loop was fast"* is exactly what
        // a dead channel looks like.
        let ch = W381Chan { h: chan, token };
        let pre = w379_release_through(
            rm,
            W381Probe::LaunchDma,
            ch,
            mem,
            DBL_TARGET_AT,
            OFF_PRE,
            DBL_MAGIC_PRE,
        );
        println!("info  R6 control (open)   = {pre:?}");
        if !pre.landed() {
            println!(
                "??    R6 CONTROL FAILED    = a four-byte LAUNCH_DMA into the rung's own \
                 mapped object did not land. ⊘ Every number below would be the cost of \
                 submitting into a channel that carries nothing — UNINTERPRETABLE, and \
                 reported as NOTRUN rather than as a red"
            );
            return false;
        }
        control_ok = true;
        println!("ok    R6 control (open)   = the channel carries work; the loop can be believed");

        // ── ARM S — THE GRADED LOOP ──────────────────────────────────────────────────────
        let (src_off, sem_off) = kayfabe_isolate_host::rm::HostRmBackend::copy_probe_offsets();
        let mut pooled: Vec<u64> = Vec::with_capacity(cfg.n_max * cfg.reps);
        let mut pooled_refused = 0usize;
        let mut pooled_wall = std::time::Duration::ZERO;
        let mut pooled_trunc = false;
        let mut rep_p50: Vec<f64> = Vec::with_capacity(cfg.reps);
        let mut stalled_reps = 0usize;
        let mut pooled_drains = 0usize;
        let mut pooled_drain_timeouts = 0usize;
        let mut pooled_first_stall: Option<usize> = None;

        for rep in 0..cfg.reps {
            // ⚠ Capacity taken UP FRONT: a `Vec` growing inside the loop would reallocate
            // between two samples and put a `memcpy` of the samples so far inside one of
            // them. The push itself stays outside the timed region regardless.
            let mut ns: Vec<u64> = Vec::with_capacity(cfg.n_max);
            let mut refused = 0usize;
            let started = std::time::Instant::now();
            let mut truncated = false;
            // ★★★ THE DRAIN, INSTRUMENTED. Before this it was a silent 2 s per window that
            // ate the wall budget and made a STALLED channel look merely like a short
            // sample — the confound and the finding wearing one face.
            let mut drains = 0usize;
            let mut drain_timeouts = 0usize;
            let mut drain_wall = std::time::Duration::ZERO;
            let mut first_stall: Option<usize> = None;
            for i in 0..cfg.n_max {
                if started.elapsed() >= cfg.budget {
                    truncated = true;
                    break;
                }
                // OUTSIDE the region: seed the source word this submission copies. A
                // distinct value per iteration so a landed copy names ITS OWN submission.
                let payload = 0x8400_0000u32
                    .wrapping_add((rep as u32) << 16)
                    .wrapping_add(i as u32);
                if rm.ring_store_u32(chan, src_off, payload).is_err() {
                    refused += 1;
                    continue;
                }

                // ══ THE MEASURED REGION — one call, nothing else ══════════════════════
                let t0 = std::time::Instant::now();
                let r =
                    rm.submit_copy_at(chan, token, src_off, DBL_TARGET_AT + OFF_LOOP, 4, payload);
                let dt = t0.elapsed();
                // ══ END OF THE MEASURED REGION ════════════════════════════════════════

                let submitted = r.is_ok();
                if submitted {
                    ns.push(dt.as_nanos() as u64);
                } else {
                    // ⊘ A refusal is NOT a fast submission. It is excluded from the samples
                    // and counted separately, so a channel that dies halfway cannot read as
                    // a healthy quick one.
                    refused += 1;
                }

                // OUTSIDE the region, and it is a CORRECTNESS bound, not a courtesy — see
                // `DBL_DRAIN_EVERY`. The ring has 32 pushbuffer slots and nothing here waits
                // for retirement; submitting past that without draining would rewrite a slot
                // the engine has not read.
                //
                // ⊘ Gated on `submitted`, and that is not tidiness: waiting for a semaphore
                // to reach a payload whose submission was REFUSED spins the whole deadline
                // every window, and the two seconds it burns would be charged to nothing —
                // a refused channel would read as a slow one rather than as a refused one.
                if submitted && (i + 1).is_multiple_of(DBL_DRAIN_EVERY) {
                    let d0 = std::time::Instant::now();
                    let deadline = d0 + DBL_DRAIN_TIMEOUT;
                    let mut retired = false;
                    loop {
                        if matches!(rm.ring_load_u32(chan, sem_off), Ok(v) if v == payload) {
                            retired = true;
                            break;
                        }
                        if std::time::Instant::now() >= deadline {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_micros(200));
                    }
                    drains += 1;
                    drain_wall += d0.elapsed();
                    if !retired {
                        drain_timeouts += 1;
                        first_stall = Some(i);
                        // ⊘⊘ STOP THE REPETITION HERE, and this is not an optimisation.
                        // Submissions issued after the channel has stopped retiring are a
                        // DIFFERENT quantity — the cost of composing a push into a ring
                        // nothing is draining — and pooling them with the others under one
                        // name is how a bimodal number gets reported as one median.
                        break;
                    }
                }
            }
            let wall = started.elapsed();
            pooled.extend_from_slice(&ns);
            let d = DblDist::of(ns, truncated, refused, wall);
            d.print(&format!("arm=submit rep={rep}"));
            println!(
                "DBL_DRAIN rep={rep} every={DBL_DRAIN_EVERY} drains={drains} \
                 timeouts={drain_timeouts} drain_ms={:.1} stalled={} first_stall_at={}",
                drain_wall.as_secs_f64() * 1e3,
                first_stall.is_some(),
                first_stall.map_or_else(|| "none".to_string(), |i| i.to_string()),
            );
            if first_stall.is_some() {
                stalled_reps += 1;
                if pooled_first_stall.is_none() {
                    pooled_first_stall = first_stall;
                }
            }
            pooled_drains += drains;
            pooled_drain_timeouts += drain_timeouts;
            if d.n > 0 {
                rep_p50.push(d.p50_us);
            }
            pooled_refused += refused;
            pooled_wall += wall;
            pooled_trunc |= truncated;
        }

        let s = DblDist::of(pooled, pooled_trunc, pooled_refused, pooled_wall);
        s.print("arm=submit rep=POOLED");
        println!(
            "DBL_REP_MEDIANS us=[{}]  ⊘ within-process reps CANNOT see a per-boot lottery \
             (`submit_ms` measured 9.1x across three consecutive boots of ONE build); they \
             show within-run stability only, and the SCRIPT runs this binary more than once",
            rep_p50
                .iter()
                .map(|v| format!("{v:.1}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        // ★★★★★ **THE POOLED STALL LINE — this is where the guest result actually lives.**
        // A channel that stops retiring is not slow, it is BROKEN, and the two are reported
        // by completely different numbers. `first_stall_at` is the submission index whose
        // drain window failed to retire, and it is the bisection answer WITHOUT a second boot.
        println!(
            "DBL_STALL reps={} stalled_reps={stalled_reps} drains={pooled_drains} \
             timeouts={pooled_drain_timeouts} first_stall_at={}",
            cfg.reps,
            pooled_first_stall.map_or_else(|| "none".to_string(), |i| i.to_string()),
        );

        // ── THE CLOSING CONTROL ──────────────────────────────────────────────────────────
        //
        // ★ It is the half that makes the loop attributable: the opening control proves the
        // channel worked BEFORE the loop, and only this one proves it still did AFTER it.
        // A loop that killed its own channel on submission 3 and then measured 500 cheap
        // refusals would pass the opening control alone.
        let post = w379_release_through(
            rm,
            W381Probe::LaunchDma,
            ch,
            mem,
            DBL_TARGET_AT,
            OFF_POST,
            DBL_MAGIC_POST,
        );
        println!("info  R6 control (close)  = {post:?}");
        if post.landed() {
            println!("ok    R6 control (close)  = the channel still carried work at the end");
        } else {
            control_ok = false;
            println!(
                "??    R6 CONTROL FAILED    = the channel no longer carries work AFTER the \
                 timed loop. ⊘ The distribution above is over submissions whose fate is \
                 unknown — NOTRUN, not a red"
            );
            // ⊘⊘⊘ **AND IT DOES NOT RETURN HERE, WHICH IT USED TO.** The two ungraded arms
            // below are DIAGNOSIS, and diagnosis is most needed exactly when something has
            // failed. Returning early skipped them on the one arm that had a result to
            // explain — *"a diagnostic gated on the failure"*, in this file, on its first
            // guest run. They are cheap, bounded by the same wall budget, and a `bare`
            // doorbell that still succeeds against a channel whose copies have stopped is a
            // fact worth having.
            println!(
                "⊘     R6 POST-MORTEM      = the two ungraded arms below run ANYWAY, against \
                 a channel now known to have stopped retiring. Read them as diagnosis of \
                 that state, never as a latency measurement"
            );
        }

        // ── ARM D — the bare doorbell, PRINTED AND UNGRADED ──────────────────────────────
        //
        // ⊘ Ungraded on purpose: a device is entitled to see that `GP_PUT` has not moved and
        // do nothing, and *"a no-op is fast"* is a finding about the no-op. What it IS for is
        // ATTRIBUTION of a red already measured on arm S — if a real submission is expensive
        // and this is cheap, the cost is in the ring stores or the planning; if this is
        // expensive too, the cost is in the trap.
        {
            let mut ns: Vec<u64> = Vec::with_capacity(cfg.n_max);
            let mut refused = 0usize;
            let started = std::time::Instant::now();
            let mut truncated = false;
            for _ in 0..cfg.n_max {
                if started.elapsed() >= cfg.budget {
                    truncated = true;
                    break;
                }
                let t0 = std::time::Instant::now();
                let r = rm.ring_doorbell_only(token);
                let dt = t0.elapsed();
                if r.is_ok() {
                    ns.push(dt.as_nanos() as u64);
                } else {
                    refused += 1;
                }
            }
            let wall = started.elapsed();
            DblDist::of(ns, truncated, refused, wall).print("arm=bare rep=0 ⊘UNGRADED");
        }

        // ── ARM F — a FRESH mapping behind every ring, PRINTED AND UNGRADED ──────────────
        //
        // ★★★ **This arm exists to make a green on arm S interpretable.** If publication is
        // incremental, a loop that rings the same channel at the same address N times may pay
        // the price once and be cheap for the rest — and would then pass while a real
        // workload, which maps as it goes, does not. Here every ring is preceded by a FRESH
        // 64 KiB object at a FRESH VA, so there is always something unpublished behind it.
        //
        // ⊘ Ungraded, because its cost legitimately includes an RM allocation and a map per
        // iteration on BOTH arms and the ratio between arms is not the same quantity arm S's
        // gate is about. It is DIAGNOSIS, printed beside the verdict, never part of it.
        {
            // ⚠ A tenth of arm S's count: each iteration is an alloc plus a map, and the
            // budget is spent on RM rather than on the doorbell.
            let n_fresh = (cfg.n_max / 8).max(8);
            let mut ns: Vec<u64> = Vec::with_capacity(n_fresh);
            let mut refused = 0usize;
            let started = std::time::Instant::now();
            let mut truncated = false;
            for i in 0..n_fresh {
                if started.elapsed() >= cfg.budget {
                    truncated = true;
                    break;
                }
                let at = DBL_FRESH_BASE + (i as u64) * DBL_FRESH_STRIDE;
                let Ok(obj) = rm.alloc_probe_local(W379_BYTES) else {
                    refused += 1;
                    continue;
                };
                if !matches!(rm.map_local_at(vas, obj, W379_BYTES, Some(at)), Ok(g) if g == at) {
                    refused += 1;
                    let _ = rm.free(obj);
                    continue;
                }
                fresh.push((at, obj));
                let payload = 0x8401_0000u32.wrapping_add(i as u32);
                if rm.ring_store_u32(chan, src_off, payload).is_err() {
                    refused += 1;
                    continue;
                }
                // ══ THE MEASURED REGION — the same one call as arm S ══════════════════
                let t0 = std::time::Instant::now();
                let r = rm.submit_copy_at(chan, token, src_off, at, 4, payload);
                let dt = t0.elapsed();
                // ══ END ═══════════════════════════════════════════════════════════════
                if r.is_ok() {
                    ns.push(dt.as_nanos() as u64);
                } else {
                    refused += 1;
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while !matches!(rm.ring_load_u32(chan, sem_off), Ok(v) if v == payload)
                    && std::time::Instant::now() < deadline
                {
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
            }
            let wall = started.elapsed();
            DblDist::of(ns, truncated, refused, wall).print("arm=freshmap rep=0 ⊘UNGRADED");
        }

        // ⊘ NO GATE IS APPLIED WHEN THE CLOSING CONTROL FAILED. A `PASS` or a `FAIL`
        // computed over submissions into a channel that stopped retiring would be a
        // confident number about a broken thing — and `PASS` is the one it would usually
        // be, because the submissions that never executed are the CHEAP ones.
        if !control_ok {
            println!(
                "⊘     R6 NO VERDICT       = the closing control failed, so the distribution \
                 above is NOT graded. See `DBL_STALL` for what actually happened; the \
                 machine-readable verdict is NOTRUN"
            );
            return false;
        }

        // ── THE VERDICT ──────────────────────────────────────────────────────────────────
        //
        // ★★★★★ **THE SAMPLE FLOOR HAS AN ESCAPE HATCH, AND WITHOUT IT THE RUNG IS BACKWARDS
        // ON EXACTLY THE CASE IT EXISTS FOR.** The wall budget is what makes this a
        // five-second gate; the consequence is that the WORSE the arm is, the FEWER samples
        // it produces. `[measured, LLM boot]` `worst_trap=1750538us` — at that price a 1.5 s
        // repetition yields ONE sample, three repetitions yield three, and a rule of *"under
        // fifty samples ⇒ UNMEASURED"* would report the most catastrophic possible result as
        // *"we could not tell"*. ⇒ A catastrophically slow arm would be the one arm the rung
        // could never grade.
        //
        // ⊘ The hatch is deliberately narrow and it is **sound rather than lenient**: it does
        // not lower the bar, it uses a statistic that needs no sample size. If the FASTEST
        // submission observed is already over the gate, then no median over any number of
        // further samples could be under it — every sample is at least the minimum, by
        // definition. So *"n is too small for a median"* and *"the answer is determinate
        // anyway"* are simultaneously true, and the rung says both.
        let floor_ref = cfg
            .native_p50_us
            .map_or(DBL_NATIVE_SANITY_CEILING_US, |b| b * cfg.gate_multiple);
        if s.n < DBL_MIN_SAMPLES {
            if s.n > 0 && s.min_us > floor_ref {
                println!(
                    "FAIL  R6 VERDICT (n={})  = ★ DETERMINATE DESPITE THE SAMPLE COUNT. Only \
                     {} submissions fit inside the wall budget — which is itself the symptom \
                     — but the FASTEST of them took {:.1}us against a gate of {floor_ref:.1}us. \
                     ⊘ No median over any number of further samples can be below a value \
                     every sample already exceeds, so the small n does NOT make this \
                     uninterpretable",
                    s.n, s.n, s.min_us
                );
                println!("DBL_MEASURED_P50_US={:.2}", s.p50_us);
                println!("DBL_GATE_US={floor_ref:.2}");
                println!("DBL_ROLE=GRADED_ON_MIN");
                return false;
            }
            // ⊘ NOT a red: too few samples AND not determinate on the minimum. See
            // `unmeasured`'s declaration for the defect this flag exists because of.
            unmeasured = true;
            println!(
                "??    R6 SAMPLES          = {} graded samples, floor is {DBL_MIN_SAMPLES}, and \
                 the fastest ({:.1}us) is NOT above the gate ({floor_ref:.1}us) — so the \
                 answer is not determinate either. ⊘ A median over a handful is a number, \
                 not a measurement: UNMEASURED, and NOT a failure value",
                s.n, s.min_us
            );
            return false;
        }
        println!("DBL_MEASURED_P50_US={:.2}", s.p50_us);
        println!("{}", dbl_verdict_note());

        match cfg.native_p50_us {
            Some(base) => {
                let gate = base * cfg.gate_multiple;
                println!("DBL_ROLE=GRADED");
                println!(
                    "DBL_NATIVE_P50_US={base:.2} DBL_GATE_MULTIPLE={:.0} DBL_GATE_US={gate:.2}",
                    cfg.gate_multiple
                );
                // ★★ THE CONTINUOUS NUMBER, beside the binary one. A pass/fail alone tells an
                // iterating lane nothing about whether it moved: two builds can both be red
                // and be a factor of forty apart. This is the quantity to graph — and it is
                // deliberately printed for BOTH outcomes, because a lane that only records
                // its ratio when it fails cannot tell a fix from a lucky boot.
                println!("DBL_RATIO_X={:.1}", s.p50_us / base);
                if s.p50_us <= gate {
                    println!(
                        "★     R6 VERDICT         = p50 {:.1}us <= gate {gate:.1}us \
                         ({:.1}x the native floor).",
                        s.p50_us,
                        s.p50_us / base
                    );
                    println!(
                        "⚠     R6 READ THIS       = ★★★★★ IF THIS IS THE GUEST ARM, A PASS IS \
                         A FINDING AND NOT A GREEN. It would mean the 60-71ms inline \
                         publication measured on the LLM boot is NOT on the raw client's \
                         doorbell path, and the LLM's cost has been mis-attributed. Compare \
                         `arm=freshmap` against `arm=submit` before concluding anything: if \
                         freshmap is the expensive one, the cost is in PUBLISHING NEW ROWS \
                         and this loop simply had nothing left to publish"
                    );
                    true
                } else {
                    println!(
                        "FAIL  R6 VERDICT         = p50 {:.1}us > gate {gate:.1}us — {:.0}x \
                         the native floor for the SAME act by the SAME binary",
                        s.p50_us,
                        s.p50_us / base
                    );
                    false
                }
            }
            None => {
                // ⊘ No external floor ⇒ this run cannot be graded against one, and saying
                // otherwise would be grading against a number derived from itself.
                println!("DBL_ROLE=CALIBRATION");
                println!("DBL_CALIBRATION_NATIVE_P50_US={:.2}", s.p50_us);
                println!(
                    "DBL_GATE_FOR_THE_OTHER_ARM_US={:.2} (= p50 x {:.0})",
                    s.p50_us * cfg.gate_multiple,
                    cfg.gate_multiple
                );
                if s.p50_us <= DBL_NATIVE_SANITY_CEILING_US {
                    println!(
                        "★     R6 VERDICT         = PASS as a CALIBRATION: the controls held \
                         and p50 {:.1}us is under the {DBL_NATIVE_SANITY_CEILING_US:.0}us \
                         sanity ceiling, so this number is usable as the other arm's floor. \
                         ⊘ THIS IS NOT \"NATIVE MET A GATE\" — a calibration cannot fail a \
                         gate derived from itself, and this rung does not pretend it can",
                        s.p50_us
                    );
                    true
                } else {
                    println!(
                        "FAIL  R6 VERDICT         = p50 {:.1}us is ABOVE the \
                         {DBL_NATIVE_SANITY_CEILING_US:.0}us sanity ceiling. Nothing in a \
                         fence plus a store into a mapped window legitimately medians that \
                         high, so the box is contended or the path is not what we think — \
                         and this number MUST NOT be used as anyone's floor",
                        s.p50_us
                    );
                    false
                }
            }
        }
    };

    let verdict = go();

    // ── teardown, on every path ─────────────────────────────────────────────────────────
    for (at, obj) in fresh.drain(..) {
        let _ = rm.unmap_local(vas, at);
        let _ = rm.free(obj);
    }
    if mapped_target {
        let _ = rm.unmap_local(vas, DBL_TARGET_AT);
    }
    if let Some(h) = mem_h {
        let _ = rm.free(h);
    }
    if let Some(h) = chan_h {
        let _ = rm.free(h);
    }
    let _ = rm.free(vas);

    println!(
        "RUNGCTL_doorbell_latency={}",
        if control_ok { "PASS" } else { "FAIL" }
    );
    // ⊘ THREE OUTCOMES AND THEY ARE NOT ORDERED BY SEVERITY. `NOTRUN` covers two different
    // ways of having nothing to say — the control did not pass, or it did and the loop
    // produced no gradeable distribution — and neither is a failure value. Folding either
    // into `FAIL` reports a finding that was never measured.
    if !control_ok || unmeasured {
        println!("RUNG_doorbell_latency=NOTRUN");
    } else if verdict {
        println!("RUNG_doorbell_latency=PASS");
    } else {
        println!("RUNG_doorbell_latency=FAIL");
    }
    verdict
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// w289 — `--defer-liveness`: DOES A FRESHLY-WRITTEN PTE GO LIVE WITHOUT AN INVALIDATE?
// ═══════════════════════════════════════════════════════════════════════════════════════

/// Where this rung's channel rings live — 320 GiB, an octave away from every other rung's
/// addresses so nothing here can be covered by a mapping some earlier rung left behind.
const DL_RING_BASE: u64 = 0x0000_0050_0000_0000;
/// Where the address under test lives — 384 GiB.
const DL_VA_BASE: u64 = 0x0000_0060_0000_0000;
/// Per-arm stride, 8 GiB. Each arm gets its own address space *and* its own addresses, so
/// an arm can never inherit a page table, a reservation or a cached translation from the
/// arm before it.
const DL_ARM_STRIDE: u64 = 0x0000_0002_0000_0000;
/// Per-channel ring stride, 1 GiB. Three channels per arm, none of them sharing a page
/// directory with the address under test.
const DL_CHAN_STRIDE: u64 = 0x0000_0000_4000_0000;
/// Where the copy destination sits, relative to an arm's base: 4 GiB above it, i.e. under a
/// different page directory entirely. ⊘ Deliberately NOT inside the span under test — a
/// destination in the same leaf table would be a second reason the leaf table exists, and
/// the whole point of the guard below is that there is exactly one.
const DL_SCRATCH_OFF: u64 = 0x0000_0001_0000_0000;

/// The VA span one page-directory-0 entry covers on this MMU regime: 2 MiB.
///
/// `crates/kayfabe-chips/src/ga10x.rs:542` — *"`PD0`, bits 28:21 — a 16-byte dual entry, and
/// also a 2 MiB leaf"*. Bits 28:21 is `2^21` of VA per entry. The guard mapping and the
/// address under test are placed inside ONE of these, which is what makes them share a leaf
/// page table by construction rather than by hope.
///
/// ⚠ That same line names the hazard this rung has to avoid: PD0 is **also** a 2 MiB leaf,
/// so a 2 MiB mapping at a 2 MiB-aligned VA can become a single huge PTE at the directory
/// level. [`DL_MAP_BYTES`] and [`kayfabe_abi::bringup::NVOS46_FLAGS_PAGE_SIZE_4KB`] are what
/// keep the entry under test an entry in a leaf table.
const DL_PDE0_SPAN: u64 = 0x0020_0000;
/// The data page — 2 MiB of device-local memory.
const DL_DATA_BYTES: u64 = 0x0020_0000;
/// How much of it is mapped at the address under test: 64 KiB.
///
/// ⊘ **Not the whole object, and the difference is load-bearing.** A 2 MiB mapping at a
/// 2 MiB-aligned VA is exactly what `_dmaGetPageSize` is entitled to promote to a single
/// huge PTE, which lives at the DIRECTORY level — so a run that mapped the whole object
/// could answer a leaf-level question at a level that is not the leaf, and would look
/// identical on the log.
const DL_MAP_BYTES: u64 = 0x0001_0000;
/// The guard mapping's length, 64 KiB. Same size and same aperture as the data mapping so
/// RM has no reason to choose a different page size for the two.
const DL_GUARD_BYTES: u64 = 0x0001_0000;
/// The copy destination's length, 64 KiB.
const DL_SCRATCH_BYTES: u64 = 0x0001_0000;
/// One page of notifier per channel.
const DL_NOTIFIER_BYTES: u64 = 0x1000;

/// The first word of the data page. A copy that lands moves exactly this.
const DL_DATA_MAGIC: u32 = 0x0DEF_0289;
/// The first word of the guard page — the channel control's payload. Distinct from
/// [`DL_DATA_MAGIC`], so *"the control landed"* and *"the test landed"* can never be read
/// off the same word.
const DL_GUARD_MAGIC: u32 = 0x6A2D_0289;
/// What the destination holds before every copy. Neither 0 nor 1: *"not the magic"* has to
/// mean *"nothing wrote here"* rather than *"we cannot tell"*.
const DL_SENTINEL: u32 = 0xDEAD_0289;
/// The payload each copy releases on its own channel semaphore — a retirement **qualifier**
/// only, read exclusively to say which KIND of red a red is. ⊘ It never turns a red green.
const DL_RETIRE: u32 = 0x8928_9289;
/// How long a copy is given to land before it is called lost.
const DL_LAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Which of the three arms is running.
///
/// ⊘ Three, not two. The DEFER arm on its own cannot distinguish *"the deferred PTE is not
/// live"* from *"nothing in this rig maps anything at all"*, and the invalidate arm on its
/// own cannot distinguish *"the invalidate made it live"* from *"the map would have worked
/// anyway"*. Each arm is the other's control and all three run in one invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeferArm {
    /// ★ **THE RIG CONTROL.** An ordinary map — RM invalidates for itself at the `done:`
    /// gate — followed by the same copy. It must be `Live`. If it is not, the prime's fault
    /// took the test channel with it, or the addresses are wrong, or the copy verb is
    /// broken, and neither of the other two arms says anything.
    Plain,
    /// ★★★★★ **THE QUESTION.** `DEFER_TLB_INVALIDATION = TRUE`, and no invalidate by any
    /// transport afterwards.
    Defer,
    /// ★★★ **THE KNOWN-POSITIVE.** Identical to [`DeferArm::Defer`] up to and including the
    /// map, then `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` before the copy. It must be `Live`.
    DeferThenInvalidate,
}

impl DeferArm {
    /// The printed form, so a log line, a grader and a human agree on the vocabulary.
    fn as_str(self) -> &'static str {
        match self {
            DeferArm::Plain => "PLAIN(rig-control)",
            DeferArm::Defer => "DEFER(the-question)",
            DeferArm::DeferThenInvalidate => "DEFER+INVALIDATE(known-positive)",
        }
    }

    /// Whether the map carries `NVOS46_FLAGS_DEFER_TLB_INVALIDATION`.
    fn defers(self) -> bool {
        !matches!(self, DeferArm::Plain)
    }

    /// Whether this arm issues an explicit invalidate between the map and the copy.
    fn invalidates(self) -> bool {
        matches!(self, DeferArm::DeferThenInvalidate)
    }
}

/// What a copy through the address under test did.
///
/// ⊘ [`DeferLive::NotRun`] is a first-class value and is never folded into `Dead`. *"The
/// experiment did not run"* and *"the experiment ran and the address did not resolve"* are
/// the difference between a broken rig and a finding, and a rung that reported the first as
/// the second would manufacture exactly the answer it is looking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeferLive {
    /// ★ The data word arrived at the destination.
    Live,
    /// The data word never arrived. `saw` is what the destination actually held.
    Dead {
        /// The word the destination held when the deadline expired.
        saw: u32,
    },
    /// ⊘ Never asked. The string says why, and it is printed rather than swallowed.
    NotRun(&'static str),
}

impl DeferLive {
    /// The printed form.
    fn as_str(self) -> &'static str {
        match self {
            DeferLive::Live => "LIVE",
            DeferLive::Dead { .. } => "DEAD",
            DeferLive::NotRun(_) => "NOTRUN",
        }
    }
}

/// What the host driver says about the leaf PTE at the address under test, after the map.
///
/// ⊘ Four values because [`DeferPte::Unmeasured`] is not [`DeferPte::Absent`].
/// `NV0080_CTRL_CMD_DMA_GET_PTE_INFO` carries `RMCTRL_FLAGS_RM_TEST_ONLY_CODE` in its nvoc
/// flags (`0x100008`, `ogkm-580: src/nvidia/generated/g_device_nvoc.c:733-745`) and a
/// release driver refuses it with `NV_ERR_TEST_ONLY_CODE_NOT_ENABLED` (`0x7E`) for every
/// address, including known-mapped ones — see
/// [`kayfabe_abi::submit::NV0080_CTRL_CMD_DMA_GET_PDE_INFO`]. Reading that refusal as
/// *"there is no PTE"* would attribute a later fault to a map that in fact succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeferPte {
    /// ★ RM holds a valid PTE at this VA, at `page_size` bytes.
    Valid {
        /// The page size RM reports for the entry.
        page_size: u64,
    },
    /// The control answered and said no valid PTE covers the VA.
    Absent,
    /// ⊘ The control refused. **Not an answer.** Carries RM's status.
    Unmeasured(String),
}

impl DeferPte {
    /// The printed form. ⚠ `UNMEASURED` prints the status it was refused with, because
    /// *"refused"* without the number is the shape that makes a test-only refusal
    /// indistinguishable from a wrong handle.
    fn as_str(&self) -> String {
        match self {
            DeferPte::Valid { page_size } => format!("PTE_VALID(page_size={page_size:#x})"),
            DeferPte::Absent => "PTE_ABSENT".to_string(),
            DeferPte::Unmeasured(why) => format!("PTE_UNMEASURED({why})"),
        }
    }
}

/// Everything one arm established, so the grader reads a value rather than re-deriving it
/// from the log.
#[derive(Debug)]
struct DeferReport {
    /// Which arm this is.
    arm: DeferArm,
    /// ★★★ Did the prime provoke a fault? `None` means the notifier could not be read at
    /// all, which is a third state and not a `false`.
    prime_faulted: Option<bool>,
    /// The `ROBUST_CHANNEL_*` exception type the prime's notifier carried, if it fired.
    prime_except: Option<u32>,
    /// Did the test channel still work *after* the prime's fault, witnessed by a copy through
    /// the guard VA? If this is false, nothing later in the arm is interpretable.
    chan_survived: bool,
    /// Whether the map path was pinned to the small page size, or fell back to RM's choice.
    /// ⚠ Printed and carried rather than assumed: if RM refused `PAGE_SIZE_4KB` the level
    /// under test is RM's pick, and *"this is about the leaf"* stops being a statement this
    /// rung can make.
    page_size_4kb: bool,
    /// What the page-directory oracle said about the address under test **before the guard
    /// mapping existed**. ★ Expected `PdeAbsent`: it is what makes the guard, and nothing
    /// else, the thing that instantiated the leaf table.
    pde_before_guard: W379HostVa,
    /// Whether a page table already covered the address under test **before** the map — the
    /// assertion that the map could not have allocated a page level, and therefore could not
    /// have reached `gvaspaceAlloc`'s sparse branch or any other invalidate.
    pde_before_map: W379HostVa,
    /// What RM says about the leaf PTE after the map.
    pte_after_map: DeferPte,
    /// The address RM reported the mapping at, and whether it is the one that was asked for.
    placed: Option<u64>,
    /// The invalidate's own status, when the arm issued one.
    invalidate: Option<Result<(), String>>,
    /// The verdict: did the copy through the address under test move the data word?
    live: DeferLive,
    /// ★★★ For the DEFER arm only, and only when it came back `Dead`: invalidate, then try
    /// again on a third channel. A `Live` here says the PTE was in memory all along and the
    /// only missing thing was the invalidate — which is what makes a `Dead` attributable.
    rescue: Option<DeferLive>,
}

/// Render an [`RmError`] as the thing the owner's rule demands on every refusal: **RM's own
/// status**, not our variant name.
///
/// ⊘ `ioctl(2)` returning `0` is not *"nothing refused"* — RM puts its status **inside the
/// parameter struct**, and `status_check` is what turns it into an error here. So every
/// refusal this rung prints carries the number RM sent, and the two statuses this port
/// deliberately folds together say so rather than picking one.
fn dl_why(e: &RmError) -> String {
    match e {
        RmError::Other(s) => format!("rmStatus={s:#06x}"),
        RmError::InsufficientPermissions => {
            "rmStatus=0x1b NV_ERR_INSUFFICIENT_PERMISSIONS".to_string()
        }
        RmError::NoMemory => "rmStatus=0x1a-or-0x51 ⊘ this port folds NV_ERR_INSUFFICIENT_\
             RESOURCES and NV_ERR_NO_MEMORY into ONE variant, so which of the two RM sent is \
             not recoverable here. ⚠ On a FIXED map 0x51 is RM saying the VA is ALREADY \
             mapped, which is a success in the C's reading and a failure in ours"
            .to_string(),
        other => format!("{other:?} — a port-side refusal, not an RM status"),
    }
}

/// Copy four bytes `src_va -> dst_va` on `ch` and wait for [`DL_DATA_MAGIC`]-class `want` to
/// arrive at word 0 of `dst`, read back through an **independent** CPU mapping.
///
/// ⊘ The destination is re-poisoned with [`DL_SENTINEL`] **before** every submission, so
/// *"the word we want is still there"* can never be a leftover from the previous copy. The
/// poison is a CPU store into an object nothing on the GPU is touching, so it introduces no
/// page-table work and no invalidate of its own.
///
/// ⊘ And the read-back is [`HostRmBackend::read_words_independently`] — a fresh device node
/// and a fresh mapping — for `w379_release_through`'s reason: reading through the mapping
/// the sentinel was written through proves a page is writable and nothing else.
fn dl_copy(
    rm: &mut HostRmBackend,
    ch: W381Chan,
    dst: kayfabe_isolate::HostHandle,
    src_va: u64,
    dst_va: u64,
    want: u32,
) -> DeferLive {
    if rm
        .fill_words(dst, DL_SCRATCH_BYTES, DL_SENTINEL, 0)
        .is_err()
    {
        return DeferLive::NotRun("the destination could not be poisoned");
    }
    if rm
        .submit_copy_va(ch.h, ch.token, src_va, dst_va, 4, DL_RETIRE)
        .is_err()
    {
        // ⊘ A refused SUBMISSION is not a dead address: the engine was never asked.
        return DeferLive::NotRun("the copy submission was refused");
    }
    let deadline = std::time::Instant::now() + DL_LAND_TIMEOUT;
    loop {
        let saw = match rm.read_words_independently(dst, DL_SCRATCH_BYTES, &[0]) {
            Ok(w) => w[0],
            Err(_) => return DeferLive::NotRun("the destination could not be read back"),
        };
        if saw == want {
            return DeferLive::Live;
        }
        if std::time::Instant::now() >= deadline {
            return DeferLive::Dead { saw };
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// The retirement **qualifier** for one of this rung's copies: did the channel's own
/// semaphore take [`DL_RETIRE`]?
///
/// ⊘⊘ **ORDER THE OBSERVABLES.** Read only when the primary observable — the word in the
/// destination — is already a red, and only ever to say WHICH KIND of red it is. It may
/// never turn a red green.
///
/// ⚠ **Deliberately not [`w381_retired`], and this is not a style choice.** That helper
/// compares the semaphore against [`W381_RETIRE_PAYLOAD`], which is a different constant
/// from the one this rung's copies release, so it would answer
/// `NOT-RETIRED(sem holds something else)` for every copy here that retired perfectly — a
/// confident, plausible, always-wrong qualifier of exactly the kind its own doc-comment
/// warns about, arriving through reuse rather than through a typo.
fn dl_retired(rm: &HostRmBackend, chan: kayfabe_isolate::HostHandle) -> &'static str {
    let (_src_off, sem_off) = kayfabe_isolate_host::rm::HostRmBackend::copy_probe_offsets();
    match rm.ring_load_u32(chan, sem_off) {
        Ok(v) if v == DL_RETIRE => "RETIRED",
        Ok(0) => "NOT-RETIRED(sem still 0)",
        Ok(_) => "NOT-RETIRED(sem holds something else)",
        Err(_) => "UNMEASURED(ring read refused)",
    }
}

/// Ask RM about the **leaf PTE** at `va`, and report the refusal as a refusal.
fn dl_pte(rm: &mut HostRmBackend, vas: kayfabe_isolate::HostHandle, va: u64) -> DeferPte {
    match rm.pte_info(vas, va) {
        Ok(Some(b)) => DeferPte::Valid {
            page_size: b.page_size,
        },
        Ok(None) => DeferPte::Absent,
        Err(e) => DeferPte::Unmeasured(dl_why(&e)),
    }
}

/// ★★ **CONFOUNDER 3, AS A CHECK RATHER THAN AS A SENTENCE IN A REPORT.**
///
/// Any global invalidate anywhere on this GPU — issued by *any* process, for *any* reason —
/// clears the primed state this whole rung rests on. So the box must be otherwise idle, and
/// *"it probably was"* is not a thing a later reader can check.
///
/// This walks `/proc/*/fd` and reports every **other** process holding a `/dev/nvidia*`
/// descriptor open. ⊘ It returns a `Result`, not a `Vec`: on a kernel or a container where
/// `/proc` cannot be walked the honest answer is *"not measured"*, and a silent empty list
/// would read as *"the box is idle"* — which is the `dlen=0` failure in a new place.
fn dl_gpu_users() -> Result<Vec<u32>, String> {
    let me = std::process::id();
    let dir = std::fs::read_dir("/proc").map_err(|e| format!("/proc unreadable: {e}"))?;
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == me {
            continue;
        }
        // ⊘ A pid whose `fd` directory cannot be read is SKIPPED, not counted as clean:
        // that is a process we could not see into, and the summary line says how many.
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            // ⊘ An fd we cannot resolve is SKIPPED rather than counted either way: it is a
            // descriptor we could not see, and pretending it is not a GPU node would be the
            // same manufactured quiet as an unreadable `/proc`.
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            if target.to_string_lossy().starts_with("/dev/nvidia") {
                out.push(pid);
                break;
            }
        }
    }
    out.sort_unstable();
    Ok(out)
}

/// One arm's addresses, derived once from its slot so no call site does the arithmetic
/// twice. ⊘ The guard and the address under test are computed from ONE base and are
/// asserted to sit inside one [`DL_PDE0_SPAN`] — the property the whole rung rests on, as a
/// value rather than as a comment.
#[derive(Debug, Clone, Copy)]
struct DlPlan {
    /// The 2 MiB-aligned base of the span under test. The **guard** mapping goes here.
    vbase: u64,
    /// ★★★★★ **THE ADDRESS UNDER TEST.** Inside the same [`DL_PDE0_SPAN`] as `vbase`, so
    /// the guard's page table is the table this address's leaf entry lives in.
    v: u64,
    /// Where the copy destination is mapped — 4 GiB away, under a different page directory.
    scratch_at: u64,
    /// The three channel rings: prime, test, rescue.
    rings: [u64; 3],
}

impl DlPlan {
    /// Derive an arm's addresses from its slot.
    fn for_slot(slot: u64) -> DlPlan {
        let vbase = DL_VA_BASE + slot * DL_ARM_STRIDE;
        DlPlan {
            vbase,
            // ⊘ `+ DL_GUARD_BYTES`, i.e. immediately past the guard and still 2 MiB - 128 KiB
            // short of the span's end. Adjacent-but-disjoint is what puts the two mappings in
            // one leaf table without letting either cover the other.
            v: vbase + DL_GUARD_BYTES,
            scratch_at: vbase + DL_SCRATCH_OFF,
            rings: [
                DL_RING_BASE + slot * DL_ARM_STRIDE,
                DL_RING_BASE + slot * DL_ARM_STRIDE + DL_CHAN_STRIDE,
                DL_RING_BASE + slot * DL_ARM_STRIDE + 2 * DL_CHAN_STRIDE,
            ],
        }
    }

    /// Whether the guard and the address under test really are inside one page-directory
    /// entry's span. Checked rather than trusted: the constants above make it true, and a
    /// future edit to any one of them makes it false silently.
    fn one_leaf_table(&self) -> bool {
        self.vbase.is_multiple_of(DL_PDE0_SPAN)
            && self.v >= self.vbase
            && self.v + DL_MAP_BYTES <= self.vbase + DL_PDE0_SPAN
    }

    /// Whether every address this arm names sits under RM's default `FERMI_VASPACE_A` limit
    /// ([`W385_VAS_LIMIT`]).
    ///
    /// ⊘ Checked rather than assumed, and refused **by its own name**: an address past the
    /// ceiling comes back as a refused fixed map, which reads as *"RM would not place our
    /// mapping"* — a statement about the deferral, made by a rung that asked for an address
    /// that does not exist. The same failure w385 gives `WINDOW_ABOVE_VAS_LIMIT` its own
    /// violation for.
    fn fits_the_vaspace(&self) -> bool {
        [
            self.vbase + DL_PDE0_SPAN,
            self.v + DL_MAP_BYTES,
            self.scratch_at + DL_SCRATCH_BYTES,
            self.rings[2] + DL_CHAN_STRIDE,
        ]
        .iter()
        .all(|&hi| hi <= W385_VAS_LIMIT)
    }
}

/// One arm, end to end. Everything it establishes goes into `rep`; everything it allocates
/// goes into `handles` so the caller can dispose of it whatever happens here.
///
/// Returns `None` the moment the arm stops being interpretable — and prints why, on the
/// same line, every time.
fn dl_arm_body(
    rm: &mut HostRmBackend,
    vas: kayfabe_isolate::HostHandle,
    arm: DeferArm,
    plan: &DlPlan,
    rep: &mut DeferReport,
    handles: &mut Vec<kayfabe_isolate::HostHandle>,
) -> Option<()> {
    use kayfabe_isolate_host::rm::NotifierAperture;

    let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
        println!("FAIL  DL engine          = COPY0 is not expressible");
        return None;
    };

    // ── the objects. None of them is mapped yet; the fills are CPU stores. ──────────────
    let (Ok(guard), Ok(data), Ok(scratch)) = (
        rm.alloc_probe_local(DL_GUARD_BYTES),
        rm.alloc_probe_local(DL_DATA_BYTES),
        rm.alloc_probe_local(DL_SCRATCH_BYTES),
    ) else {
        println!("FAIL  DL objects         = the arm needs a guard, a data page and a destination");
        return None;
    };
    handles.push(guard);
    handles.push(data);
    handles.push(scratch);
    if rm
        .fill_words(guard, DL_GUARD_BYTES, DL_GUARD_MAGIC, 0)
        .is_err()
        || rm
            .fill_words(data, DL_DATA_BYTES, DL_DATA_MAGIC, 0)
            .is_err()
        || rm
            .fill_words(scratch, DL_SCRATCH_BYTES, DL_SENTINEL, 0)
            .is_err()
    {
        println!("FAIL  DL fill            = an object could not be seeded through the CPU");
        return None;
    }
    println!(
        "ok    DL objects         = guard {DL_GUARD_BYTES:#x} @ {:#018x} magic {DL_GUARD_MAGIC:#010x}, \
         data {DL_DATA_BYTES:#x} (mapping {DL_MAP_BYTES:#x}) magic {DL_DATA_MAGIC:#010x}, \
         dst {DL_SCRATCH_BYTES:#x} @ {:#018x} sentinel {DL_SENTINEL:#010x}",
        plan.vbase, plan.scratch_at
    );

    // ── THREE CHANNELS, ALL BUILT BEFORE THE PRIME, AND THAT ORDER IS THE EXPERIMENT ────
    //
    // ★★★ Creating a channel maps its ring, its USERD and RM's own context buffers into
    // THIS address space, and every one of those maps ends at the `done:` gate that
    // invalidates (`ogkm-580: virt_mem_allocator_gm107.c:2610-2615`). A channel created
    // between the prime and the copy would therefore erase the primed state and the rung
    // would measure a first-fill — the exact thing the prime exists to rule out. So the
    // prime channel (which the fault kills), the test channel and the rescue channel are
    // all minted here, before anything is primed.
    //
    // ⊘ Each gets its OWN notifier object. Sharing one would make a second channel's RC
    // record overwrite the first's, and *"the notifier fired"* would stop naming who faulted.
    let mut chans: Vec<(W381Chan, kayfabe_isolate::HostHandle)> = Vec::new();
    for (i, ring_at) in plan.rings.iter().enumerate() {
        let Ok(notifier) = rm.alloc_sysmem(DL_NOTIFIER_BYTES) else {
            println!("FAIL  DL notifier {i}      = refused");
            return None;
        };
        handles.push(notifier);
        let (h, token) = match rm.alloc_channel_at_with_error_notifier(
            vas,
            engine_type,
            Some(GpuVa(*ring_at)),
            notifier,
        ) {
            Ok(v) => v,
            Err(e) => {
                println!(
                    "FAIL  DL channel {i}       = refused at ring {ring_at:#018x} — {}",
                    dl_why(&e)
                );
                return None;
            }
        };
        handles.push(h);
        if let Err(e) = rm.schedule(h) {
            println!("FAIL  DL schedule {i}      = {}", dl_why(&e));
            return None;
        }
        chans.push((W381Chan { h, token }, notifier));
    }
    println!(
        "ok    DL channels        = 3 built BEFORE the prime (prime/test/rescue), rings at \
         {:#018x} {:#018x} {:#018x} — ⊘ none is created after it, because a channel's own \
         ctx-buffer maps each end in an invalidate",
        plan.rings[0], plan.rings[1], plan.rings[2]
    );

    // ── STEP 4 — ALL PAGE-LEVEL INSTANTIATION HAPPENS HERE, AND IT IS ASSERTED ──────────
    //
    // ★★ The **guard** mapping is the instantiator. It covers the low 64 KiB of the same
    // 2 MiB span the address under test sits in, so it forces the whole level chain down to
    // the leaf page table into existence and then HOLDS it — it is never unmapped inside the
    // arm, so `_gvaspaceReleaseUnreservedPTEs` cannot take the table away again.
    //
    // ⊘ Why not `NVOS46_FLAGS_DMA_UNICAST_REUSE_ALLOC` and a pre-reserved VA range, which
    // would be the textbook way to make step 6 write nothing but a leaf PTE? Because
    // `NV01_MEMORY_VIRTUAL` **reserves no VA at all** — `virtmemConstruct_IMPL` returns
    // early for that class before any allocation (`ogkm-580:
    // src/nvidia/src/kernel/mem_mgr/virtual_mem.c:350-352`), so its `offset`/`limit` bound
    // where RM may place a mapping and reserve nothing. A real reservation needs
    // `NV50_MEMORY_VIRTUAL` and `NV_MEMORY_ALLOCATION_PARAMS`, which this client does not
    // speak. The guard buys the same property and, unlike the reservation, it is CHECKED
    // below by an oracle that is not ours.
    //
    // ⊘⊘ And the confounder as originally stated is narrower than it looks, from source:
    // the VA allocation the map path performs (`vaspaceAlloc`, reached because
    // `DMA_UNICAST_REUSE_ALLOC` is FALSE by default, `virt_mem_allocator_gm107.c:419,1028`)
    // lands in `gvaspaceAlloc`'s *"Pin page tables upfront"* branch — `mmuWalkReserveEntries`
    // and **no invalidate anywhere in it** (`ogkm-580: gpu_vaspace.c:1639-1682`). The branch
    // that DOES invalidate is the **sparse** one (`:1581-1608`), which needs `flags.bSparse`,
    // which nothing on the map path sets. `gvaspaceIncAllocRefCnt` only bumps a refcount
    // (`:1945-1961`). ⇒ The residual hazard is a *page-directory write* the prime's walk
    // could have cached above the leaf, and that is what the guard removes.
    let pde_virgin = w379_host_va(rm, vas, plan.v);
    rep.pde_before_guard = pde_virgin;
    println!(
        "info  DL PDE before guard= {} @ {:#018x} — ★ expect PDE_ABSENT: it is what makes the \
         guard, and not something else, the thing that instantiated the leaf table",
        pde_virgin.as_str(),
        plan.v
    );

    // ★ The page-size flag is CALIBRATED on the guard map and then used for every map in the
    // arm. Pinning the small page size is what keeps this a question about the LEAF; if RM
    // refuses it we fall back and say so, rather than silently answering a different
    // question at a different level.
    let mut ps4k = true;
    let guard_va =
        match rm.map_local_at_with_flags(vas, guard, DL_GUARD_BYTES, Some(plan.vbase), false, true)
        {
            Ok(v) => v,
            Err(e) => {
                println!(
                    "??    DL guard 4KB        = refused — {} ⇒ retrying with PAGE_SIZE_DEFAULT. \
                 ⚠ The level under test is then RM's choice, not ours",
                    dl_why(&e)
                );
                ps4k = false;
                match rm.map_local_at_with_flags(
                    vas,
                    guard,
                    DL_GUARD_BYTES,
                    Some(plan.vbase),
                    false,
                    false,
                ) {
                    Ok(v) => v,
                    Err(e2) => {
                        println!("FAIL  DL guard map       = {}", dl_why(&e2));
                        return None;
                    }
                }
            }
        };
    rep.page_size_4kb = ps4k;
    if guard_va != plan.vbase {
        println!(
            "??    DL guard placement  = asked {:#018x}, RM reported {guard_va:#018x}. ⊘ The \
             guard is not covering the span under test, so it instantiates the wrong table",
            plan.vbase
        );
        return None;
    }
    println!(
        "ok    DL guard map       = {DL_GUARD_BYTES:#x} @ {guard_va:#018x}, page-size flag {} \
         — HELD for the whole arm",
        if ps4k { "4KB" } else { "DEFAULT" }
    );

    let scratch_va = match rm.map_local_at_with_flags(
        vas,
        scratch,
        DL_SCRATCH_BYTES,
        Some(plan.scratch_at),
        false,
        ps4k,
    ) {
        Ok(v) if v == plan.scratch_at => v,
        Ok(v) => {
            println!(
                "??    DL dst placement    = asked {:#018x}, got {v:#018x}",
                plan.scratch_at
            );
            return None;
        }
        Err(e) => {
            println!("FAIL  DL dst map         = {}", dl_why(&e));
            return None;
        }
    };
    println!("ok    DL dst map         = {DL_SCRATCH_BYTES:#x} @ {scratch_va:#018x}");

    // ★★★ THE ASSERTION CONFOUNDER 2 ASKS FOR. A page table covers the address under test
    // BEFORE the map, so the map cannot be the thing that allocates one.
    rep.pde_before_map = w379_host_va(rm, vas, plan.v);
    match rep.pde_before_map {
        W379HostVa::PdeCovers => println!(
            "★     DL PDE before map  = PDE_COVERS @ {:#018x} — the leaf table EXISTS, so step \
             6 writes a leaf PTE and allocates no level",
            plan.v
        ),
        W379HostVa::PdeAbsent => {
            println!(
                "FAIL  DL PDE before map  = PDE_ABSENT @ {:#018x} — the guard did NOT instantiate \
                 the table covering the address under test, so step 6 would allocate a page \
                 level and this arm cannot exclude an invalidate it did not ask for",
                plan.v
            );
            return None;
        }
        W379HostVa::Unmeasured => println!(
            "⊘     DL PDE before map  = UNMEASURED — the page-directory oracle refused, so \
             CONFOUNDER-2 IS UNASSERTED for this arm. ⚠ Note the asymmetry, which the grader \
             uses: an un-excluded level allocation could only have caused an EXTRA invalidate, \
             so it can manufacture verdict A and can never manufacture verdict B"
        ),
    }

    // ── STEP 5 — ★★★★★ THE PRIME. THIS IS THE WHOLE EXPERIMENT. ────────────────────────
    //
    // A run that maps with DEFER, touches, and works proves nothing: *"no entry was ever
    // cached for this VA"* and *"a fresh walk picks the new entry up"* are physically the
    // same state. So the MMU is made to walk the address under test **while it is invalid**,
    // FIRST. What the final copy then tests is eviction of a cached non-present result, not
    // a first fill.
    //
    // ⚠ The fault is non-replayable, so the channel that issued it is killed by the host RC
    // path. That is expected and is why this is a channel of its own.
    println!(
        "FAULT_MARK=defer_liveness_prime arm={} va={:#018x} ⚠ THE HOST dmesg WILL CARRY AN Xid \
         31 FOR THIS CHANNEL AND IT IS THE POINT",
        arm.as_str(),
        plan.v
    );
    let primed = dl_copy(rm, chans[0].0, scratch, plan.v, scratch_va, DL_DATA_MAGIC);
    println!("info  DL prime copy      = {primed:?}");
    if matches!(primed, DeferLive::Live) {
        println!(
            "??    DL PRIME LANDED    = a copy out of {:#018x} SUCCEEDED before anything was \
             mapped there. ⊘ The address was not invalid, so nothing was primed and this arm \
             is UNINTERPRETABLE",
            plan.v
        );
        return None;
    }
    match rm.read_error_notifier(chans[0].1, NotifierAperture::Sysmem) {
        Ok(n) => {
            rep.prime_faulted = Some(n.fired());
            rep.prime_except = Some(n.except_type);
            println!(
                "info  DL prime notifier  = fired={} status={:#06x} except_type={:#x} engine={:#06x}",
                n.fired(),
                n.status,
                n.except_type,
                n.engine_type
            );
        }
        Err(e) => {
            // ⊘ THIRD STATE. "We could not read the notifier" is not "it did not fire", and
            // collapsing them would let an unreadable instrument pass as a fault.
            rep.prime_faulted = None;
            println!(
                "⊘     DL prime notifier  = UNREADABLE — {} ⇒ whether the walk faulted is \
                 UNMEASURED",
                dl_why(&e)
            );
        }
    }
    if rep.prime_faulted != Some(true) {
        println!(
            "PRIME_DID_NOT_FAULT arm={} va={:#018x} retire={} — ⊘ the MMU is not known to have \
             walked the address under test, so the final copy would be testing FIRST FILL and \
             not eviction. NO VERDICT IS REPORTED FOR THIS ARM",
            arm.as_str(),
            plan.v,
            dl_retired(rm, chans[0].0.h)
        );
        return None;
    }
    println!(
        "★     DL PRIME           = FAULTED. The MMU walked {:#018x} and the entry it found \
         was non-present",
        plan.v
    );

    // ── THE CHANNEL CONTROL — both surviving channels, before anything is mapped at V ───
    //
    // ★★★ Without this, *"the copy at step 9 did not land"* is indistinguishable from *"the
    // prime's engine reset took the other channels with it"*, and the second is a broken rig
    // reported as a finding. Each channel copies out of the **guard** VA — mapped, known
    // good, and holding a different magic — so a landing here says the channel works and
    // says nothing about the address under test.
    //
    // ⚠ It is a walk in the primed address space, so it does put the guard's translation in
    // the TLB. That is a constant across all three arms and cannot produce a differential;
    // an uncontrolled dead channel can, and does, and has.
    let ctl_test = dl_copy(
        rm,
        chans[1].0,
        scratch,
        guard_va,
        scratch_va,
        DL_GUARD_MAGIC,
    );
    let ctl_rescue = dl_copy(
        rm,
        chans[2].0,
        scratch,
        guard_va,
        scratch_va,
        DL_GUARD_MAGIC,
    );
    rep.chan_survived =
        matches!(ctl_test, DeferLive::Live) && matches!(ctl_rescue, DeferLive::Live);
    println!("info  DL channel control = test {ctl_test:?}, rescue {ctl_rescue:?}");
    if !rep.chan_survived {
        println!(
            "??    DL CONTROL FAILED  = a channel that never faulted cannot land a copy through \
             a mapping that has been live since before the prime. ⊘ The prime's recovery took \
             more than its own channel; NOTHING BELOW WOULD BE ATTRIBUTABLE and the arm stops \
             here"
        );
        return None;
    }
    println!("ok    DL channel control = both surviving channels still land after the prime");

    // ── STEP 6 — THE MAP ───────────────────────────────────────────────────────────────
    match rm.map_local_at_with_flags(vas, data, DL_MAP_BYTES, Some(plan.v), arm.defers(), ps4k) {
        Ok(got) => {
            rep.placed = Some(got);
            println!(
                "ok    DL map             = {DL_MAP_BYTES:#x} @ {got:#018x} asked {:#018x} \
                 DEFER={} PAGE_SIZE={}",
                plan.v,
                arm.defers(),
                if ps4k { "4KB" } else { "DEFAULT" }
            );
            if got != plan.v {
                println!(
                    "??    DL map placement   = RM placed the mapping elsewhere, so the primed \
                     address and the mapped address are different addresses"
                );
                return None;
            }
        }
        Err(e) => {
            println!(
                "FAIL  DL map             = refused with DEFER={} — {}",
                arm.defers(),
                dl_why(&e)
            );
            return None;
        }
    }

    // ── STEP 7 — CONFIRM THE PTE IS VALID IN MEMORY ────────────────────────────────────
    //
    // ⊘ Never skipped, and its refusal is never silent: if we cannot establish that the map
    // actually wrote a valid entry, a dead copy at step 9 is unattributable — it could be a
    // TLB that was never told, or a map that never happened.
    rep.pte_after_map = dl_pte(rm, vas, plan.v);
    let pde_after = w379_host_va(rm, vas, plan.v);
    println!(
        "info  DL PTE after map   = {} ; PDE after map = {}",
        rep.pte_after_map.as_str(),
        pde_after.as_str()
    );
    match &rep.pte_after_map {
        DeferPte::Valid { .. } => println!(
            "★     DL PTE            = VALID IN MEMORY. A dead copy below is therefore about \
             the TLB and not about the map"
        ),
        DeferPte::Absent => {
            println!(
                "??    DL PTE            = RM reports NO valid PTE at {:#018x} after a map it \
                 accepted. ⊘ Nothing below would be attributable to the deferral",
                plan.v
            );
            return None;
        }
        DeferPte::Unmeasured(why) => println!(
            "⊘     DL PTE            = UNMEASURED ({why}) — expected on a release driver: \
             GET_PTE_INFO carries RMCTRL_FLAGS_RM_TEST_ONLY_CODE. ⚠ FALLBACK IN FORCE: the \
             PDE answer above is PAGE-TABLE granularity and does NOT say the leaf is valid, \
             so the attribution moves to the RESCUE at step 10 — invalidate, retry, and a \
             landing there proves the PTE was in memory all along"
        ),
    }

    // ── STEP 8 — THE INVALIDATE, OR DELIBERATELY NOTHING AT ALL ────────────────────────
    if arm.invalidates() {
        let out = rm.invalidate_tlb(vas);
        match &out {
            Ok(()) => println!(
                "ok    DL invalidate      = NV2080_CTRL_CMD_DMA_INVALIDATE_TLB issued on the \
                 subdevice, hVASpace = this arm's space, rmStatus=0x0000"
            ),
            Err(e) => println!("FAIL  DL invalidate      = {}", dl_why(e)),
        }
        rep.invalidate = Some(out.map_err(|e| dl_why(&e)));
        if rep.invalidate.as_ref().is_some_and(Result::is_err) {
            println!(
                "??    DL KNOWN-POSITIVE  = the invalidate transport itself was refused, so this \
                 arm cannot serve as the control"
            );
            return None;
        }
    } else {
        println!(
            "info  DL invalidate      = NONE ISSUED BY ANY TRANSPORT — no control, no \
             pushbuffer MEM_OP, nothing. This is the arm's defining property"
        );
    }

    // ── STEP 9 — THE COPY THROUGH THE FRESHLY-WRITTEN PTE ──────────────────────────────
    rep.live = dl_copy(rm, chans[1].0, scratch, plan.v, scratch_va, DL_DATA_MAGIC);
    println!(
        "info  DL test copy       = {:?} retire={} ",
        rep.live,
        dl_retired(rm, chans[1].0.h)
    );
    if let Ok(n) = rm.read_error_notifier(chans[1].1, NotifierAperture::Sysmem) {
        println!(
            "info  DL test notifier   = fired={} status={:#06x} except_type={:#x}",
            n.fired(),
            n.status,
            n.except_type
        );
    }

    // ── STEP 10 — THE RESCUE. Only for the arm that withheld the invalidate, and only ──
    // when it came back dead.
    //
    // ★★★★★ This is what turns a red into an attributed red, and it is stronger than the
    // PTE oracle that a release driver refuses: invalidate, then copy again on a channel
    // that has never faulted. A landing says the entry was in the page table the whole time
    // and the ONLY thing missing was the invalidate — which is verdict B with its mechanism
    // named. A second dead copy says the deferred map produced no usable entry at all, and
    // then this arm is a statement about our own map and not about the hardware's TLB.
    if arm == DeferArm::Defer && matches!(rep.live, DeferLive::Dead { .. }) {
        println!(
            "info  DL rescue          = the deferred copy was DEAD; issuing the invalidate now \
             and retrying on the third channel, to say WHICH red this is"
        );
        match rm.invalidate_tlb(vas) {
            Ok(()) => println!("ok    DL rescue invalidate = issued, rmStatus=0x0000"),
            Err(e) => {
                println!("FAIL  DL rescue invalidate = {}", dl_why(&e));
                rep.rescue = Some(DeferLive::NotRun("the rescue invalidate was refused"));
                return Some(());
            }
        }
        // ⊘ The rescue channel is re-controlled HERE, not only before the prime: the test
        // copy above may itself have faulted, and an engine reset between then and now would
        // make a dead rescue read as "the PTE was never written".
        let ctl2 = dl_copy(
            rm,
            chans[2].0,
            scratch,
            guard_va,
            scratch_va,
            DL_GUARD_MAGIC,
        );
        if !matches!(ctl2, DeferLive::Live) {
            println!(
                "??    DL rescue control  = {ctl2:?} — the rescue channel did not survive the \
                 test copy, so the rescue would measure the channel and not the PTE"
            );
            rep.rescue = Some(DeferLive::NotRun("the rescue channel did not survive"));
            return Some(());
        }
        let out = dl_copy(rm, chans[2].0, scratch, plan.v, scratch_va, DL_DATA_MAGIC);
        println!(
            "info  DL rescue copy     = {out:?} retire={}",
            dl_retired(rm, chans[2].0.h)
        );
        rep.rescue = Some(out);
    }
    Some(())
}

impl DeferReport {
    /// An arm that has established nothing yet. ⊘ Every "did it happen" field starts in the
    /// state that means *"we did not get that far"*, never in the state that means *"no"*.
    fn new(arm: DeferArm) -> DeferReport {
        DeferReport {
            arm,
            prime_faulted: None,
            prime_except: None,
            chan_survived: false,
            page_size_4kb: false,
            pde_before_guard: W379HostVa::Unmeasured,
            pde_before_map: W379HostVa::Unmeasured,
            pte_after_map: DeferPte::Unmeasured("the arm never reached the map".to_string()),
            placed: None,
            invalidate: None,
            live: DeferLive::NotRun("the arm never reached the copy"),
            rescue: None,
        }
    }
}

/// Run one arm in a **fresh** address space, then dispose of everything it built.
///
/// ★ The address space is per-arm and not per-rung. A `FERMI_VASPACE_A` that an earlier arm
/// mapped into, faulted in, and invalidated carries state no later arm can account for —
/// and the whole rung is a statement about cached translations, so *"nothing unrelated is
/// cached in here"* has to be true by construction rather than by argument.
fn defer_arm(rm: &mut HostRmBackend, arm: DeferArm, slot: u64) -> DeferReport {
    let plan = DlPlan::for_slot(slot);
    let mut rep = DeferReport::new(arm);
    println!();
    println!("═══ DL ARM {} (slot {slot}) ═══", arm.as_str());
    println!(
        "info  DL plan            = guard {:#018x}  V {:#018x}  dst {:#018x}  rings \
         {:#018x}/{:#018x}/{:#018x}",
        plan.vbase, plan.v, plan.scratch_at, plan.rings[0], plan.rings[1], plan.rings[2]
    );
    if !plan.one_leaf_table() {
        println!(
            "FAIL  DL geometry        = the guard and the address under test are NOT inside one \
             {DL_PDE0_SPAN:#x} span, so they would not share a leaf page table and the guard \
             would instantiate the wrong one"
        );
        return rep;
    }
    if !plan.fits_the_vaspace() {
        println!(
            "FAIL  DL geometry        = an address this arm names is past {W385_VAS_LIMIT:#018x}, \
             RM's default address-space limit. ⊘ Refused by name: a fixed map RM declines \
             because the address does not exist would otherwise read as a statement about the \
             deferral"
        );
        return rep;
    }
    println!(
        "ok    DL geometry        = guard and V share ONE {DL_PDE0_SPAN:#x} directory span (so \
         they share a leaf page table by construction), and every address is under \
         {W385_VAS_LIMIT:#018x}"
    );
    let vas = match rm.alloc_vaspace() {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL  DL vaspace         = {}", dl_why(&e));
            return rep;
        }
    };
    println!("ok    DL vaspace         = a FRESH address space for this arm alone");

    let mut handles: Vec<kayfabe_isolate::HostHandle> = Vec::new();
    let _ = dl_arm_body(rm, vas, arm, &plan, &mut rep, &mut handles);

    // ⊘ Teardown runs whatever the body decided, and the three addresses are unmapped by
    // NAME rather than from a list the body kept: a list the body appends to is a list an
    // early return can leave short, and an unmap of something that was never mapped is a
    // refusal we can ignore.
    for va in [plan.vbase, plan.v, plan.scratch_at] {
        let _ = rm.unmap_local(vas, va);
    }
    for h in handles.into_iter().rev() {
        let _ = rm.free(h);
    }
    let _ = rm.free(vas);
    rep
}

/// ★★★★★ **w289 — DOES A FRESHLY-WRITTEN PTE BECOME LIVE WITHOUT A TLB INVALIDATE?**
///
/// `NVOS46_FLAGS_DEFER_TLB_INVALIDATION` (bit 31, `ogkm-580:
/// src/common/sdk/nvidia/inc/nvos.h:2149-2151`) lets a client map memory and skip the
/// invalidate: with it set, `dmaAllocMapping_GM107` selects `DMA_DEFER_TLB_INVALIDATE`
/// (`virt_mem_allocator_gm107.c:417`) and the `kbusFlush_HAL` + `gvaspaceInvalidateTlb` at
/// the function's `done:` label (`:2610-2615`) is not taken. Two worlds follow, and they
/// select between two very different designs for this port:
///
/// ```text
///   (A)  a TLB miss walks and picks the fresh PTE up
///        => the deferred map WORKS
///        => a mapping can go live with NOTHING OBSERVABLE to a hypervisor
///
///   (B)  the fresh PTE is not live until an invalidate
///        => DEFER is a BATCHING contract
///        => either the invalidate arrives (observable) or the guest's own access faults
/// ```
///
/// # ⊘ SOURCE DOES NOT DECIDE IT, AND HERE IS BOTH HALVES
///
/// The SDK header documents the hazard as **stale entries** only (`nvos.h:2144-2148`) — an
/// unmap hazard — which leans (A). Against that, **both** RM and UVM invalidate on a fresh
/// upgrade anyway: `uvm_mmu.c:805-808` says *"Upgrades don't have to flush out accesses, so
/// no membar is needed on the TLB invalidate"* and then still issues `tlb_invalidate_all`,
/// which is dead work under (A). ⇒ It has to be asked of hardware.
///
/// # ★★★ THE PRIMING STEP IS THE WHOLE EXPERIMENT
///
/// The naive version — map with DEFER, touch, see if it works — **cannot distinguish A from
/// B**. If it passes, that means *"no entry was ever cached for this VA"*, which is
/// physically identical to *"fresh VA, the walk picks it up"*. So the address is **primed**:
/// a copy engine is pointed at it while it is invalid, it faults, and the MMU has now walked
/// it and may have cached the non-present result. The final copy then tests **eviction**,
/// not first fill. A prime that does not fault invalidates the arm, by name and out loud.
///
/// # The sequence, per arm
///
/// ```text
///   1  a FRESH FERMI_VASPACE_A, so no unrelated cached state exists in it
///   2  a 2 MiB device-local data page, a guard page, a destination, three notifiers
///   3  THREE copy-engine channels bound to that space, ALL created before the prime
///   4  the guard mapped at the base of the span  -> ALL page-level instantiation, here
///      + assert a page table now covers the address under test (PDE oracle)
///   5  PRIME: copy FROM the address under test -> faults, channel RCs, notifier names it
///      + a channel control: the other two channels still land through the guard VA
///   6  map the data page at the address under test, with DEFER (or without, arm 1)
///   7  confirm the leaf PTE is valid in memory  (GET_PTE_INFO; refusal handled, see below)
///   8  issue no invalidate at all (arm 2) / issue one (arm 3) / RM issued its own (arm 1)
///   9  copy FROM the address under test into the destination and poll
///  10  arm 2 only, and only if 9 was dead: invalidate, then retry on the third channel
/// ```
///
/// ⇒ **completes correctly = (A); faults or returns stale = (B).**
///
/// # ⊘ THE PTE ORACLE IS EXPECTED TO REFUSE, AND STEP 10 IS WHY THAT IS SURVIVABLE
///
/// `NV0080_CTRL_CMD_DMA_GET_PTE_INFO` carries `RMCTRL_FLAGS_RM_TEST_ONLY_CODE` in its nvoc
/// flags (`0x100008`, `ogkm-580: src/nvidia/generated/g_device_nvoc.c:733-745`) and a
/// release driver refuses it for **every** address. This rung asks anyway and prints the
/// refusal with RM's status, because *"we could not confirm the PTE"* and *"there is no
/// PTE"* are the difference between a finding and an unattributable fault. The fallback is
/// **step 10**, and it is stronger than the oracle it replaces: invalidating and retrying
/// turns *"dead"* into *"dead, and the entry was in the page table the whole time"*.
///
/// # ⚠ SCOPE, so this cannot be read as a general result
///
/// It tests the **leaf** level, through the small-page table, on ONE board of the GA10x MMU
/// regime, for a **read** access, with one copy engine, at one moment. It says nothing about
/// big or huge pages, about page-directory entries, about write accesses, about other
/// engines, or about any other MMU generation.
///
/// ⚠ And it needs the box to be otherwise idle: any global invalidate anywhere on this GPU
/// clears the primed state. The rung reports which other processes hold `/dev/nvidia*` open
/// rather than asserting the box was quiet.
fn defer_liveness(rm: &mut HostRmBackend, gpu: u32) -> bool {
    println!(
        "info  DL defer-liveness  = GPU {gpu}, euid {} — does a PTE written with \
         NVOS46_FLAGS_DEFER_TLB_INVALIDATION become live with NO invalidate on any transport?",
        kayfabe_linux_raw::geteuid()
    );
    println!(
        "info  DL the two worlds  = (A) the walk picks the fresh PTE up ⇒ a mapping can go \
         live with nothing observable to a hypervisor; (B) it is not live until an \
         invalidate ⇒ DEFER is a BATCHING contract and the boundary stays observable"
    );
    println!(
        "DEFER_SCOPE=leaf-PTE, small-page table, GA10x MMU regime, ONE board, VIRT_READ, one \
         copy engine — ⊘ NOT a statement about big/huge pages, about page-directory entries, \
         about writes, or about any other MMU generation"
    );

    // ★★ CONFOUNDER 3, reported rather than assumed. A global invalidate raised by ANY
    // process on this GPU clears the primed state, and a run that shared the board with a
    // CUDA job cannot tell verdict A from a neighbour's invalidate.
    match dl_gpu_users() {
        Ok(pids) if pids.is_empty() => println!(
            "ok    DL idle check      = no other process holds a /dev/nvidia* descriptor open"
        ),
        Ok(pids) => println!(
            "??    DL IDLE CHECK      = {} OTHER process(es) hold /dev/nvidia* open: {pids:?}. \
             ⚠ ANY global invalidate they raise clears the primed state, which can only \
             manufacture verdict A. Re-run on an idle box before believing an A",
            pids.len()
        ),
        Err(why) => println!(
            "⊘     DL idle check      = UNMEASURED ({why}) — ⚠ this is NOT 'the box is idle'"
        ),
    }

    // ⊘ The order is the reading order and each arm is independent: its own address space,
    // its own addresses, its own channels. Every arm provokes an Xid in its own prime, so no
    // ordering makes an arm fault-free and none is 'protected' by running first.
    let plain = defer_arm(rm, DeferArm::Plain, 0);
    let defer = defer_arm(rm, DeferArm::Defer, 1);
    let inval = defer_arm(rm, DeferArm::DeferThenInvalidate, 2);

    println!();
    println!("═══ DL SUMMARY ═══");
    for r in [&plain, &defer, &inval] {
        println!(
            "info  DL {:32} prime_faulted={:?} except={:?} chan_survived={} pde_before_map={} \
             pte={} placed={:?} copy={} rescue={:?}",
            r.arm.as_str(),
            r.prime_faulted,
            r.prime_except.map(|e| format!("{e:#x}")),
            r.chan_survived,
            r.pde_before_map.as_str(),
            r.pte_after_map.as_str(),
            r.placed.map(|v| format!("{v:#018x}")),
            r.live.as_str(),
            r.rescue.map(|v| v.as_str()),
        );
    }

    // ── THE GRADE. Ordered so a broken instrument can never reach the verdict line. ─────
    let primes_ok = [&plain, &defer, &inval]
        .iter()
        .all(|r| r.prime_faulted == Some(true));
    println!(
        "DEFER_PRIME_FAULTS={}/3",
        [&plain, &defer, &inval]
            .iter()
            .filter(|r| r.prime_faulted == Some(true))
            .count()
    );
    println!(
        "DEFER_RIGCTL={}",
        match plain.live {
            DeferLive::Live => "PASS",
            DeferLive::Dead { .. } => "FAIL",
            DeferLive::NotRun(_) => "NOTRUN",
        }
    );
    println!(
        "DEFER_NEGCTL={}",
        match inval.live {
            DeferLive::Live => "PASS",
            DeferLive::Dead { .. } => "FAIL",
            DeferLive::NotRun(_) => "NOTRUN",
        }
    );

    let verdict: String = if !primes_ok {
        // ⚠ The literal token, so a grep for it finds this line and the per-arm one alike.
        "INVALID PRIME_DID_NOT_FAULT — at least one arm never made the MMU walk the address \
         under test while it was invalid, so its final copy would test FIRST FILL"
            .to_string()
    } else if !matches!(plain.live, DeferLive::Live) {
        "INVALID RIG_CONTROL_FAILED — an ORDINARY map, with RM's own invalidate, did not make \
         the address readable. Nothing about the deferred arms is interpretable"
            .to_string()
    } else if !matches!(inval.live, DeferLive::Live) {
        "INVALID NEGATIVE_CONTROL_FAILED — DEFER followed by an explicit \
         NV2080_CTRL_CMD_DMA_INVALIDATE_TLB did NOT make the address readable. The rig is \
         broken and a (B) reading off the DEFER arm would be worthless"
            .to_string()
    } else {
        match (defer.live, defer.rescue) {
            (DeferLive::Live, _) if defer.pde_before_map == W379HostVa::Unmeasured => {
                // ★ The asymmetry, applied. An unexcluded page-level allocation could only
                // have caused an EXTRA invalidate, so it can fake A and can never fake B.
                "INVALID CONFOUNDER_2_UNASSERTED — the deferred copy LANDED, but the \
                 page-directory oracle refused before the map, so 'RM allocated no page level \
                 and therefore invalidated nothing' is unmeasured. That confound manufactures \
                 exactly this answer"
                    .to_string()
            }
            (DeferLive::Live, _) => "A".to_string(),
            (DeferLive::Dead { .. }, Some(DeferLive::Live)) => "B".to_string(),
            (DeferLive::Dead { .. }, Some(DeferLive::Dead { .. })) => {
                "INVALID DEFERRED_MAP_WROTE_NO_USABLE_PTE — the copy was dead BEFORE and AFTER \
                 an invalidate, so the deferred map produced no entry at all and this arm is a \
                 statement about our own map rather than about the hardware's TLB"
                    .to_string()
            }
            (DeferLive::Dead { .. }, _)
                if matches!(defer.pte_after_map, DeferPte::Valid { .. }) =>
            {
                // The rescue could not run, but RM itself confirmed the leaf PTE, so the
                // attribution stands on the oracle instead.
                "B".to_string()
            }
            (DeferLive::Dead { saw }, r) => format!(
                "INVALID DEAD_BUT_UNATTRIBUTED — the deferred copy left {saw:#010x} in the \
                 destination, the PTE oracle did not answer, and the rescue did not run \
                 ({r:?}). 'Not live' and 'never written' are not separated"
            ),
            (DeferLive::NotRun(why), _) => format!("INVALID DEFER_ARM_DID_NOT_RUN — {why}"),
        }
    };
    println!("DEFER_LIVENESS={verdict}");
    let decided = verdict == "A" || verdict == "B";
    println!(
        "RUNGCTL_defer_liveness={}",
        if matches!(plain.live, DeferLive::Live) && matches!(inval.live, DeferLive::Live) {
            "PASS"
        } else {
            "FAIL"
        }
    );
    // ⊘ PASS means DECIDED, not "A". Both worlds are legitimate answers and a rung that
    // preferred one of them would be grading on the outcome it hoped for; the only failure
    // this rung has is not being able to tell them apart.
    println!(
        "RUNG_defer_liveness={}",
        if decided { "PASS" } else { "NOTRUN" }
    );
    decided
}

fn main() -> std::process::ExitCode {
    // ★★★ **w309 — ECHO ARGV, FIRST LINE, ALWAYS.**
    //
    // ⊘ w305's runner tried to recover which arm had been requested by grepping the probe log
    // for `ce-client-fault` and printed `[]` on BOTH boots, because nothing in the chain ever
    // echoed the client's arguments. It reported the field as unrecoverable and fell back to
    // inferring the arm from a downstream line. ⇒ One line, unconditional, before any flag is
    // parsed: what this process was ASKED to do. `R33 arm 4 CONFIG` then reports what it
    // actually DID, and a harness can compare the two rather than trust either.
    println!(
        "info  RMLADDER ARGV     = [{}]",
        std::env::args().skip(1).collect::<Vec<_>>().join(" ")
    );
    let mut gpu = 0u32;
    let mut want_concurrency = false;
    let mut want_engines = false;
    let mut want_census = false;
    let mut want_timer = false;
    let mut want_probe: Option<Vec<(u32, usize)>> = None;
    let mut want_binapi: Option<Vec<(u32, usize)>> = None;
    let mut want_gpu_info = false;
    let mut want_bus_info = false;
    let mut want_gpga_probe = false;
    let mut want_gpga_probe = false;
    let mut want_atomics = false;
    let mut want_pce_mask = false;
    let mut want_osdesc: Option<OsDescSeed> = None;
    let mut want_fb_join: Option<OsDescSeed> = None;
    let mut want_dictated_ring = false;
    let mut want_dictated_neg = false;
    let mut want_late_map_race = false;
    // ★★★★★ T1 — `--blockage-coverage` (`REQUIREMENTS_TARGET.md` R1, falsifier C1). Its OWN
    // flag and NOT folded into any battery: it is the only rung whose result lives in the
    // HOST's log rather than in its own stdout, so it needs a bracket nobody else's output
    // is inside. See [`blockage_coverage`].
    let mut want_blockage_coverage = false;
    let mut want_uvm = false;
    // ★★★★★ w392d — `--uvm-mean`, THE MEAN CLIENT. Its own flag and deliberately not
    // folded into any battery: its verdict is a LEDGER over five rows, and a battery
    // that ran it beside other rungs would interleave their channels' completions with
    // its own engine reads. See [`mean::run`].
    let mut want_mean = false;
    let mut mean_threads: usize = 4;
    let mut mean_p1_rounds: u32 = 4;
    // ⊘ `None` means *"draw one and PRINT it"*, never *"do not seed"*: a content
    //   failure whose pattern cannot be reproduced is an anecdote.
    let mut mean_nonce: Option<u32> = None;
    let mut mean_falsify = false;
    // ★★★★★ w379 — the mapping-plane rungs. Each is its own flag AND is included in
    // `--w379`, so a run can name one rung or take the whole battery; ⊘ there is no flag
    // that runs a rung WITHOUT its positive control, because a rung whose control did not
    // pass has no interpretable result to report.
    let mut want_alias_two_vas = false;
    let mut want_alias_unmap = false;
    let mut want_map_propagation = false;
    let mut want_missing_page = false;
    let mut want_map_stress = false;
    // ★★★★★ w381 — R4 and the cross-client rung, and THE PROBE SELECTOR.
    let mut want_rpc_mixed = false;
    let mut want_cross_client = false;
    // ★★★★★ w289 — `--defer-liveness`. Its OWN flag and deliberately NOT part of
    // `--w379`/`--w381`: it provokes three Xids of its own, it is the only rung in this
    // file whose result depends on what is CACHED in an MMU, and folding it into a
    // battery would put two other rungs' invalidates between its prime and its copy.
    let mut want_defer_liveness = false;
    // ⊘ The DEFAULT IS THE w379 PRIMITIVE, so every committed w379 arm stays byte-comparable
    // to its own predecessors. `--probe-launch-dma` is the only way to change it and the
    // choice is printed before any rung runs — a run that does not say which primitive it
    // used cannot be compared to any other run.
    let mut probe = W381Probe::SemRelease;
    // ★★★★★ w384 — the doorbell-latency rung and its configuration. `None` ⇒ not selected,
    // so the rung's absence and the rung's default arming are different states and a log can
    // tell them apart.
    let mut want_doorbell_latency: Option<DblCfg> = None;
    // ★★★★★ w385 — `--concurrent-fuzz`. Its own flag block, its own defaults, and NOT
    // folded into `--w379`/`--w381`: every rung in those batteries is single-threaded and
    // sequential, and quietly adding a thread pool to a committed battery would make every
    // earlier arm incomparable to its own predecessors.
    // ★★★ THE DEFAULT IS THE BENCH GUEST'S `-smp`, NOT A ROUND NUMBER. `boot_nvkvm.sh` boots
    // the Mode-2 guest with `-smp 3`, and in kayfabe **each guest vCPU is a host thread** —
    // so three contending threads IS the topology the product has to survive. A default of
    // "8 because 8 is a nice number" would be testing a machine nobody runs.
    // ⊘ If `boot_nvkvm.sh` changes its `-smp`, this number is stale and the run's own
    // `W385 topology` line is where that shows up.
    const W385_GUEST_SMP: usize = 3;
    let mut want_concurrent_fuzz = false;
    let mut fuzz_threads: usize = W385_GUEST_SMP;
    let mut fuzz_cores: usize = W385_GUEST_SMP;
    let mut fuzz_iters: u64 = 64;
    let mut fuzz_clients: usize = 2;
    let mut fuzz_jitter_us: u64 = 200;
    // ⊘ `None` means *"draw one and PRINT it"*, never *"do not seed"*. There is no unseeded
    // mode: a fuzz failure you cannot replay is an anecdote.
    let mut fuzz_seed: Option<u64> = None;
    let mut fuzz_deadline_s: u64 = 900;
    let mut fuzz_stall_s: u64 = 120;
    let mut want_guest_pin = false;
    let mut want_guest_ring = false;
    let mut want_executor_vas = false;
    let mut want_executor_alias = false;
    let mut want_fb_view: Option<FbViewJoin> = None;
    let mut want_bar1_crossing = false;
    let mut want_ce_client = false;
    let mut want_ce_client_fault = false;
    // ★ w305 — see `--ce-client-fault-shared-vas`. Default false ⇒ byte-identical default arm.
    let mut want_ce_client_fault_shared_vas = false;
    // ★ w309 — see `--ce-client-fault-rm-placed` / `--ce-client-fault-no-notifier`. Both
    // `false` ⇒ `ReachProbeArms::default()` ⇒ byte-identical to every committed arm.
    let mut want_ce_client_fault_rm_placed = false;
    let mut want_ce_client_fault_no_notifier = false;
    // ★★★★★ w288 TIER 2 — SYSMEM by default; see `--notifier-vidmem` for the measured
    // reason the other arm exists and why neither is a fallback for the other.
    let mut notifier_aperture = kayfabe_isolate_host::rm::NotifierAperture::Sysmem;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--gpu" => {
                let Some(v) = args.next() else {
                    eprintln!("--gpu needs a value");
                    return std::process::ExitCode::from(64);
                };
                match v.parse() {
                    Ok(n) => gpu = n,
                    Err(_) => {
                        eprintln!("--gpu {v} is not a number");
                        return std::process::ExitCode::from(64);
                    }
                }
            }
            "--concurrency" => want_concurrency = true,
            "--timer" => want_timer = true,
            "--engines" => want_engines = true,
            "--doorbell-census" => want_census = true,
            "--gpu-info-sweep" => want_gpu_info = true,
            "--bus-info-sweep" => want_bus_info = true,
            // ★★★ `docs/design/gpga_is_one_reserved_object.md` step 1's gate: can the whole
            // guest framebuffer be reserved as ONE object, and how fast is it to READ over
            // PCIe? Both answers decide the design before a five-minute boot can.
            "--gpga-reserve-probe" => want_gpga_probe = true,
            "--atomics-probe" => want_atomics = true,
            "--pce-mask-probe" => want_pce_mask = true,
            "--osdesc-probe" => want_osdesc = Some(OsDescSeed::BeforeDescribe),
            // ⊘ The negative control. Same chain, unwritten memfd, inverted verdict.
            "--osdesc-negative" => want_osdesc = Some(OsDescSeed::Never),
            "--fb-memfd-join" => want_fb_join = Some(OsDescSeed::BeforeDescribe),
            // ⊘ The negative control. Same chain, `S` never written, forward verdict
            // inverted — and the reverse arm must still hold.
            "--fb-memfd-join-negative" => want_fb_join = Some(OsDescSeed::Never),
            "--dictated-ring" => want_dictated_ring = true,
            // ⊘ The negative control. Same address, occupied first, inverted verdict.
            "--dictated-ring-negative" => want_dictated_neg = true,
            // ★★★★★ W377. Runs BOTH arms (control + race) in one invocation, because the
            // race arm is uninterpretable without the control and separating them into two
            // flags would let someone run only the half that produces a headline.
            "--late-map-race" => want_late_map_race = true,
            // ★★★★★ T1. Runs BOTH phases in one invocation, for `--late-map-race`'s reason
            // and one more: phase B is phase A's *known-positive*, so a flag that could run
            // only phase A would produce the favourable half of a differential with nothing
            // to compare it to.
            "--blockage-coverage" => want_blockage_coverage = true,
            "--uvm-invalidate" => want_uvm = true,
            // ★★★★★ w392d — the MEAN client. Runs every row in one invocation, for
            // `--late-map-race`'s reason: a flag that could select one row would
            // produce the favourable half of a ledger whose whole point is that an
            // unexercised row holds the verdict red.
            "--uvm-mean" => want_mean = true,
            "--mean-threads" => {
                let Some(v) = args.next() else {
                    eprintln!("--mean-threads needs a value");
                    return std::process::ExitCode::from(64);
                };
                match v.parse::<usize>() {
                    Ok(n) if n >= 2 => mean_threads = n,
                    // ⊘ Refused rather than clamped: a run that silently became
                    //   single-threaded would fail the THREADS row for a reason
                    //   nothing in its output names.
                    Ok(n) => {
                        eprintln!("--mean-threads {n} cannot race; the row needs >= 2");
                        return std::process::ExitCode::from(64);
                    }
                    Err(_) => {
                        eprintln!("--mean-threads {v} is not a number");
                        return std::process::ExitCode::from(64);
                    }
                }
            }
            "--mean-rounds" => {
                let Some(v) = args.next() else {
                    eprintln!("--mean-rounds needs a value");
                    return std::process::ExitCode::from(64);
                };
                match v.parse::<u32>() {
                    Ok(n) if n >= 2 => mean_p1_rounds = n,
                    // ⊘ One round cannot catch a stale mapping: P1's whole content is
                    //   the comparison BETWEEN rounds at one VA.
                    Ok(n) => {
                        eprintln!("--mean-rounds {n} cannot see a stale mapping; need >= 2");
                        return std::process::ExitCode::from(64);
                    }
                    Err(_) => {
                        eprintln!("--mean-rounds {v} is not a number");
                        return std::process::ExitCode::from(64);
                    }
                }
            }
            // ★★★★★ THE FALSIFIER. Opt-in because it provokes a real Xid 31 and kills its
            // own channel — see [`mean::falsifier`].
            "--mean-falsify" => mean_falsify = true,
            "--mean-nonce" => {
                let Some(v) = args.next() else {
                    eprintln!("--mean-nonce needs a value");
                    return std::process::ExitCode::from(64);
                };
                match u32::from_str_radix(v.trim_start_matches("0x"), 16) {
                    Ok(n) => mean_nonce = Some(n),
                    Err(_) => {
                        eprintln!("--mean-nonce {v} is not hex");
                        return std::process::ExitCode::from(64);
                    }
                }
            }
            // ★★★★★ w379 R1′ — one allocation, two GPU VAs, and a release through the
            // FIRST one after the second is mapped.
            "--alias-two-vas" => want_alias_two_vas = true,
            // ★★★★★ w379 R1″ — the discriminator: unmap one alias, ask whether the other
            // survived.
            "--alias-unmap-observe" => want_alias_unmap = true,
            // ★★★★★ w379 R2 — does a mapping at a dictated VA land in the HOST's tables?
            "--map-propagation" => want_map_propagation = true,
            // ★★★★★ w379 R3 — a missing page on purpose: contained, and named.
            // ⚠ Provokes a real `Xid 31` and kills its victim channel. That is the
            // measurement, not a side effect — which is why it is opt-in.
            "--missing-page-fault" => want_missing_page = true,
            // ★★★★★ w379 R5 — the stress rung: interleaved alloc/map/free over a rolling
            // window. ⊘ Single-client; cross-client leakage is NOT covered by it.
            "--map-stress" => want_map_stress = true,
            // The whole battery, in dependency order: R2 establishes that a dictated
            // mapping lands at all, R1′ asks whether TWO of them can, R1″ asks what an
            // unmap does to the survivor, R3 asks what a MISSING one does to a bystander.
            "--w379" => {
                want_map_propagation = true;
                want_alias_two_vas = true;
                want_alias_unmap = true;
                want_missing_page = true;
                want_map_stress = true;
            }
            // ★★★★★ w381 — SWITCH THE LIVENESS PRIMITIVE to the one the Mode-2 CPU
            // copy-engine emulator actually serves. See [`W381Probe`] for why the w379
            // primitive cannot run in a guest at all, and why this is a substitution rather
            // than a weakening.
            "--probe-launch-dma" => probe = W381Probe::LaunchDma,
            // ⊘ The explicit spelling of the default. It exists so a harness can state the
            // arm it wants instead of relying on absence, which is how two runs come to
            // differ in a way neither log records.
            "--probe-sem-release" => probe = W381Probe::SemRelease,
            // ★★★★★ w381 R4 — allocations that cross to the emulated GSP interleaved with
            // ones served locally: does ORDERING and IDENTITY survive the mix?
            "--rpc-mixed-allocs" => want_rpc_mixed = true,
            // ★★★★★ w381 R5b — a SECOND RM client, and the standing requirement that two
            // guest processes must not see each other's memory.
            "--cross-client-leak" => want_cross_client = true,
            // ★★★★★ THE GUEST-SERVABLE BATTERY. Everything `--w379` selects, plus R4 and the
            // cross-client rung, on the `LAUNCH_DMA` primitive — i.e. the exact invocation
            // that is meant to run identically on bare metal AND inside a Mode-2 guest.
            // ⊘ It is ONE flag on purpose: the differential is only a differential if both
            // halves ran the same selection with the same primitive, and two harnesses
            // assembling that selection out of six flags each is how they come to differ.
            "--w381" => {
                want_map_propagation = true;
                want_alias_two_vas = true;
                want_alias_unmap = true;
                want_missing_page = true;
                want_map_stress = true;
                want_rpc_mixed = true;
                want_cross_client = true;
                probe = W381Probe::LaunchDma;
            }
            // ★★★★★ w385 — THE MULTI-THREADED FUZZ. See [`concurrent_fuzz`].
            "--concurrent-fuzz" => want_concurrent_fuzz = true,
            // ★★★★★ w289 — see `defer_liveness`. One flag, no knobs: every arm of this
            // rung is a control for another one, so there is no configuration in which
            // it runs a subset and still means anything.
            "--defer-liveness" => want_defer_liveness = true,
            // Every knob takes `--flag N` **and** `--flag=N`, because a harness that writes
            // one and a human who types the other must not silently get the default.
            s if s.starts_with("--fuzz-threads")
                || s.starts_with("--fuzz-iters")
                || s.starts_with("--fuzz-clients")
                || s.starts_with("--fuzz-cores")
                || s.starts_with("--fuzz-jitter-us")
                || s.starts_with("--fuzz-deadline")
                || s.starts_with("--fuzz-stall")
                || s.starts_with("--seed") =>
            {
                let (name, inline) = match s.split_once('=') {
                    Some((n, v)) => (n, Some(v.to_string())),
                    None => (s, None),
                };
                let Some(v) = inline.or_else(|| args.next()) else {
                    eprintln!("{name} needs a value");
                    return std::process::ExitCode::from(64);
                };
                // ⊘ `0x`-prefixed seeds are accepted because that is how this rung PRINTS
                // them; a seed you cannot paste back is not a replay handle.
                let parsed = if let Some(h) = v.strip_prefix("0x") {
                    u64::from_str_radix(h, 16)
                } else {
                    v.parse::<u64>()
                };
                let Ok(n) = parsed else {
                    eprintln!("{name} {v} is not a number");
                    return std::process::ExitCode::from(64);
                };
                match name {
                    "--fuzz-threads" => fuzz_threads = (n as usize).clamp(1, 64),
                    "--fuzz-iters" => fuzz_iters = n.max(1),
                    "--fuzz-clients" => fuzz_clients = (n as usize).clamp(1, 8),
                    "--fuzz-cores" => fuzz_cores = (n as usize).clamp(1, 64),
                    "--fuzz-jitter-us" => fuzz_jitter_us = n,
                    "--fuzz-deadline" => fuzz_deadline_s = n.max(1),
                    "--fuzz-stall" => fuzz_stall_s = n.max(1),
                    _ => fuzz_seed = Some(n),
                }
            }
            // ★★★★★ w384 — WHAT ONE DOORBELL COSTS THE SUBMITTING THREAD. Its own flag and
            // its own dispatch block: it is a TIMING rung, and running it behind six other
            // rungs would time a process whose allocator, page tables and RM client are in a
            // state those rungs put them in. ⊘ It is deliberately NOT in `--w381`.
            "--doorbell-latency" => want_doorbell_latency = Some(DblCfg::default()),
            // ⚠ Each knob EDITS the config rather than replacing it, so the flags may be
            // given in any order and `--doorbell-latency-n 64` alone still implies the rung.
            // A flag whose meaning depended on its position is a harness defect waiting for
            // a script to be reordered.
            "--doorbell-latency-n"
            | "--doorbell-latency-budget-ms"
            | "--doorbell-latency-reps"
            | "--doorbell-latency-native-us"
            | "--doorbell-latency-gate" => {
                let Some(v) = args.next() else {
                    eprintln!("{flag} needs a value");
                    return std::process::ExitCode::from(64);
                };
                let mut cfg = want_doorbell_latency.unwrap_or_default();
                let ok = match flag.as_str() {
                    "--doorbell-latency-n" => v.parse::<usize>().map(|n| cfg.n_max = n).is_ok(),
                    "--doorbell-latency-budget-ms" => v
                        .parse::<u64>()
                        .map(|m| cfg.budget = std::time::Duration::from_millis(m))
                        .is_ok(),
                    "--doorbell-latency-reps" => v.parse::<usize>().map(|r| cfg.reps = r).is_ok(),
                    // ★★★ THE FLOOR THE GUEST ARM IS GRADED AGAINST, fed in by the script from
                    // the native arm it just ran — never a constant baked into the binary. A
                    // baked floor would expire as a box changed and nobody would notice,
                    // which is the shape `a_capture_derived_table_expires_as_a_vendor_
                    // regression` records.
                    "--doorbell-latency-native-us" => v
                        .parse::<f64>()
                        .map(|u| cfg.native_p50_us = Some(u))
                        .is_ok(),
                    _ => v.parse::<f64>().map(|k| cfg.gate_multiple = k).is_ok(),
                };
                if !ok {
                    eprintln!("{flag} {v} is not a number");
                    return std::process::ExitCode::from(64);
                }
                want_doorbell_latency = Some(cfg);
            }
            "--guest-ram-pin" => want_guest_pin = true,
            "--guest-ring-channel" => want_guest_ring = true,
            "--executor-vas" => want_executor_vas = true,
            // ★ Arm C, opt-in: it provokes a real host fault when the boundary HOLDS.
            "--executor-vas-alias" => {
                want_executor_vas = true;
                want_executor_alias = true;
            }
            // ★★★ R33 — the raw CE client. Its own flag, and it RETURNS: the whole point is
            // a program small enough to push into a guest, so it must not drag the isolate,
            // the sandbox rung or a second channel along.
            "--ce-client" => want_ce_client = true,
            // ⊘ Arm 4, opt-in: it provokes a real `Xid 31 FAULT_PDE` and kills its channel.
            "--ce-client-fault" => {
                want_ce_client = true;
                want_ce_client_fault = true;
            }
            // ★★★★★ **w305 — arm 4 in arm 1's ALREADY-WORKING VAS instead of a fresh one.**
            //
            // The `road_to_v1_after_cup2.md` §2 fix, in the only form that is not a no-op:
            // the operands were always in the ringing channel's VAS, so what this changes is
            // that the space has already carried retired work. ⊘ Opt-in; the default arm is
            // byte-identical to every committed run, so the two are comparable.
            "--ce-client-fault-shared-vas" => {
                want_ce_client = true;
                want_ce_client_fault = true;
                want_ce_client_fault_shared_vas = true;
            }
            // ★★★★★ **w309 — THE OTHER TWO CONFOUNDS, one flag each, both opt-in.**
            //
            // ⊘ Deliberately NOT composed into a single `--ce-client-fault-arm=<name>`: the
            // whole design of this rung is *move ONE at a time*, and a single enum invites an
            // arm that moves two and reads as one row.
            //
            // `--ce-client-fault-rm-placed` — RM chooses the probe's ring and both operands,
            // as it does for arm 1, instead of the probe dictating them inside
            // `REACH_PROBE_WINDOW`. ⚠ This re-opens the 2026-08-10 self-alias that INVERTED a
            // verdict, so the probe refuses by name (`PROBE_SELF_ALIASED`) if the fault VA
            // lands inside anything RM placed for it.
            "--ce-client-fault-rm-placed" => {
                want_ce_client = true;
                want_ce_client_fault = true;
                want_ce_client_fault_rm_placed = true;
            }
            // `--ce-client-fault-no-notifier` — no error-notifier object is allocated and
            // `hObjectError` is unset, exactly as every caller before w287 built it. ⊘ Plane A
            // is UNMEASURED on this arm by construction; it can only ever answer *"is the
            // notifier's presence what stops the control landing?"*, never criterion 1.
            "--ce-client-fault-no-notifier" => {
                want_ce_client = true;
                want_ce_client_fault = true;
                want_ce_client_fault_no_notifier = true;
            }
            // ★★★★★ **w288 TIER 2 — the notifier's APERTURE, as an explicit arm.**
            //
            // Default is SYSMEM, because that is the faithful shape (`[w287 census, 63/63]`)
            // and the ONLY one a GUEST-side run can have served: this port's
            // `ErrorNotifier` vocabulary is `Sysmem { gpa }` or `Unreachable`, so a vidmem
            // notifier attaches nothing at all and the run measures nothing, silently.
            //
            // ⊘⊘ This flag exists because the other arm is MEASURED, not hypothetical:
            // `[measured 2026-08-13, vh2, rev f7a74bc]` a `NV01_MEMORY_SYSTEM` notifier was
            // refused **natively** in both flag settings tried (`NV_ERR_INVALID_ARGUMENT` at
            // the CPU map with `_MAPPING_NO_MAP`; `EINVAL` at the allocation without it), so
            // a native run may need the vidmem arm. ⚠ It is a named CHOICE and never a
            // fallback: nothing tries one and retries the other, because a run whose
            // aperture depended on what RM accepted could not say which experiment it ran.
            "--notifier-vidmem" => {
                notifier_aperture = kayfabe_isolate_host::rm::NotifierAperture::Vidmem;
            }
            // ★★★★★ w393 — the BAR1 crossing on bare metal; see `bar1_crossing_probe`.
            "--bar1-crossing" => want_bar1_crossing = true,
            "--bar1-crossing-child" => return bar1_crossing_child(),
            "--fb-view-probe" => want_fb_view = Some(FbViewJoin::Shared),
            // ⊘ The negative control. Same chain, private guest-side pages, inverted verdict.
            "--fb-view-negative" => want_fb_view = Some(FbViewJoin::Private),
            "--probe-ctrl" => {
                let Some(v) = args.next() else {
                    eprintln!("--probe-ctrl needs a `cmd[:size],...` list");
                    return std::process::ExitCode::from(64);
                };
                match parse_ctrl_specs(&v) {
                    Ok(specs) => want_probe = Some(specs),
                    Err(e) => {
                        eprintln!("--probe-ctrl {e}");
                        return std::process::ExitCode::from(64);
                    }
                }
            }
            "--binapi-ctrl" => {
                let Some(v) = args.next() else {
                    eprintln!("--binapi-ctrl needs a `cmd[:size],...` list");
                    return std::process::ExitCode::from(64);
                };
                match parse_ctrl_specs(&v) {
                    Ok(specs) => want_binapi = Some(specs),
                    Err(e) => {
                        eprintln!("--binapi-ctrl {e}");
                        return std::process::ExitCode::from(64);
                    }
                }
            }
            other => {
                eprintln!("unknown flag {other}");
                return std::process::ExitCode::from(64);
            }
        }
    }

    // ★★★ R33 — the census is armed BEFORE the connection is opened, because `R0`–`R6`
    // (`CARD_INFO`, `REGISTER_FD`, `CHECK_VERSION_STR`, the root/device/subdevice allocs)
    // are part of what a raw client costs. Arming it after `open` would report a number
    // that is smaller, still looks like an answer, and is not one.
    if want_ce_client {
        kayfabe_linux_raw::census::reset();
        kayfabe_linux_raw::census::record_sequence(true);
        kayfabe_linux_raw::census::phase("R0-R6 bring-up");
    }

    let dev = match DevDir::open(c"/dev") {
        Ok(d) => d,
        Err(e) => {
            println!("FAIL  R0 open(/dev): {e}");
            return std::process::ExitCode::from(1);
        }
    };

    // R0–R6 all happen inside `open`, each carrying its own rung name if it fails. That is
    // the point of `BringUpError::rung` — one message, one layer.
    // ★ #156 — same pinned host-class profile the isolate child uses. The ladder is a
    // diagnostic for the SAME path, so a different profile here would make it a
    // diagnostic for a different one.
    let conn = match RmConnection::open(&dev, GpuId(gpu), kayfabe_chips::pinned_host_classes()) {
        Ok(c) => c,
        Err(e) => {
            println!("FAIL  {}", e);
            return std::process::ExitCode::from(1);
        }
    };
    println!("ok    R2 version         = {:?}", conn.driver_version());
    println!("ok    R4 hClient         = {:#010x}", conn.client());
    println!("ok    R6 hSubdevice      = {:#010x}", conn.subdevice());

    let id = IsolateId::new(0, GpuId(gpu));
    let conn = Arc::new(conn);
    let subdevice = kayfabe_isolate::HostHandle::new(id, u64::from(conn.subdevice()));
    // ★ Its own export table. The ladder is a diagnostic run by hand, not an isolate under
    // a VMM, so nothing will ever read a backing out of this one — it exists so the
    // backend's constructor has the same shape here as in the child.
    let mut rm = HostRmBackend::new(
        id,
        Arc::clone(&conn),
        Arc::new(kayfabe_isolate_host::ChildExports::new()),
    );

    // ★★★ R33 runs here and RETURNS. It is the OWNER'S RAW CE CLIENT, and everything below
    // it — the second channel, the isolate, the sandbox rung — would put ioctls in the
    // census that the client does not need, and a sandboxed child in a guest that may not
    // have one.
    if want_ce_client {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        // ⊘ Printed BEFORE the arms, on every run: a run whose notifier aperture is not on
        // the log is a run whose quiet notifier cannot be graded.
        println!(
            "info  R33 NOTIFIER APERTURE = {} ⊘ on the GUEST path a VIDMEM notifier decodes \
             to ErrorNotifier::Unreachable, so NO host notifier is attached and arm 4 \
             measures NOTHING while looking like it ran",
            notifier_aperture.as_str(),
        );
        let ok = ce_client(
            &mut rm,
            gpu,
            want_ce_client_fault,
            want_ce_client_fault_shared_vas,
            kayfabe_isolate_host::rm::ReachProbeArms {
                dictate_addresses: !want_ce_client_fault_rm_placed,
                error_notifier: !want_ce_client_fault_no_notifier,
            },
            notifier_aperture,
        );
        print_ioctl_census(if want_ce_client_fault {
            "R33 raw CE client, arms 1-4"
        } else {
            "R33 raw CE client, arms 1-3"
        });
        println!(
            "done — raw CE client only ({})",
            if ok {
                "ALL ARMS MET"
            } else {
                "WITH FAILED EVIDENCE"
            }
        );
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★ R18 runs here and RETURNS, before R7 — a control probe must not be paid for with
    // a channel, a doorbell and a copy engine. It reads; it allocates nothing; and leaving
    // the rest of the ladder unrun keeps the answer attributable to the control alone.
    if let Some(specs) = want_probe {
        probe_ctrl(&mut rm, subdevice, &specs);
        println!("done — probe only");
        return std::process::ExitCode::SUCCESS;
    }

    // ★ R21 runs here and RETURNS, for R18's reason: it issues seventy-one controls on the
    // subdevice and allocates nothing, so every refusal is the index's.
    if want_gpu_info {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        gpu_info_sweep(&mut rm, subdevice);
        println!("done — gpu-info sweep only");
        return std::process::ExitCode::SUCCESS;
    }

    // ★ R22 runs here and RETURNS, for R21's reason: it issues fifty-four controls on the
    // subdevice and allocates nothing, so every refusal is the index's.
    if want_bus_info {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        bus_info_sweep(&mut rm, subdevice);
        println!("done — bus-info sweep only");
        return std::process::ExitCode::SUCCESS;
    }

    // ★ R23 runs here and RETURNS, for R18's reason: eight controls on the bare Subdevice,
    // nothing allocated, so a refusal is the request's or the object's and cannot be a
    // channel's from three rungs earlier — which is the whole variable under test.
    // ★★★★★ GPGA RESERVATION PROBE — the host-side gate for
    // `docs/design/gpga_is_one_reserved_object.md` step 1. Answers, without a VM, the two
    // questions the design turns on: is the whole framebuffer reservable as ONE object, and
    // what does it cost to READ over PCIe.
    if want_gpga_probe {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        // ⊘ Starts at the advertised 12288 MiB deliberately: the design says the guest's size
        // must be DERIVED from what succeeds, and this prints the gap between the number we
        // advertise and the number the card will actually give.
        let mb = rm.largest_reservable_mb(12288);
        println!("GPGA_LARGEST_RESERVABLE_MB={mb}  (advertised today: 12288)");
        if mb == 0 {
            println!("GPGA_PROBE=(F) ⊘ nothing down to 256 MiB reserved — the design's premise fails here");
        } else {
            // The PCIe read cost, on a slice the size of the page tables a 12 GiB mapping
            // needs (~24 MiB), which is the quantity the refresh would re-read.
            let probe_len: u64 = 24 << 20;
            match rm.time_vidmem_read(probe_len) {
                Ok((took, _acc)) => {
                    let mbps = (probe_len as f64 / (1 << 20) as f64) / took.as_secs_f64();
                    println!(
                        "GPGA_VIDMEM_READ={:.1} MiB/s over {} MiB in {:.1} ms  ⇒ a full \
                         page-table re-read costs ~{:.0} ms, and 1178 refreshes would cost \
                         ~{:.1} s if NOT promoted",
                        mbps,
                        probe_len >> 20,
                        took.as_secs_f64() * 1e3,
                        took.as_secs_f64() * 1e3,
                        took.as_secs_f64() * 1178.0
                    );
                    let (h_took, _) =
                        kayfabe_isolate_host::rm::HostRmBackend::time_hostmem_read(probe_len);
                    let h_mbps =
                        (probe_len as f64 / (1 << 20) as f64) / h_took.as_secs_f64();
                    println!(
                        "GPGA_HOSTMEM_READ={h_mbps:.0} MiB/s (the SAME loop over ordinary RAM) \
                         ⇒ ratio {:.0}x — if this is fast, the cost is the BUS, not the wrapper",
                        h_mbps / mbps
                    );
                    println!("GPGA_PROBE=(P) ★ reserved {mb} MiB and measured the read cost");
                }
                Err(e) => println!("GPGA_PROBE=(E) ⊘ reserved {mb} MiB but the read probe refused: {e:?}"),
            }
        }
        return std::process::ExitCode::SUCCESS;
    }

    if want_atomics {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        atomics_probe(&mut rm, subdevice);
        println!("done — atomics probe only");
        return std::process::ExitCode::SUCCESS;
    }

    // ★ R24 runs here and RETURNS, for R23's reason: five controls on the bare Subdevice,
    // nothing allocated, so a refusal is the request's or the object's and cannot be a
    // channel's from three rungs earlier.
    if want_pce_mask {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        ce_pce_mask_probe(&mut rm, subdevice);
        println!("done — pce-mask probe only");
        return std::process::ExitCode::SUCCESS;
    }

    // ★ R20 runs here and RETURNS, for R18's reason: the whole value of the rung is that the
    // only object in play is the `NV2081_BINAPI` it allocates itself, so a refusal is the
    // control's or the class's and cannot be a channel's from three rungs earlier.
    if let Some(specs) = want_binapi {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        binapi_probe(&mut rm, subdevice, &specs);
        println!("done — binapi probe only");
        return std::process::ExitCode::SUCCESS;
    }

    // ★ R25 runs here and RETURNS, for R18's reason and one more: it allocates its own
    // `Vas`, its own descriptor and its own copy-engine channel, so a refusal is
    // attributable to `OS_DESCRIPTOR` and cannot be R13's channel or R9's mapping from
    // earlier in the ladder. That attribution IS the rung — the question is which of four
    // named things refused, not whether something did.
    // ★ R26 runs here and RETURNS, for R25's reason exactly: it allocates its own `Vas` and
    // its own channel at its own address, so a refusal is attributable to the **dictated
    // placement** and cannot be R13's RM-placed channel from earlier in the ladder. Two
    // channel allocations in one process, one placed and one not, would make "which one
    // refused?" a question — and the answer to that question IS the rung.
    if want_dictated_neg {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = dictated_ring_negative(&mut rm, gpu);
        println!("done — dictated-ring negative control only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★★★★★ w289 — THE DEFERRED-INVALIDATE LIVENESS RUNG runs here and RETURNS, and the
    // isolation is the whole instrument rather than a convention: its result is a statement
    // about what a hardware MMU has CACHED, so anything else in this process that maps,
    // unmaps, frees a channel or reserves a VA range raises an invalidate this rung cannot
    // see and cannot control for. ⊘ It is deliberately absent from `--w379`, `--w381` and
    // every other battery, for exactly that reason.
    if want_defer_liveness {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        // ⊘ The probe selector is NOT honoured here and the line says so rather than being
        // silently absent: every submission in this rung is a `LAUNCH_DMA` out of the address
        // under test, because the question is whether a **read translation** resolves. A
        // `SEM_RELEASE` writes an address and proves the write path only, and the Mode-2
        // emulator does not act on it at all.
        println!(
            "W381_PROBE=launch-dma — ⊘ FIXED for this rung, whatever `--probe-*` asked for: the \
             question is whether a READ through a freshly-written PTE resolves, and a semaphore \
             release cannot ask it"
        );
        let ok = defer_liveness(&mut rm, gpu);
        println!("done — w289 defer-liveness rung only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★★★★★ w384 — THE DOORBELL-LATENCY RUNG runs here and RETURNS, and the isolation is
    // the point rather than a convention: it is the only rung in this file whose result is a
    // TIME, so anything that ran before it in the same process — RM allocations, page tables,
    // a heap the earlier rungs grew, a channel some other rung left scheduled — is a
    // confound it cannot control for and cannot see. ⊘ It is deliberately absent from
    // `--w379` and `--w381` for the same reason.
    if let Some(cfg) = want_doorbell_latency {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        // ⊘ The probe selector is NOT honoured here and the line says so rather than being
        // silently absent: this rung's submission is a `LAUNCH_DMA` unconditionally, because
        // the host-FIFO `SEM_RELEASE` the other arm uses is not served by the Mode-2
        // emulator at all — so a `sem-release` run would produce a guest distribution over
        // submissions nothing ever executed.
        println!(
            "W381_PROBE=launch-dma — ⊘ FIXED for this rung, whatever `--probe-*` asked for: \
             the Mode-2 emulator does not act on `SemRelease`, and timing submissions that \
             are never executed would be a distribution about nothing"
        );
        let ok = doorbell_latency(&mut rm, gpu, cfg);
        println!("done — w384 doorbell-latency rung only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★★★★★ w385 — THE CONCURRENT FUZZ. It runs here and RETURNS, for the reason every
    // rung below does: it allocates its own address spaces, its own channels and its own
    // objects at addresses no other rung uses, so an outcome is attributable to THIS rung.
    // ⊘ And it must not share a process with a rung that deliberately provokes an `Xid`.
    if want_concurrent_fuzz {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        println!("W381_PROBE={}", probe.as_str());
        // ⊘ A seed is DRAWN when none was given, and drawn seeds are printed exactly like
        // given ones — so every run, without exception, carries its own replay handle.
        let seed = fuzz_seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x385)
                | 1
        });
        let cfg = W385Cfg {
            threads: fuzz_threads,
            iters: fuzz_iters,
            clients: fuzz_clients,
            jitter_us: fuzz_jitter_us,
            seed,
            deadline_s: fuzz_deadline_s,
            stall_s: fuzz_stall_s,
            // ⊘ The top-level placement is UNPINNED and is only the seed of the arm table:
            // `concurrent_fuzz` runs `unpinned`, `percore`, `crowd-free` and `crowd` and
            // sets each arm's placement EXPLICITLY, so no arm inherits a default nobody
            // named. A default you never see exercised is a default you do not have.
            pin: W385Pin::Unpinned,
            cores: fuzz_cores,
        };
        let ok = concurrent_fuzz(&conn, probe, gpu, cfg);
        println!("done — w385 concurrent fuzz only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★★★★★ W377 runs here and RETURNS, for R26's reason exactly: each arm allocates its
    // own `Vas`, its own channel and its own target at addresses no other rung uses, so an
    // outcome is attributable to THIS rung's placements and to nothing earlier in the
    // ladder.
    // ★★★★★ w379 — the mapping-plane battery. It RETURNS, for the same reason W377 does:
    // every rung allocates its own `Vas`, its own channel and its own objects at addresses
    // no other rung uses, so an outcome is attributable to THIS rung's placements and to
    // nothing earlier in the ladder.
    if want_map_propagation
        || want_alias_two_vas
        || want_alias_unmap
        || want_missing_page
        || want_map_stress
        || want_rpc_mixed
        || want_cross_client
    {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        // ★★★★★ w381 — PRINTED BEFORE ANY RUNG RUNS, on every invocation. A battery whose
        // liveness primitive is not on its own log cannot be compared to the other half of
        // its differential, and *"we must have used the copy probe"* is exactly the kind of
        // reconstruction this repo has paid for. ⊘ It is also the ONE line that says whether
        // a guest arm could have reached its positive control at all.
        println!(
            "W381_PROBE={} — {}",
            probe.as_str(),
            match probe {
                W381Probe::SemRelease =>
                    "the w379 host-FIFO `SEM_RELEASE`. ⊘ The Mode-2 CPU copy-engine emulator                      decodes `PushMethod::SemRelease` and DELIBERATELY DOES NOT ACT ON IT, so                      inside a guest EVERY rung below is expected to report its CONTROL as                      FAILED and NOTRUN. That is the emulator's declared scope, NOT a red",
                W381Probe::LaunchDma =>
                    "a four-byte `LAUNCH_DMA` copied out of the channel's own ring. ★ This is                      the verb the Mode-2 emulator serves end to end (`run_submission` ->                      `execute_ours_spans` MOVES THE BYTES -> `write_resolved_completion`), so                      these rungs are servable on BOTH sides of the differential",
            }
        );
        // ⊘ Every SELECTED rung's verdict line is printed by the rung itself; the ones NOT
        // selected are printed here as `NOTRUN`, so a grader reading these lines always
        // sees the full vocabulary and can never mistake *"this rung was not asked for"*
        // for *"this rung was asked for and said nothing"*.
        if !want_map_propagation {
            println!("RUNG_map_propagation=NOTRUN");
        }
        if !want_alias_two_vas {
            println!("RUNG_alias_two_vas=NOTRUN");
        }
        if !want_alias_unmap {
            println!("RUNG_alias_unmap=NOTRUN");
        }
        if !want_missing_page {
            println!("RUNG_missing_page=NOTRUN");
        }
        if !want_map_stress {
            println!("RUNG_map_stress=NOTRUN");
        }
        if !want_rpc_mixed {
            println!("RUNG_rpc_mixed=NOTRUN");
        }
        if !want_cross_client {
            println!("RUNG_cross_client=NOTRUN");
        }
        // ⚠ Each rung runs and its result is recorded; none short-circuits the next. A
        // battery that stopped at the first red would report a stopping point rather than a
        // result — the exact shape `cargo test --workspace` was caught doing.
        let mut all = true;
        if want_map_propagation {
            all &= map_propagation(&mut rm, gpu);
        }
        if want_alias_two_vas {
            all &= alias_two_vas(&mut rm, probe, gpu);
        }
        if want_alias_unmap {
            all &= alias_unmap_observe(&mut rm, probe, gpu);
        }
        if want_rpc_mixed {
            all &= rpc_mixed_allocs(&mut rm, probe, gpu);
        }
        if want_cross_client {
            all &= cross_client_leak(&mut rm, probe, gpu);
        }
        // ⚠ R3 is LAST of the fault-free rungs' neighbours on purpose: it provokes a real
        // `Xid 31` and kills its victim channel, and anything after it would be running
        // behind a fault it did not cause. R5's rolling window follows it only because its
        // own bystander arm is what proves the fault took nothing else with it.
        if want_missing_page {
            all &= missing_page_fault(&mut rm, probe, gpu);
        }
        if want_map_stress {
            all &= map_stress(&mut rm, probe, gpu);
        }
        println!("done — w379/w381 mapping-plane rungs only");
        return if all {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    if want_late_map_race {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = late_map_race(&mut rm, gpu);
        println!("done — late-map race only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★★★★★ T1 — R1/C1. ⊘ Placed AFTER `--late-map-race` and before every other rung so a
    // run that names both gets the race's four regions and this rung's four in a fixed
    // order; the two use disjoint regions, so either order is correct and only ONE of them
    // is comparable across boots.
    // ★★★★★ w392c — POINT 2's known-positive. Runs BEFORE `--blockage-coverage` so a single
    // invocation can arm all three coverage points in one process, with the RM connection
    // this arm borrows still open underneath it.
    if want_uvm {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        // ⊘ Allocate a REAL VA space first — passing 0 got `NV_ERR_PAGE_TABLE_NOT_AVAIL`
        //   from UVM on bare metal, which is a defect in the client and not a finding.
        let hvas = match rm.host_alloc_vaspace_externally_owned() {
            Ok(h) => {
                println!("ok    W392C hVaSpace      = {h:#010x}");
                h
            }
            Err(e) => {
                println!("FAIL  W392C hVaSpace      = {e:?} — ⊘ the CLIENT could not build a VA");
                println!("      space, so nothing below is a statement about UVM.");
                return std::process::ExitCode::FAILURE;
            }
        };
        let ok = uvm_raw::drive(gpu, rm.host_ctl_fd(), rm.host_client(), hvas);
        println!("done — uvm-invalidate probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        };
    }
    // ★★★★★ w392d — THE MEAN CLIENT. Placed after `--uvm-invalidate` (whose chain it
    // reuses for P2) and before every other rung, because its engine reads wait on a
    // completion semaphore and another rung's channel retiring into the same process
    // would be indistinguishable noise.
    if want_mean {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        // ⚠ Drawn from the clock, PRINTED by `mean::run`, and overridable with
        //   `--mean-nonce` so any failure replays with the same patterns.
        let nonce = mean_nonce.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x4B46_0001, |d| d.subsec_nanos() ^ (d.as_secs() as u32))
                | 1
        });
        let cfg = mean::Cfg {
            gpu,
            threads: mean_threads,
            p1_rounds: mean_p1_rounds,
            nonce,
            falsify: mean_falsify,
        };
        let ok = mean::run(&mut rm, &conn, &cfg);
        println!("done — w392d mean client only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }
    if want_blockage_coverage {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = blockage_coverage(&mut rm, gpu);
        println!("done — blockage-coverage probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    if want_dictated_ring {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = dictated_ring_probe(&mut rm, gpu);
        println!("done — dictated-ring probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★ R32 runs here and RETURNS, for R25's reason exactly: it builds its own memfd, its
    // own two mappings, its own descriptor and its own `Vas`, so every refusal attributes
    // to the join and cannot be an earlier rung's channel or mapping.
    if let Some(seed) = want_fb_join {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = fb_memfd_join_probe(&mut rm, gpu, seed);
        println!("done — fb-memfd-join probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★ R29 runs here and RETURNS, for R25's reason exactly: it builds its own plane, its
    // own descriptor and its own `Vas`, so a refusal attributes to the guest-RAM route and
    // cannot be an earlier rung's channel or mapping.
    if want_guest_pin {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = guest_ram_pin_probe(&mut rm, gpu);
        println!("done — guest-RAM pin probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★ R31 runs here and RETURNS, for R25's reason exactly: it allocates its own `Vas`,
    // its own memfd, its own descriptor and its own channels, and every address it names is
    // a constant no other rung uses — so nothing it observes can be an earlier rung's
    // leftover. ⊘ It schedules nothing and rings nothing, so unlike R30 it cannot fault.
    if want_guest_ring {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = guest_ring_channel_probe(&mut rm, gpu);
        println!("done — guest-ring channel probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★★★★★ w393 — the BAR1 crossing runs here and RETURNS, for R30's reason: its objects
    // and its child process must be the only things in the census.
    if want_bar1_crossing {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = bar1_crossing_probe(&mut rm, gpu);
        println!("done \u{2014} bar1-crossing probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★ R30 (the CPU-view arm) runs here and RETURNS, for R25's reason: it allocates its own
    // address space, its own objects and its own memfd, and leaving the rest of the ladder
    // unrun keeps every refusal attributable to the CPU-view chain alone.
    if let Some(join) = want_fb_view {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = fb_view_probe(&mut rm, gpu, join);
        println!("done \u{2014} fb-cpu-view probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★ R30 runs here and RETURNS, for R25's reason exactly: it allocates its own `Vas`,
    // its own copy-engine channel and its own probe objects, so every answer attributes to
    // the PLACEMENT of the isolate's control structures and cannot be an earlier rung's
    // channel or mapping. It is also the rung that provokes a host fault on its arm C, and
    // a fault must never land in the middle of a ladder someone is reading top to bottom.
    if want_executor_vas {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = executor_vas_probe(&mut rm, gpu, want_executor_alias);
        println!("done — executor-VAS probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    if let Some(seed) = want_osdesc {
        println!(
            "REV_UNDER_TEST={}",
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        let ok = osdesc_probe(&mut rm, gpu, seed);
        println!("done — osdesc probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★ R19 runs here and RETURNS, for R18's reason: it maps and reads, it submits nothing,
    // and leaving the rest of the ladder unrun keeps every refusal attributable to the
    // timer mapping alone rather than to a channel that failed three rungs earlier.
    if want_timer {
        let ok = timer_probe(&conn);
        println!("done — timer probe only");
        return if ok {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }

    // ★ R13c runs here and RETURNS, for R18's reason and one more: its instrument is a
    // *diff*, and every unrelated channel the rest of the ladder allocates is another bit
    // moving under it. Isolation is what makes the attribution hold.
    if want_census {
        println!(
            "REV_UNDER_TEST={}",
            // ★★ Stamped at BUILD time, never read from the box's checkout at run time: a
            // `git rev-parse` in the harness reports whatever the tree says NOW, which is
            // how a suite result once got attributed to a revision it was not built from.
            // Absent is printed as `unstamped` and the consumer refuses it.
            option_env!("KAYFABE_BUILD_REV").unwrap_or("unstamped")
        );
        doorbell_census(&mut rm, subdevice, gpu);
        println!("done — doorbell census only");
        return std::process::ExitCode::SUCCESS;
    }

    // R7 — a per-`Vas` host address space. This is #14's proven fix made real: two guest
    // processes' identical guest VAs publish into DIFFERENT host VASes and cannot collide.
    let vas = match rm.alloc_vaspace() {
        Ok(h) => {
            println!("ok    R7 hVaSpace       = {:#010x}", h.raw());
            h
        }
        Err(e) => {
            println!("FAIL  R7 FERMI_VASPACE_A: {e:?}");
            return std::process::ExitCode::from(1);
        }
    };

    // R8 — system memory.
    const LEN: u64 = 0x10_0000;
    let mem = match rm.alloc_sysmem(LEN) {
        Ok(h) => {
            println!("ok    R8 hMemory        = {:#010x} ({LEN} bytes)", h.raw());
            h
        }
        Err(e) => {
            println!("FAIL  R8 NV_ESC_RM_ALLOC_MEMORY: {e:?}");
            println!("      (the ladder stops here; R9 needs a memory object)");
            let _ = rm.free(vas);
            return std::process::ExitCode::from(1);
        }
    };

    // R9 — map it into the address space from R7, AT A CHOSEN ADDRESS. A non-zero GPU VA
    // out of this is the first end-to-end fact this project has ever had about its own
    // host plane; that the VA is the one we ASKED for is #102's fact, and only hardware
    // can supply it (no mock can, which is how the gap survived).
    //
    // The address is a plausible guest compute VA — high enough to sit above whatever the
    // driver reserves at the bottom of a fresh `FERMI_VASPACE_A`, and 2 MiB-aligned.
    const AT: GpuVa = GpuVa(0x2_0020_0000);
    match rm.map_gpu_va(vas, mem, LEN, AT) {
        Ok(va) if va == AT.0 => {
            println!("ok    R9 host GPU VA    = {va:#018x} (FIXED, as requested)");
        }
        Ok(va) => println!(
            "FAIL  R9 placement       = asked {:#018x}, RM chose {va:#018x} \
             (DMA_OFFSET_FIXED_TRUE not honoured — the data plane cannot work)",
            AT.0
        ),
        Err(e) => println!("FAIL  R9 NV_ESC_RM_MAP_MEMORY_DMA: {e:?}"),
    }

    // A control, on the subdevice, purely to prove the control path encodes: an unknown
    // command must come back as a NAMED RM status rather than as a transport failure.
    let mut payload = [0u8; 4];
    match rm.control(subdevice, ControlCmd(0x2080_0110), &mut payload) {
        Ok(()) => println!("ok    R+ control          = accepted"),
        Err(e) => println!("info  R+ control          = {e:?} (a status, not a transport failure)"),
    }

    // An alloc of a class we do not expect to be permitted, to see the refusal shape.
    match rm.alloc(kayfabe_isolate::HostHandle::NULL, ClassId(0xFFFF), &[]) {
        Ok(h) => println!("info  R+ bogus class      = accepted as {:#010x}", h.raw()),
        Err(e) => println!("ok    R+ bogus class      = refused: {e:?}"),
    }

    // ★★★ R13 — A REAL HOST CHANNEL. Six RM objects, a GPU mapping and two controls.
    //
    // The evidence is the work-submit token: `(runlistId << 16) | chid`, assigned by RM
    // from the GPU's channel RAM. We do not compute it and cannot predict it, and a
    // channel that was never bound to a runlist does not have one — the control answers
    // `NV_ERR_INVALID_STATE` (0x40) instead. So a token here is a fact about hardware.
    //
    // Two channels are allocated, on two different `Vas`es, for one reason: **the tokens
    // must differ.** One token proves a control returned a number; two different tokens
    // prove the number identifies a channel. A backend that returned a constant would
    // pass the first check and fail this one.
    let mut channels = Vec::new();
    for (n, engine) in [(1u32, EngineKind::Ce), (2, EngineKind::GrCompute)] {
        let vas = match rm.alloc_vaspace() {
            Ok(h) => h,
            Err(e) => {
                println!("FAIL  R13.{n} vaspace      = {e:?}");
                break;
            }
        };
        // ⊘ `None` on the notifier: this ladder rung is about which runlist an engine type
        // lands on. `alloc_channel_at_with_error_notifier` is the diagnostic that names one.
        match rm.alloc_channel(vas, engine, None, None, None) {
            Ok((chan, token)) => {
                println!(
                    "ok    R13.{n} channel      = {:#010x}, engine {engine:?}, \
                     token {token:#010x} (runlist {} chid {})",
                    chan.raw(),
                    (token >> 16) & 0xFFFF,
                    token & 0xFFFF
                );
                match rm.schedule(chan) {
                    Ok(()) => println!("ok    R13.{n} schedule     = on the runlist"),
                    Err(e) => println!("FAIL  R13.{n} schedule     = {e:?}"),
                }
                channels.push((chan, vas, token));
            }
            Err(e) => {
                println!("FAIL  R13.{n} channel      = {e:?}");
                let _ = rm.free(vas);
            }
        }
    }
    if channels.len() == 2 {
        let (a, b) = (channels[0].2, channels[1].2);
        if a == b {
            println!(
                "FAIL  R13 token identity  = both channels report {a:#010x} — a token \
                      that does not identify a channel is not evidence"
            );
        } else {
            println!("★     R13 token identity  = {a:#010x} != {b:#010x} (two live channels)");
        }
    }
    // An engine the port cannot place on a runlist must be REFUSED, not sent as zero —
    // `engineType = 0` is the C's proven wrong-runlist bug and it fails three steps later.
    match rm.alloc_channel(
        channels.first().map_or(vas, |c| c.1),
        EngineKind::Other,
        None,
        None,
        // ⊘ `None`: the refusal under test happens before any object is allocated, so a
        // notifier here would name an object nothing ever reaches.
        None,
    ) {
        Err(RmError::Other(s)) if s == kayfabe_isolate_host::rm::NOT_ON_THIS_RUNG => {
            println!("ok    R13 unknown engine  = refused before any object was allocated");
        }
        Ok((h, _)) => println!("FAIL  R13 unknown engine  = accepted as {:#010x}", h.raw()),
        Err(e) => println!("FAIL  R13 unknown engine  = wrong refusal {e:?}"),
    }
    // ★★★ R14 — THE RING, CPU-MAPPED. The mapping itself proves nothing (an anonymous
    // page maps and reads back too), so the evidence is that the bytes are in the GPU's
    // object: written through one mapping, read back through a second, INDEPENDENT one —
    // different descriptor, different mmap context, different address. Two mappings of one
    // anonymous allocation cannot exist.
    if let Some(&(chan, _, _)) = channels.first() {
        const PROBE_OFFSET: u64 = 0x800;
        const PATTERN: u32 = 0xA5A5_1234;
        match rm.prove_ring_is_device_memory(chan, PROBE_OFFSET, PATTERN) {
            Ok((a, b)) if a == PATTERN && b == !PATTERN => println!(
                "★     R14 device memory   = wrote {PATTERN:#010x}/{:#010x} through mapping A, \
                 read both back through an INDEPENDENT mapping B",
                !PATTERN
            ),
            Ok((a, b)) if a == b => println!(
                "FAIL  R14 device memory   = mapping B returned {a:#010x} at BOTH offsets — \
                 a constant, not an aliasing view"
            ),
            Ok((a, b)) => println!(
                "FAIL  R14 device memory   = mapping B saw {a:#010x}/{b:#010x}, wanted \
                 {PATTERN:#010x}/{:#010x} — the mappings do not alias, so the bytes are ours",
                !PATTERN
            ),
            Err(e) => println!("FAIL  R14 device memory   = {e:?}"),
        }
        match rm.userd_cursors(chan) {
            Ok((get, put)) => println!("ok    R14 USERD cursors   = GP_GET {get} GP_PUT {put}"),
            Err(e) => println!("FAIL  R14 USERD cursors   = {e:?}"),
        }
        // The ring is bounded, and the bound is the object's — a store past it must be a
        // refusal here, not a fault or a write into whatever the driver mapped next.
        match rm.ring_load_u32(chan, 0x1_0000) {
            Err(RmError::Other(s)) if s == kayfabe_isolate_host::rm::NOT_IN_THIS_OBJECT => {
                println!("ok    R14 ring bound      = a load past the object is refused BY BOUND");
            }
            Ok(v) => println!("FAIL  R14 ring bound      = read {v:#010x} past the object"),
            Err(e) => println!("FAIL  R14 ring bound      = wrong refusal {e:?}"),
        }
    }

    // ★★★ R15 — THE DOORBELL. One host-FIFO semaphore release, submitted for real.
    //
    // The evidence bar, and nothing below it counts: the semaphore word must go
    // `0 -> payload` AND `GP_GET` must advance to meet `GP_PUT`. A doorbell store that
    // returns without error proves only that a page was writable. `GP_GET` is the one
    // word in the crate hardware writes and we do not.
    //
    // The payload is neither 0 (the sentinel written first) nor the token (which is
    // stored into the doorbell window and could alias), so a false pass is unavailable.
    let mut evidence_failed = false;
    if let Some(&(chan, _, token)) = channels.first() {
        const PAYLOAD: u32 = 0xBEEF_5EA1;
        match rm.submit_semaphore_probe(chan, token, PAYLOAD, std::time::Duration::from_secs(2)) {
            Ok(o) if o.landed(PAYLOAD) => println!(
                "★     R15 SEM LANDED      = sem {:#010x} (want {PAYLOAD:#010x}), \
                 GP_GET {} -> caught GP_PUT {} — the GPU consumed our ring and released \
                 our semaphore",
                o.semaphore, o.gp_get, o.gp_put
            ),
            Ok(o) => {
                evidence_failed = true;
                println!(
                    "FAIL  R15 SEM NEVER LANDED= sem {:#010x} (want {PAYLOAD:#010x}), \
                     GP_GET {} GP_PUT {} — {}",
                    o.semaphore,
                    o.gp_get,
                    o.gp_put,
                    if o.gp_get == 0 && o.gp_put != 0 {
                        "hardware never fetched the entry: USERD is not where the channel \
                         says it is (userdOffset), or the doorbell/token is wrong"
                    } else {
                        "the entry was fetched but the methods did not release"
                    }
                );
            }
            Err(e) => {
                evidence_failed = true;
                println!("FAIL  R15 submit          = {e:?}");
            }
        }
    }

    // ★★★ R17 — A REAL COPY ENGINE MOVES BYTES OF DEVICE MEMORY. Two vidmem buffers, a
    // sentinel in the destination, one `LAUNCH_DMA`, and the destination read back
    // through an independent second mapping. The `before` value is what makes it
    // non-vacuous: the destination provably did not already contain the answer.
    {
        const PATTERN: u32 = 0xC0FF_EE00;
        match rm.prove_ce_copy(vas, PATTERN) {
            Ok(e) if e.copied() => println!(
                "★     R17 CE COPY         = {} bytes: dst[0] {:#010x} -> {:#010x}, \
                 dst[last] {:#010x} (want {:#010x}) — read back through an INDEPENDENT mapping",
                e.bytes, e.before, e.after, e.after_last, e.expect_after_last
            ),
            Ok(e) => {
                evidence_failed = true;
                println!(
                    "FAIL  R17 CE COPY         = dst[0] {:#010x} -> {:#010x} (want {:#010x}), \
                     dst[last] {:#010x} (want {:#010x}); engine sem {:#010x} (want \
                     {:#010x}) GP_GET {} GP_PUT {} — {}",
                    e.before,
                    e.after,
                    e.expect_after,
                    e.after_last,
                    e.expect_after_last,
                    e.submit.semaphore,
                    e.payload,
                    e.submit.gp_get,
                    e.submit.gp_put,
                    if e.submit.gp_get == e.submit.gp_put {
                        "the entry WAS fetched and the methods did nothing: SET_OBJECT \
                         class, subchannel or an operand"
                    } else {
                        "the entry was never fetched: USERD, the token, or the schedule"
                    }
                );
            }
            Err(e) => {
                evidence_failed = true;
                println!("FAIL  R17 CE COPY         = {e:?}");
            }
        }
    }

    if want_engines {
        engines(&mut rm, gpu);
    }

    for (chan, vas, _) in channels {
        match rm.free(chan) {
            Ok(()) => println!("ok    R13 free channel    = group, ring, USERD and mapping"),
            Err(e) => println!("FAIL  R13 free channel    = {e:?}"),
        }
        let _ = rm.free(vas);
    }

    let _ = rm.free(mem);
    let _ = rm.free(vas);

    // ★★★ R10/R11 — the SAME work, through the whole stack: a sandboxed child process, the
    // wire protocol, and `Worker::execute`'s verb chain. Everything above proved the ioctls;
    // this proves the isolate. Skipped (loudly) when the isolate binary is not beside us,
    // because a silent skip is worse than a red run.
    {
        {
            let factory =
                kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real);
            let mut isolate = kayfabe_isolate::IsolateFactory::spawn(&factory, id);
            if isolate.is_retired() {
                println!(
                    "FAIL  R10 isolate         = it did not start (its own RM bring-up failed)"
                );
                return std::process::ExitCode::from(1);
            }
            println!(
                "ok    R10 isolate         = {} workers",
                isolate.pool_size()
            );
            let Some(mut w) = isolate.checkout() else {
                println!("FAIL  R10 checkout        = no worker");
                return std::process::ExitCode::from(1);
            };
            match w.execute(
                &kayfabe_isolate::VerbPlan::Publish {
                    host_vas: None,
                    len: LEN,
                    at: AT,
                },
                &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"),
            ) {
                Ok(kayfabe_isolate::VerbReply::Published {
                    host_va, memory, ..
                }) => {
                    println!(
                        "ok    R11 through-isolate = host GPU VA {host_va:#018x}, hMemory {:#010x}",
                        memory.raw()
                    );
                }
                Ok(other) => println!("FAIL  R11 through-isolate = unexpected reply {other:?}"),
                Err(e) => println!("FAIL  R11 through-isolate = {:?}", e.err),
            }

            // ★★★ R16 — DOES A CPU MAPPING SURVIVE THE SANDBOX? This is the question
            // rung 2 could not answer: `kayfabe-rm-ladder` runs as root, so every mapping
            // above took `RmValidateMmapRequest`'s `osIsAdministrator()` fast path
            // (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:2023-2054`) and never
            // executed the validation code. The isolate has **no capabilities at all**, so
            // it takes the validation path, which walks BAR0 range by range.
            //
            // The Doorbell plan exercises BOTH whitelist rows in one go:
            //   * `alloc_channel` CPU-maps the ring and USERD — the FRAMEBUFFER row,
            //     permitted read-write for a non-admin ("See bug 1784955");
            //   * `ring_doorbell` needs the connection's usermode window — the
            //     `kfifoGetUsermodeMapInfo_HAL` row, the only BAR0 range left READ-WRITE
            //     for a non-admin, which is what lets an ordinary CUDA process ring its
            //     own doorbell.
            //
            // A refusal here is a FINDING, not a licence to weaken the sandbox.
            //
            // ★ The plan is built through `gated_doorbell` — the only constructor — with
            // an EMPTY working set, which passes by design: the gate's content is "every
            // claimed VA is published in THIS Vas", and this submission claims none.
            struct NothingClaimed;
            impl kayfabe_isolate::RingWorkingSet for NothingClaimed {
                fn is_host_published(&self, _va: kayfabe_arch::ids::GpuVa) -> bool {
                    false
                }
            }
            match kayfabe_isolate::VerbPlan::gated_doorbell(
                &NothingClaimed,
                &[],
                None,
                None,
                EngineKind::Ce,
                true,
                // ⊘ `None`: this rung asks whether the gate admits an EMPTY working set.
                // A guest-RAM grant would need a guest, and this driver has none.
                // ⊘ w393 — there is no `adopt` argument any more: a doorbell birth never
                // adopts, by type; a `Passthrough` channel is born at its own alloc.
                None,
            ) {
                Err(u) => println!("FAIL  R16 ring gate       = refused an empty set at {u:?}"),
                Ok(plan) => match w.execute(
                    &plan,
                    &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"),
                ) {
                    Ok(kayfabe_isolate::VerbReply::Doorbell { channel, .. }) => println!(
                        "★     R16 sandboxed doorbell = the capability-less isolate CPU-mapped \
                         the ring, USERD and the usermode BAR0 window, and rang channel {:#010x} \
                         token {:#010x}",
                        channel.map_or(0, |c| c.0.raw()),
                        channel.map_or(0, |c| c.1)
                    ),
                    Ok(other) => println!("FAIL  R16 sandboxed doorbell = unexpected {other:?}"),
                    Err(e) => {
                        evidence_failed = true;
                        println!(
                            "FAIL  R16 sandboxed doorbell = {:?} — the sandbox blocked a \
                             mapping the whitelist predicted it would allow. This is a \
                             FINDING; do not relax the sandbox to make it pass.",
                            e.err
                        );
                    }
                },
            }
            isolate.checkin(w);
        }
    }

    if want_concurrency && !concurrency(gpu, 4, 200) {
        return std::process::ExitCode::from(1);
    }

    // ★ The three ★-evidence rungs are the only ones that set the exit code, and they set
    // it on the *evidence*, not on the call returning. A submission that came back `Ok`
    // with a semaphore that never moved is the failure this whole file exists to catch.
    if evidence_failed {
        println!("done — WITH FAILED EVIDENCE");
        return std::process::ExitCode::from(1);
    }
    println!("done");
    std::process::ExitCode::SUCCESS
}

// =========================================================================================
// ★★★★★ w392c — THE UVM RAW CLIENT: POINT 2's ONLY POSSIBLE KNOWN-POSITIVE
// =========================================================================================

/// ⊘⊘ **THE CLAIM THIS ARM EXISTS TO REFUTE, quoted from this file's own
/// [`blockage_coverage`]:**
///
/// > ⊘ **Emulated doorbell** — **NOT** exercised here, by construction. It needs a
/// > guest-*kernel* channel (UVM's), **which a raw client cannot allocate.**
///
/// That is true and misleading. A raw client cannot *allocate* a kernel channel — and it
/// does not need to. It needs to make **nvidia-uvm allocate one**, which is what
/// `/dev/nvidia-uvm` is for. `UVM_REGISTER_GPU` builds UVM's channel manager, whose
/// `UVM_CHANNEL_TYPE_MEMOPS` pool is where every `MEM_OP`/`MMU_TLB_INVALIDATE` on this
/// chip is pushed (`ogkm-580: kernel-open/nvidia-uvm/uvm_ampere_host.c:255`,
/// `uvm_mmu.c:62`, `:722`).
///
/// ⇒ The sentence turned *"not built"* into *"cannot be done"*, and the one coverage point
/// with no raw-client arm is the one the tree recorded as unreachable.
///
/// # ★★★ WHY IT MUST PASS ON BARE METAL FIRST — the owner's gate, and it is the whole design
///
/// A raw client that fails *inside the guest* is *uninterpretable*: "our device is missing
/// something" and "the client is wrong" produce the identical log. So this arm is written
/// to be run **twice**:
///
/// 1. **on the host, bare metal, against the real driver and a real GPU** — it MUST pass.
///    That run is the client's own known-positive, and it is the only thing that makes a
///    guest-side failure attributable to us.
/// 2. **in the Mode-2 guest** — where every refusal is now ours to explain.
///
/// ⊘ A run that has only ever happened in the guest proves nothing about either side.
///
/// # ⚠ THE `_IOC_SIZE` TRAP, WHICH BITES HERE FOR REAL
///
/// `CharDevice::ioctl` refuses when `_IOC_SIZE(request) > buffer.len()`, because for an
/// `_IOC`-encoded number that is exactly how many bytes the driver may copy. **UVM's
/// numbers are not `_IOC`-encoded** — `UVM_IOCTL_BASE(i) = i` on Linux
/// (`uvm_ioctl.h:40`) — so `UVM_REGISTER_GPU` is the plain integer `37` and decodes to
/// `_IOC_SIZE = 0`, which passes trivially.
///
/// `UVM_INITIALIZE` is the exception and it is the recorded one: `0x3000_0001` decodes to
/// **`_IOC_SIZE = 12288`** against a **16-byte** struct. Our guard would refuse it. The fix
/// is to hand it a 12 288-byte buffer with the params at offset 0 — the driver copies its
/// own `sizeof` and ignores the rest. ⊘ Padding is not a workaround here, it is the
/// *correct* reading of the guard's contract: for a request number that carries no size
/// field we cannot know what the driver will copy, so we must not under-provide.
mod uvm_raw {
    use kayfabe_linux_raw::CharDevice;

    /// `uvm_linux_ioctl.h:32`.
    const UVM_INITIALIZE: u64 = 0x3000_0001;
    /// `uvm_ioctl.h:532`, `UVM_IOCTL_BASE(37)` = `37` on Linux.
    const UVM_REGISTER_GPU: u64 = 37;
    /// `uvm_ioctl.h:400`, `UVM_IOCTL_BASE(25)`.
    const UVM_REGISTER_GPU_VASPACE: u64 = 25;
    /// `uvm_ioctl.h:538` — `UVM_UNREGISTER_GPU`, so the arm leaves no session behind.
    const UVM_UNREGISTER_GPU: u64 = 38;
    /// `uvm_ioctl.h:1087`, `UVM_IOCTL_BASE(75)`. The **secondary** fd that binds an `mm`.
    const UVM_MM_INITIALIZE: u64 = 75;
    /// `nvstatuscodes.h:176`. ⚠ A **WARNING**, and it means SUCCESS — see the call site.
    const NV_WARN_NOTHING_TO_DO: u32 = 0x0001_0006;

    // `UVM_MM_INITIALIZE_PARAMS` (`uvm_ioctl.h:1089-1092`): { NvS32 uvmFd; NV_STATUS rmStatus; }
    const MM_UVMFD: usize = 0;
    const MM_STATUS: usize = 4;
    const MM_LEN: usize = 8;
    /// What `_IOC_SIZE(UVM_INITIALIZE)` decodes to. See the module docs.
    const INITIALIZE_DECODED_SIZE: usize = 12288;

    // ★★★ THE ABI, AS OFFSETS. Each constant cites the struct it encodes, and the layout
    // rule that produced it. `#[repr(C)]` alignment: a field of width W starts at the next
    // multiple of W.
    //
    // `UVM_INITIALIZE_PARAMS` (`uvm_linux_ioctl.h:34-38`): { u64 flags; NV_STATUS rmStatus; }
    const INIT_FLAGS: usize = 0;
    const INIT_STATUS: usize = 8;
    const INIT_LEN: usize = 16;

    // `UVM_REGISTER_GPU_PARAMS` (`uvm_ioctl.h:534-543`):
    //   { NvProcessorUuid gpu_uuid;   // 16 bytes at 0
    //     NvBool numaEnabled;         // 1 byte at 16
    //     ⚠ THREE PADDING BYTES at 17..20 — NvS32 must start 4-aligned
    //     NvS32 numaNodeId;  NvS32 rmCtrlFd;  NvHandle hClient;
    //     NvHandle hSmcPartRef;  NV_STATUS rmStatus; }
    const REG_UUID: usize = 0;
    const REG_NUMA_ENABLED: usize = 16;
    const REG_NUMA_NODE: usize = 20;
    const REG_CTL_FD: usize = 24;
    const REG_HCLIENT: usize = 28;
    const REG_HSMC: usize = 32;
    const REG_STATUS: usize = 36;
    const REG_LEN: usize = 40;

    // `UVM_REGISTER_GPU_VASPACE_PARAMS` (`uvm_ioctl.h:402-409`): uuid, then four 4-byte
    // fields with no padding anywhere (16 is already 4-aligned).
    const VAS_UUID: usize = 0;
    const VAS_CTL_FD: usize = 16;
    const VAS_HCLIENT: usize = 20;
    const VAS_HVASPACE: usize = 24;
    const VAS_STATUS: usize = 28;
    const VAS_LEN: usize = 32;

    // `UVM_UNREGISTER_GPU_PARAMS` (`uvm_ioctl.h:550-554`): uuid then status.
    const UNREG_STATUS: usize = 16;
    const UNREG_LEN: usize = 20;

    /// ★★★ **A HAND-WRITTEN ENCODER, and the crate's `#![forbid(unsafe_code)]` is why —
    /// but it is also the better instrument.**
    ///
    /// The obvious version is `copy_nonoverlapping` off a `#[repr(C)]` struct. This crate
    /// forbids `unsafe`, so that is not available; and the replacement is *more* checkable,
    /// not less. Writing each field at an explicit offset makes the ABI layout — including
    /// **the three padding bytes** C inserts between `NvBool numaEnabled` and
    /// `NvS32 numaNodeId` — a fact stated in our source and testable against
    /// `uvm_ioctl.h`, instead of one silently inherited from whatever the compiler chose.
    ///
    /// ⊘ A wrong offset here would place `rmCtrlFd` where the kernel reads `numaNodeId`,
    /// and UVM would fail with a *plausible* status rather than a loud one. That is exactly
    /// the class of defect this project keeps paying for, so the offsets are named.
    struct Enc(Vec<u8>);

    impl Enc {
        /// `len` is the struct's size; `pad_to` satisfies the `_IOC_SIZE` guard.
        fn new(len: usize, pad_to: usize) -> Self {
            Self(vec![0u8; len.max(pad_to)])
        }
        fn u8_at(&mut self, off: usize, v: u8) {
            self.0[off] = v;
        }
        fn u32_at(&mut self, off: usize, v: u32) {
            self.0[off..off + 4].copy_from_slice(&v.to_ne_bytes());
        }
        fn i32_at(&mut self, off: usize, v: i32) {
            self.0[off..off + 4].copy_from_slice(&v.to_ne_bytes());
        }
        fn u64_at(&mut self, off: usize, v: u64) {
            self.0[off..off + 8].copy_from_slice(&v.to_ne_bytes());
        }
        fn bytes_at(&mut self, off: usize, v: &[u8]) {
            self.0[off..off + v.len()].copy_from_slice(v);
        }
    }

    /// Read the 16 raw UUID bytes back out of an ioctl reply buffer.
    fn status_of(buf: &[u8], off: usize) -> u32 {
        u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
    }

    /// ★ **The GPU's UUID, from the driver's own `/proc` node.**
    ///
    /// ⊘ Deliberately NOT synthesised and NOT guessed: `UVM_REGISTER_GPU` matches on it, and
    /// a wrong UUID fails **loudly** (`NV_ERR_GPU_INVALID_DEVICE`) rather than silently
    /// registering the wrong device — which is why parsing the text form is safe here even
    /// though the byte order is a convention rather than something we control.
    fn gpu_uuid(gpu_index: u32) -> Option<([u8; 16], String)> {
        let dir = std::fs::read_dir("/proc/driver/nvidia/gpus").ok()?;
        let mut entries: Vec<_> = dir.filter_map(Result::ok).map(|e| e.path()).collect();
        entries.sort();
        let path = entries.get(gpu_index as usize)?.join("information");
        let text = std::fs::read_to_string(&path).ok()?;
        let line = text.lines().find(|l| l.contains("GPU UUID"))?;
        let tag = line.split(':').nth(1)?.trim().to_owned();
        let hex: String = tag
            .trim_start_matches("GPU-")
            .chars()
            .filter(char::is_ascii_hexdigit)
            .collect();
        if hex.len() != 32 {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some((out, tag))
    }

    /// Drive `/dev/nvidia-uvm` far enough that nvidia-uvm builds its **channel manager** —
    /// the kernel-client channel pool whose `MEMOPS` type carries every
    /// `MMU_TLB_INVALIDATE`. Returns `true` if every step RM/UVM owed us succeeded.
    ///
    /// `rm_ctrl_fd` / `h_client` must come from an RM connection that stays **open for the
    /// whole call** — UVM dups the session out of them and does not take ownership.
    #[allow(clippy::too_many_lines)]
    pub fn drive(gpu_index: u32, rm_ctrl_fd: i32, h_client: u32, h_va_space: u32) -> bool {
        println!("--- w392c UVM raw client: forcing nvidia-uvm to build a KERNEL channel ---");

        let Some((uuid, uuid_text)) = gpu_uuid(gpu_index) else {
            println!(
                "FAIL  W392C uuid          = ⊘ could not read GPU UUID from /proc — NOT a UVM result"
            );
            return false;
        };
        println!("ok    W392C uuid          = {uuid_text}");

        // ⊘ `CharDevice::openat` needs a held `DevDir`, which is the sandbox capability the
        //   isolate carries and this probe does not. `adopt` is the documented other door:
        //   open through `std`, hand the `OwnedFd` over. No `unsafe`, and the fd's lifetime
        //   is still the `CharDevice`'s.
        let dev = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/nvidia-uvm")
        {
            Ok(f) => CharDevice::adopt(std::os::fd::OwnedFd::from(f)),
            Err(e) => {
                println!("FAIL  W392C open          = /dev/nvidia-uvm: {e}");
                println!(
                    "      ⊘ the node is absent or unopenable. This is NOT a statement about UVM."
                );
                return false;
            }
        };
        println!("ok    W392C open          = /dev/nvidia-uvm");

        // 1. UVM_INITIALIZE — must be the first op on the fd (`uvm_linux_ioctl.h:28-31`).
        //    ⚠ padded to 12288: see the module's `_IOC_SIZE` note.
        let mut enc = Enc::new(INIT_LEN, INITIALIZE_DECODED_SIZE);
        enc.u64_at(INIT_FLAGS, 0);
        let mut buf = enc.0;
        match dev.ioctl(UVM_INITIALIZE, &mut buf, &mut []) {
            Ok(_) => {
                let st = status_of(&buf, INIT_STATUS);
                println!(
                    "{} W392C INITIALIZE    = rmStatus {st:#x}",
                    if st == 0 { "ok   " } else { "FAIL " }
                );
                if st != 0 {
                    return false;
                }
            }
            Err(e) => {
                println!("FAIL  W392C INITIALIZE    = ioctl refused: {e:?}");
                return false;
            }
        }

        // 1b. ★★★★★ UVM_MM_INITIALIZE — THE STEP WHOSE ABSENCE WAS THE WHOLE 0x5d.
        //
        // ⊘⊘ THIRD defect the bare-metal gate caught, and the first two "fixes" were both
        // wrong guesses at it (a null hVaSpace, then an externally-owned VAS). The real
        // contract is stated in `uvm_ioctl.h:1060-1086`: a **secondary** fd holds a
        // reference on the memory map, and without it the va_space has no `mm`, so
        // `uvm_va_space.c:829` sets `disallow_new_registers` and every register answers
        // `NV_ERR_PAGE_TABLE_NOT_AVAIL` — whose own documented meaning (`uvm.h:368`) is
        // *"the UVM file descriptor [must] be associated with a single process"*, not
        // anything about page tables at all.
        //
        // ⚠ THE FD MUST STAY OPEN. *"Once this file-descriptor has been closed the UVM
        //   context is effectively dead"* — so it is bound to a variable that outlives the
        //   registrations below, not dropped at the end of this block.
        // ⚠ AND `NV_WARN_NOTHING_TO_DO` IS SUCCESS. *"Not all platforms require this
        //   secondary file-descriptor. On those platforms NV_WARN_NOTHING_TO_DO will be
        //   returned"* — treating a WARNING as a failure here would make the client refuse
        //   on exactly the platforms where it had nothing left to do.
        let _mm_fd = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/nvidia-uvm")
        {
            Ok(f) => {
                let mm = CharDevice::adopt(std::os::fd::OwnedFd::from(f));
                let mut enc = Enc::new(MM_LEN, 0);
                enc.i32_at(MM_UVMFD, dev.fd_number());
                let mut buf = enc.0;
                match mm.ioctl(UVM_MM_INITIALIZE, &mut buf, &mut []) {
                    Ok(_) => {
                        let st = status_of(&buf, MM_STATUS);
                        let ok = st == 0 || st == NV_WARN_NOTHING_TO_DO;
                        println!(
                            "{} W392C MM_INITIALIZE = rmStatus {st:#x}{}",
                            if ok { "ok   " } else { "FAIL " },
                            if st == NV_WARN_NOTHING_TO_DO {
                                " (NV_WARN_NOTHING_TO_DO — a WARNING that means SUCCESS on \
                                 platforms needing no secondary fd)"
                            } else {
                                ""
                            }
                        );
                        if !ok {
                            return false;
                        }
                    }
                    Err(e) => {
                        println!("FAIL  W392C MM_INITIALIZE = ioctl refused: {e:?}");
                        return false;
                    }
                }
                Some(mm)
            }
            Err(e) => {
                println!("FAIL  W392C MM_INITIALIZE = second /dev/nvidia-uvm open: {e}");
                return false;
            }
        };

        // 2. UVM_REGISTER_GPU — ★ THIS is the step that builds the channel manager, and
        //    therefore the step that makes point 2's transport exist at all.
        let mut enc = Enc::new(REG_LEN, 0);
        enc.bytes_at(REG_UUID, &uuid);
        enc.u8_at(REG_NUMA_ENABLED, 0);
        enc.i32_at(REG_NUMA_NODE, -1);
        enc.i32_at(REG_CTL_FD, rm_ctrl_fd);
        enc.u32_at(REG_HCLIENT, h_client);
        enc.u32_at(REG_HSMC, 0);
        let mut buf = enc.0;
        let registered = match dev.ioctl(UVM_REGISTER_GPU, &mut buf, &mut []) {
            Ok(_) => {
                let st = status_of(&buf, REG_STATUS);
                println!(
                    "{} W392C REGISTER_GPU  = rmStatus {st:#x}",
                    if st == 0 { "ok   " } else { "FAIL " }
                );
                st == 0
            }
            Err(e) => {
                println!("FAIL  W392C REGISTER_GPU  = ioctl refused: {e:?}");
                false
            }
        };
        if !registered {
            println!(
                "      ⊘ no channel manager was built, so a MEMOP census reading zero after this"
            );
            println!("        says nothing about our device.");
            return false;
        }

        // 3. UVM_REGISTER_GPU_VASPACE — grows UVM's page tree, which is where
        //    `uvm_mmu.c:722`'s `tlb_invalidate_all` is pushed.
        let mut enc = Enc::new(VAS_LEN, 0);
        enc.bytes_at(VAS_UUID, &uuid);
        enc.i32_at(VAS_CTL_FD, rm_ctrl_fd);
        enc.u32_at(VAS_HCLIENT, h_client);
        enc.u32_at(VAS_HVASPACE, h_va_space);
        let mut buf = enc.0;
        let vas_ok = match dev.ioctl(UVM_REGISTER_GPU_VASPACE, &mut buf, &mut []) {
            Ok(_) => {
                let st = status_of(&buf, VAS_STATUS);
                println!(
                    "{} W392C REGISTER_VAS  = rmStatus {st:#x}",
                    if st == 0 { "ok   " } else { "FAIL " }
                );
                st == 0
            }
            Err(e) => {
                println!("FAIL  W392C REGISTER_VAS  = ioctl refused: {e:?}");
                false
            }
        };

        // 4. Unregister, so the arm leaves no session pointing at an fd we are about to drop.
        let mut enc = Enc::new(UNREG_LEN, 0);
        enc.bytes_at(REG_UUID, &uuid);
        let mut buf = enc.0;
        if dev.ioctl(UVM_UNREGISTER_GPU, &mut buf, &mut []).is_ok() {
            println!(
                "ok    W392C UNREGISTER     = rmStatus {:#x}",
                status_of(&buf, UNREG_STATUS)
            );
        }

        println!("=== ★★ W392C VERDICT — pre-registered ===");
        if vas_ok {
            println!(
                "    W392C_OUTCOME=(P) ★★★★★ THE CLIENT WORKS. UVM registered the GPU and a VA"
            );
            println!("        space against a RAW RM connection — so nvidia-uvm built its channel");
            println!("        manager, and point 2's transport EXISTS on this run.");
            println!("        ⇒ On BARE METAL this is the client's known-positive.");
            println!(
                "        ⇒ In the GUEST, a MEMOP-CENSUS of zero after this is OURS to explain."
            );
        } else {
            println!("    W392C_OUTCOME=(V) REGISTER_GPU passed, REGISTER_GPU_VASPACE did not.");
            println!("        ⊘ Half a session. The channel manager may exist; the page tree does");
            println!("        not. Do NOT read a later census either way from this.");
        }
        vas_ok
    }

    // ═════════════════════════════════════════════════════════════════════════════════════
    // ★★★★★ w392d — THE PART OF UVM THAT ACTUALLY GROWS A PAGE TREE
    //
    // `drive()` above registers a GPU and a VA space and stops. That is where w392c stopped
    // too, and it is **not enough to make a mapping exist**: registration hands UVM the
    // address space and sets its page directory (`nvGpuOpsSetPageDirectory` →
    // `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`), but the tree under that directory is empty
    // until something is mapped into it.
    //
    // The three ioctls below are what CUDA uses to put something there, and they need **no
    // CPU mapping at all**: `uvm_create_external_range` validates only 4 KiB alignment and
    // inserts a node into the VA space's range tree
    // (`ogkm-580: uvm_map_external.c:600-619`), and `uvm_map_external_allocation` then dups
    // an **RM object** by `{rmCtrlFd, hClient, hMemory}` and writes its PTEs
    // (`:974-1046`). ⊘ So this client can name a GPU VA of its own choosing and have UVM —
    // not RM — publish a mapping there. That is precisely what makes P2's content check a
    // statement about the UVM transport.
    // ═════════════════════════════════════════════════════════════════════════════════════

    /// `uvm_ioctl.h:1042`, `UVM_IOCTL_BASE(73)`.
    const UVM_CREATE_EXTERNAL_RANGE: u64 = 73;
    /// `uvm_ioctl.h:491`, `UVM_IOCTL_BASE(33)`.
    const UVM_MAP_EXTERNAL_ALLOCATION: u64 = 33;
    /// `uvm_ioctl.h:509`, `UVM_IOCTL_BASE(34)`.
    const UVM_FREE: u64 = 34;
    /// `uvm_ioctl.h:425`, `UVM_IOCTL_BASE(27)`.
    const UVM_REGISTER_CHANNEL: u64 = 27;
    /// `uvm_ioctl.h:441`, `UVM_IOCTL_BASE(28)`.
    const UVM_UNREGISTER_CHANNEL: u64 = 28;

    // `UVM_REGISTER_CHANNEL_PARAMS` (`uvm_ioctl.h:427-436`).
    // ⚠ THREE… no, FOUR PADDING BYTES at 28..32: `hChannel` ends at 28 and `base` is
    // `NV_ALIGN_BYTES(8)`. Getting this wrong puts `base` where the kernel reads nothing and
    // `length` where it reads `base` — and for a CE channel, whose resource count is zero,
    // BOTH would be ignored and the call would succeed with a silently wrong struct.
    const CHAN_UUID: usize = 0;
    const CHAN_CTL_FD: usize = 16;
    const CHAN_HCLIENT: usize = 20;
    const CHAN_HCHANNEL: usize = 24;
    const CHAN_BASE: usize = 32;
    const CHAN_LEN: usize = 40;
    const CHAN_STATUS: usize = 48;
    const CHAN_SIZE: usize = 56;

    // `UVM_UNREGISTER_CHANNEL_PARAMS` (`uvm_ioctl.h:443-449`): no 8-byte field, no padding.
    const UNCHAN_HCLIENT: usize = 16;
    const UNCHAN_HCHANNEL: usize = 20;
    const UNCHAN_STATUS: usize = 24;
    const UNCHAN_SIZE: usize = 28;

    // `UVM_CREATE_EXTERNAL_RANGE_PARAMS` (`uvm_ioctl.h:1044-1048`) and
    // `UVM_FREE_PARAMS` (`:511-515`) are the same three fields in the same order.
    const RANGE_BASE: usize = 0;
    const RANGE_LEN: usize = 8;
    const RANGE_STATUS: usize = 16;
    const RANGE_SIZE: usize = 24;

    // `UvmGpuMappingAttributes` (`uvm_types.h:84-94`): `NvProcessorUuid gpuUuid` then five
    // `NvU32`. 16 + 20 = 36, alignment 4, so no padding anywhere.
    const ATTR_STRIDE: usize = 36;
    /// `UVM_MAX_GPUS` = `NV_MAX_DEVICES (32)` × `UVM_PARENT_ID_MAX_SUB_PROCESSORS (8)`
    /// (`uvm_types.h:57,64`, `nvlimits.h:37`). ⚠ It is the **array length in the ABI**, not
    /// a count of anything on this machine — get it wrong and every field after the array
    /// lands somewhere the kernel does not read.
    const UVM_MAX_GPUS: usize = 256;

    // `UVM_MAP_EXTERNAL_ALLOCATION_PARAMS` (`uvm_ioctl.h:493-504`). The three `NvU64`s, then
    // the 9216-byte attribute array, then `gpuAttributesCount` — which is `NV_ALIGN_BYTES(8)`
    // and lands at 9240, already 8-aligned, so again no padding.
    const MAP_BASE: usize = 0;
    const MAP_LEN: usize = 8;
    const MAP_OFFSET: usize = 16;
    const MAP_ATTRS: usize = 24;
    const MAP_ATTR_COUNT: usize = MAP_ATTRS + ATTR_STRIDE * UVM_MAX_GPUS;
    const MAP_CTL_FD: usize = MAP_ATTR_COUNT + 8;
    const MAP_HCLIENT: usize = MAP_CTL_FD + 4;
    const MAP_HMEMORY: usize = MAP_HCLIENT + 4;
    const MAP_STATUS: usize = MAP_HMEMORY + 4;
    const MAP_SIZE: usize = MAP_STATUS + 4;

    /// A live `nvidia-uvm` session: an initialised va_space with a GPU registered in it.
    ///
    /// ⚠ **Both descriptors must outlive every call.** The primary fd *is* the va_space
    /// (`uvm_va_space_get(filp)`); the secondary one holds the reference on the memory map
    /// without which `disallow_new_registers` is set and every register answers
    /// `NV_ERR_PAGE_TABLE_NOT_AVAIL` — the `0x5d` w392c spent two wrong guesses on.
    pub struct Session {
        dev: CharDevice,
        /// Held, never used. ⊘ Named with a leading underscore rather than dropped, because
        /// *"this fd exists to stay open"* is the entire content of the field.
        _mm: CharDevice,
        uuid: [u8; 16],
    }

    impl Session {
        /// `UVM_INITIALIZE` → `UVM_MM_INITIALIZE` → `UVM_REGISTER_GPU`, on a fresh pair of
        /// descriptors.
        ///
        /// # Errors
        /// A human-readable string naming the step and the `rmStatus`. ⊘ Never a bare bool:
        /// *"UVM refused"* and *"the node would not open"* are different findings.
        pub fn open(gpu_index: u32, rm_ctrl_fd: i32, h_client: u32) -> Result<Self, String> {
            let (uuid, _txt) = gpu_uuid(gpu_index).ok_or_else(|| {
                "could not read the GPU UUID from /proc — NOT a UVM result".to_owned()
            })?;
            let dev = open_uvm().map_err(|e| format!("open /dev/nvidia-uvm: {e}"))?;
            let mut enc = Enc::new(INIT_LEN, INITIALIZE_DECODED_SIZE);
            enc.u64_at(INIT_FLAGS, 0);
            call(
                &dev,
                UVM_INITIALIZE,
                enc.0,
                INIT_STATUS,
                "UVM_INITIALIZE",
                &[],
            )?;

            let mm = open_uvm().map_err(|e| format!("open second /dev/nvidia-uvm: {e}"))?;
            let mut enc = Enc::new(MM_LEN, 0);
            enc.i32_at(MM_UVMFD, dev.fd_number());
            // ⚠ `NV_WARN_NOTHING_TO_DO` IS SUCCESS here — see `drive()`'s note.
            call(
                &mm,
                UVM_MM_INITIALIZE,
                enc.0,
                MM_STATUS,
                "UVM_MM_INITIALIZE",
                &[NV_WARN_NOTHING_TO_DO],
            )?;

            let mut enc = Enc::new(REG_LEN, 0);
            enc.bytes_at(REG_UUID, &uuid);
            enc.u8_at(REG_NUMA_ENABLED, 0);
            enc.i32_at(REG_NUMA_NODE, -1);
            enc.i32_at(REG_CTL_FD, rm_ctrl_fd);
            enc.u32_at(REG_HCLIENT, h_client);
            enc.u32_at(REG_HSMC, 0);
            call(
                &dev,
                UVM_REGISTER_GPU,
                enc.0,
                REG_STATUS,
                "UVM_REGISTER_GPU",
                &[],
            )?;
            Ok(Session { dev, _mm: mm, uuid })
        }

        /// `UVM_REGISTER_GPU_VASPACE` — hand UVM an `IS_EXTERNALLY_OWNED` `FERMI_VASPACE_A`.
        ///
        /// ★ After this the address space's page directory is **UVM's page tree**, so a
        /// channel created in it translates through UVM's tables and not RM's.
        ///
        /// # Errors
        /// As [`Session::open`].
        pub fn register_vaspace(
            &self,
            rm_ctrl_fd: i32,
            h_client: u32,
            h_va_space: u32,
        ) -> Result<(), String> {
            let mut enc = Enc::new(VAS_LEN, 0);
            enc.bytes_at(VAS_UUID, &self.uuid);
            enc.i32_at(VAS_CTL_FD, rm_ctrl_fd);
            enc.u32_at(VAS_HCLIENT, h_client);
            enc.u32_at(VAS_HVASPACE, h_va_space);
            call(
                &self.dev,
                UVM_REGISTER_GPU_VASPACE,
                enc.0,
                VAS_STATUS,
                "UVM_REGISTER_GPU_VASPACE",
                &[],
            )
        }

        /// `UVM_CREATE_EXTERNAL_RANGE` — reserve `[base, base+len)` in UVM's range tree.
        ///
        /// # Errors
        /// As [`Session::open`].
        pub fn create_external_range(&self, base: u64, len: u64) -> Result<(), String> {
            let mut enc = Enc::new(RANGE_SIZE, 0);
            enc.u64_at(RANGE_BASE, base);
            enc.u64_at(RANGE_LEN, len);
            call(
                &self.dev,
                UVM_CREATE_EXTERNAL_RANGE,
                enc.0,
                RANGE_STATUS,
                "UVM_CREATE_EXTERNAL_RANGE",
                &[],
            )
        }

        /// `UVM_MAP_EXTERNAL_ALLOCATION` — have **UVM** write the PTEs that put the RM
        /// object `h_memory` at `base`.
        ///
        /// All five mapping attributes are left `0` = the `…Default` member of each enum
        /// (`nv_uvm_user_types.h:65-120`), which is *"let the UVM driver decide"* and is what
        /// CUDA passes for an ordinary allocation.
        ///
        /// # Errors
        /// As [`Session::open`].
        pub fn map_external(
            &self,
            base: u64,
            len: u64,
            rm_ctrl_fd: i32,
            h_client: u32,
            h_memory: u32,
        ) -> Result<(), String> {
            let mut enc = Enc::new(MAP_SIZE, 0);
            enc.u64_at(MAP_BASE, base);
            enc.u64_at(MAP_LEN, len);
            enc.u64_at(MAP_OFFSET, 0);
            // Exactly one GPU's attributes, and the only non-zero field in it is the UUID
            // that selects which registered GPU to map for.
            enc.bytes_at(MAP_ATTRS, &self.uuid);
            enc.u64_at(MAP_ATTR_COUNT, 1);
            enc.i32_at(MAP_CTL_FD, rm_ctrl_fd);
            enc.u32_at(MAP_HCLIENT, h_client);
            enc.u32_at(MAP_HMEMORY, h_memory);
            call(
                &self.dev,
                UVM_MAP_EXTERNAL_ALLOCATION,
                enc.0,
                MAP_STATUS,
                "UVM_MAP_EXTERNAL_ALLOCATION",
                &[],
            )
        }

        /// ★★★★★ **`UVM_REGISTER_CHANNEL` — the step without which a channel in a
        /// UVM-owned address space CANNOT BE SCHEDULED.**
        ///
        /// `[measured 2026-09-08 on this GA106]` creating the channel succeeds and
        /// `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` then answers `0x40`, with RM printing
        /// `kchannelIsSchedulable_IMPL: Cannot schedule externally-owned channel 0x00000009
        /// with unbound allocations!`.
        ///
        /// ⊘ **AND THE STATUS NAME WOULD HAVE SENT ANYONE AT THE WRONG MECHANISM** — the
        /// same trap `UVM_REGISTER_GPU_VASPACE`'s `0x5d` set. The gate is
        /// `kernel_channel.c:2200`: `gvaspaceIsExternallyOwned(pGVAS) && IS_GR(engineDesc)
        /// && !bIsContextBound`. ★ Note `IS_GR` — **this is a COPY channel**, and it still
        /// matches, because `kchannelGetEngine_GM107` resolves the engine from the
        /// **runlist** and *"will pick the first engine on this runlist"*
        /// (`kernel_channel_gm107.c:722-727`): on this part CE0 shares runlist 0 with GR, so
        /// a copy channel is graded by the graphics rule.
        ///
        /// The only writer of `bIsContextBound` reachable from userspace is UVM's
        /// `bind_channel_resources` (`uvm_user_channel.c:686`) → `nvGpuOpsBindChannelResources`
        /// (`nv_gpu_ops.c:10903`), and this ioctl is its door.
        ///
        /// `base`/`length` describe where the channel's *resources* go. ⊘ For a CE channel
        /// they are **unused** — `nv_gpu_ops.c:10855` says *"CE channels have 0 resources, so
        /// they skip this step"*, and `uvm_register_channel_under_write` only touches the
        /// range `if (user_channel->num_resources > 0)`. A range is passed anyway so the call
        /// is correct if that ever stops being true.
        ///
        /// # Errors
        /// As [`Session::open`].
        pub fn register_channel(
            &self,
            rm_ctrl_fd: i32,
            h_client: u32,
            h_channel: u32,
            base: u64,
            len: u64,
        ) -> Result<(), String> {
            let mut enc = Enc::new(CHAN_SIZE, 0);
            enc.bytes_at(CHAN_UUID, &self.uuid);
            enc.i32_at(CHAN_CTL_FD, rm_ctrl_fd);
            enc.u32_at(CHAN_HCLIENT, h_client);
            enc.u32_at(CHAN_HCHANNEL, h_channel);
            enc.u64_at(CHAN_BASE, base);
            enc.u64_at(CHAN_LEN, len);
            call(
                &self.dev,
                UVM_REGISTER_CHANNEL,
                enc.0,
                CHAN_STATUS,
                "UVM_REGISTER_CHANNEL",
                &[],
            )
        }

        /// `UVM_UNREGISTER_CHANNEL`. Best-effort; the caller logs.
        ///
        /// # Errors
        /// As [`Session::open`].
        pub fn unregister_channel(&self, h_client: u32, h_channel: u32) -> Result<(), String> {
            let mut enc = Enc::new(UNCHAN_SIZE, 0);
            enc.bytes_at(CHAN_UUID, &self.uuid);
            enc.u32_at(UNCHAN_HCLIENT, h_client);
            enc.u32_at(UNCHAN_HCHANNEL, h_channel);
            call(
                &self.dev,
                UVM_UNREGISTER_CHANNEL,
                enc.0,
                UNCHAN_STATUS,
                "UVM_UNREGISTER_CHANNEL",
                &[],
            )
        }

        /// `UVM_FREE` — retire a range this session created. Best-effort; the caller logs.
        ///
        /// # Errors
        /// As [`Session::open`].
        pub fn free_range(&self, base: u64, len: u64) -> Result<(), String> {
            let mut enc = Enc::new(RANGE_SIZE, 0);
            enc.u64_at(RANGE_BASE, base);
            enc.u64_at(RANGE_LEN, len);
            call(&self.dev, UVM_FREE, enc.0, RANGE_STATUS, "UVM_FREE", &[])
        }
    }

    /// Open `/dev/nvidia-uvm` read-write through `std` and adopt the descriptor.
    ///
    /// ⊘ [`CharDevice::openat`] needs a held `DevDir`, which is the sandbox capability the
    /// isolate carries and this probe does not.
    fn open_uvm() -> std::io::Result<CharDevice> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/nvidia-uvm")
            .map(|f| CharDevice::adopt(std::os::fd::OwnedFd::from(f)))
    }

    /// One UVM ioctl, with its `rmStatus` decoded and every outcome named.
    ///
    /// ⊘ `also_ok` exists for exactly one caller and it is not a convenience: a **warning**
    /// (`NV_WARN_NOTHING_TO_DO`) is `UVM_MM_INITIALIZE`'s success on platforms that need no
    /// secondary descriptor, and treating it as a failure would make the client refuse
    /// precisely where it had nothing left to do.
    fn call(
        dev: &CharDevice,
        request: u64,
        mut buf: Vec<u8>,
        status_at: usize,
        what: &'static str,
        also_ok: &[u32],
    ) -> Result<(), String> {
        match dev.ioctl(request, &mut buf, &mut []) {
            Ok(_) => {
                let st = status_of(&buf, status_at);
                if st == 0 || also_ok.contains(&st) {
                    Ok(())
                } else {
                    Err(format!("{what}: rmStatus {st:#x}"))
                }
            }
            // ⊘ An ioctl the kernel refused OUTRIGHT and one that ran and answered a status
            //   are different findings, and the message says which.
            Err(e) => Err(format!("{what}: ioctl refused: {e:?}")),
        }
    }
}

// =========================================================================================
// ★★★★★ w392d — THE MEAN CLIENT: a PASS that requires ALL THREE PATHS, CONTENT, AND A RACE
// =========================================================================================

/// ⊘⊘ **WHY w392c's CLIENT WAS TOO KIND, AND IT IS THE SAME DEFECT AS THE LLM GRADE.**
///
/// w392c checks `rmStatus`. The corruption w392 measured on this exact bench was **16 tokens
/// of garbage text with every `rmStatus` clean and zero Xids** — so a status oracle scores
/// the worst outcome we have ever measured as a pass. Owner, same session: *"the raw client
/// also needs to test for corruption of old mappings — and test that the contents is right"*
/// and *"ensure the client could only pass if all three paths are intercepted, and the
/// blocking works, also race through stale mappings, and multithreaded."*
///
/// # ★★★★★ THE ONE ARCHITECTURAL MOVE — an unexercised path is a **FAIL BY NAME**
///
/// *"Could only pass if all three paths are intercepted"* is not a checklist you remember to
/// run; it has to be **structural**, or it decays the first time an arm is skipped. So the
/// verdict is computed from a [`PathLedger`] in which every path starts
/// [`PathState::Unexercised`] and **`PASS` is unreachable unless every one has become
/// `Verified` with zero mismatches**. An arm that is not built yet does not quietly drop out
/// of the grade — it holds the whole client RED and says which one it is.
///
/// ⇒ This is the `orphan_gate_asks_visibility_not_reachability` lesson inverted: rather than
/// hoping the census sees everything, make the *absence of coverage* the loudest thing in
/// the output.
///
/// # ★★★ WHY A CONTENT CHECK CAN SPEAK ABOUT INTERCEPTION AT ALL
///
/// The client runs **in the guest** and cannot read our device's counters. It does not need
/// to. Each path maps a **freshly allocated object at its own VA**, then has the **engine**
/// read it back and compares bytes:
///
/// - we intercepted and published the mapping ⇒ the host GPU has a translation ⇒ right bytes;
/// - we missed it ⇒ no translation (fault) or a **stale** one (wrong bytes).
///
/// ⚠ **The implication holds only because each path's buffer is reachable by that path
/// alone.** That is this client's load-bearing assumption and it is stated here rather than
/// assumed: if two paths ever shared a buffer, a pass would no longer distinguish them.
///
/// # ★★★★ THE STALE RACE — the un-forgeable core
///
/// ```text
///   fill A with pattern_a (CPU)        fill B with pattern_b (CPU)
///   map   VA_X -> A
///   engine reads VA_X                  must be pattern_a
///   unmap VA_X ;  map VA_X -> B        <- THE REMAP
///   engine reads VA_X                  must be pattern_b
///                                      ⊘ if it is pattern_a, a STALE MAPPING is caught
///                                        RED-HANDED: the engine translated VA_X through a
///                                        binding we should have retired.
/// ```
///
/// `pattern_b` is derived from a per-run nonce, so it cannot be precomputed, echoed, or
/// guessed by anything that did not actually read B.
mod mean {
    use super::{HostRmBackend, RmBackend};
    use kayfabe_arch::ids::{ClassId, GpuId, GpuVa};
    use kayfabe_isolate::{HostHandle, IsolateId, RmError};
    use kayfabe_isolate_host::rm::RmConnection;
    use std::sync::Arc;

    /// One coverage point's state. ⊘ There is deliberately **no** `Skipped` — a path is
    /// either verified or it is holding the verdict red.
    #[derive(Debug)]
    pub enum PathState {
        /// Never driven on this run. **This is a FAIL**, and it carries why.
        Unexercised(String),
        /// Driven, and the engine read back exactly what was written, `rounds` times.
        Verified { rounds: u32 },
        /// Driven and the content did not match — the strongest possible signal.
        Mismatch { rounds: u32, first_bad: String },
        /// An ioctl refused before content could be tested. NOT a content result.
        Refused { step: &'static str, status: String },
    }

    impl PathState {
        fn ok(&self) -> bool {
            matches!(self, Self::Verified { .. })
        }
        fn describe(&self) -> String {
            match self {
                Self::Unexercised(why) => format!("⊘ UNEXERCISED — {why}"),
                Self::Verified { rounds } => format!("✔ VERIFIED over {rounds} round(s)"),
                Self::Mismatch { rounds, first_bad } => {
                    format!("★★★ CONTENT MISMATCH after {rounds} round(s): {first_bad}")
                }
                Self::Refused { step, status } => {
                    format!("⊘ REFUSED at {step}: {status} — NOT a content result")
                }
            }
        }
    }

    /// The three points of the owner's coverage ruling, plus the properties that make a pass
    /// mean something.
    pub struct Ledger {
        pub p1_rm_invalidate: PathState,
        pub p2_uvm_memop: PathState,
        pub p3_rpc_bind: PathState,
        /// Did the stale-mapping race run AND come out right?
        pub stale_race: PathState,
        /// How many threads drove concurrent work **and came back clean**. 1 is a FAIL: a
        /// single thread cannot exercise a publication/use race.
        pub threads: u32,
        /// ★★★★★ **EVERY THREAD THAT DID NOT COME BACK CLEAN, BY NAME.** ⊘ Without this the
        /// count above is a *"every row verified"* over the rows that happened to survive:
        /// four workers of which two mismatched would report `threads = 2` and pass the
        /// `>= 2` gate on the strength of the half that worked.
        pub thread_faults: Vec<String>,
        /// How many threads were actually STARTED. ⊘ Printed beside `threads` so *"two
        /// verified"* and *"two ran"* are never the same sentence.
        pub threads_started: u32,
    }

    impl Ledger {
        pub fn new() -> Self {
            Self {
                p1_rm_invalidate: PathState::Unexercised(
                    "no RM map+invalidate round ran".to_owned(),
                ),
                p2_uvm_memop: PathState::Unexercised(
                    "no UVM page-tree grow ran — see w392c: registration alone grows nothing"
                        .to_owned(),
                ),
                p3_rpc_bind: PathState::Unexercised(
                    "no RPC-bound mapping round ran — P3 runs inside P2's UVM session, so a \
                     P2 that stopped early leaves this row at its default"
                        .to_owned(),
                ),
                stale_race: PathState::Unexercised("the remap race did not run".to_owned()),
                threads: 0,
                thread_faults: Vec::new(),
                threads_started: 0,
            }
        }

        /// ★★★★★ **THE VERDICT, AND IT CANNOT SAY PASS ON A PARTIAL RUN.**
        pub fn report(&self) -> bool {
            println!("=== ★★★★★ w392d LEDGER — every row must be ✔ for a PASS ===");
            println!("    P1 rm-invalidate  {}", self.p1_rm_invalidate.describe());
            println!("    P2 uvm-memop      {}", self.p2_uvm_memop.describe());
            println!("    P3 rpc-bind       {}", self.p3_rpc_bind.describe());
            println!("    STALE RACE        {}", self.stale_race.describe());
            println!(
                "    THREADS           {} of {} verified {}",
                self.threads,
                self.threads_started,
                if self.threads >= 2 && self.thread_faults.is_empty() {
                    "✔"
                } else if self.thread_faults.is_empty() {
                    "⊘ ONE THREAD CANNOT RACE — a publish/use race needs a concurrent user"
                } else {
                    "⊘ A WORKER CAME BACK DIRTY — see the faults below"
                }
            );
            for f in &self.thread_faults {
                println!("        ⊘ thread fault: {f}");
            }
            let all = self.p1_rm_invalidate.ok()
                && self.p2_uvm_memop.ok()
                && self.p3_rpc_bind.ok()
                && self.stale_race.ok()
                && self.threads >= 2
                && self.thread_faults.is_empty();
            if all {
                println!("    W392D_OUTCOME=(P) ★★★★★ PASS — all three paths content-verified,");
                println!("        the stale-mapping race came out right, and it was concurrent.");
            } else {
                println!("    W392D_OUTCOME=(F) ⊘ NOT A PASS — at least one row above is not ✔.");
                println!("        ⊘ This is the DESIGNED behaviour of a partial run: an");
                println!("        unexercised path holds the whole client RED rather than");
                println!("        dropping silently out of the grade.");
            }
            all
        }
    }

    /// A per-run pattern nobody can precompute. Derived from the run nonce, the slot and the
    /// round, so no two buffers in a run ever share one and a stale read is unambiguous.
    #[must_use]
    pub fn pattern(nonce: u32, slot: u32, round: u32) -> u32 {
        // A cheap avalanche — the point is distinctness and unguessability, not crypto.
        let mut h = nonce ^ slot.rotate_left(11) ^ round.rotate_left(23) ^ 0x9E37_79B9;
        h ^= h >> 16;
        h = h.wrapping_mul(0x85EB_CA6B);
        h ^= h >> 13;
        // ⊘ Never zero and never the poison: a readback that never ran must not be able to
        //   look like a correct one.
        h | 1
    }

    /// ⊘ Cannot collide with any [`pattern`] — those are always odd (`| 1`).
    const POISON: u32 = 0xDEAD_BEEE;
    /// How long a four-byte copy is given to retire before the read is called UNMEASURED.
    const RETIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
    /// How often the completion semaphore is sampled while waiting.
    const RETIRE_POLL: std::time::Duration = std::time::Duration::from_micros(200);
    /// The mapped length of every object this client tests through. One page.
    const LEN: u64 = 0x1000;

    /// Everything one *lane* — one thread, or the main one — needs to read GPU memory with
    /// the copy engine: a channel, its work-submit token, and a scratch object that is both
    /// GPU-writable and CPU-readable.
    struct Lane {
        chan: HostHandle,
        token: u64,
        scratch: HostHandle,
        scratch_va: u64,
        /// Monotonic per-lane, so no two engine reads ever share a completion payload.
        seq: u32,
    }

    /// ★★★ **THE ENGINE READS THROUGH THE VA — never the CPU, never the handle.**
    ///
    /// Poisons the scratch first, so *"the copy never ran"* cannot look like *"the copy
    /// returned the right value"*, and then **waits for the copy's own completion
    /// semaphore** before reading it.
    ///
    /// ⊘⊘ **TWO DEFECTS THIS FUNCTION CARRIED BEFORE IT WAS EVER RUN, both of which would
    /// have made every row above it meaningless:**
    ///
    /// 1. It called [`HostRmBackend::submit_copy_at`], whose `src` argument is an **offset
    ///    inside the channel's own ring object** and which refuses anything past
    ///    `RING_OBJECT_BYTES`. Passing a full GPU VA there does not read that VA — it is
    ///    refused outright by a bounds check wearing an encode error's name. The verb that
    ///    takes an arbitrary source VA is [`HostRmBackend::submit_copy_va`].
    /// 2. It read the scratch **immediately after submitting**, with nothing waiting for
    ///    retirement. A four-byte CE copy takes microseconds, so the read would *usually*
    ///    have raced ahead of it and seen the poison — reported as *"the copy never
    ///    landed"*, i.e. a red that is the harness's.
    ///
    /// ⚠ The completion payload is **derived per call** and never a constant:
    /// `submit_copy_va` clears the semaphore to `0` before it pushes, so `0` is the
    /// *not-yet* value — and a payload reused between two reads would let the **previous**
    /// copy's retirement satisfy this one's wait.
    fn engine_read_through_va(
        rm: &mut HostRmBackend,
        lane: &mut Lane,
        src_va: u64,
    ) -> Result<u32, String> {
        let (_src_off, sem_off) = HostRmBackend::copy_probe_offsets();
        lane.seq = lane.seq.wrapping_add(1);
        let payload = 0x6D00_0000 | (lane.seq & 0x00FF_FFFF);
        rm.fill_words(lane.scratch, LEN, POISON, 0)
            .map_err(|e| format!("poison scratch: {e:?}"))?;
        rm.submit_copy_va(lane.chan, lane.token, src_va, lane.scratch_va, 4, payload)
            .map_err(|e| {
                format!(
                    "engine copy {src_va:#018x} -> {:#018x}: {e:?}",
                    lane.scratch_va
                )
            })?;
        let deadline = std::time::Instant::now() + RETIRE_TIMEOUT;
        let mut retired = false;
        while std::time::Instant::now() < deadline {
            if matches!(rm.ring_load_u32(lane.chan, sem_off), Ok(v) if v == payload) {
                retired = true;
                break;
            }
            std::thread::sleep(RETIRE_POLL);
        }
        if !retired {
            let seen = rm.ring_load_u32(lane.chan, sem_off);
            return Err(format!(
                "⊘ the copy from {src_va:#018x} NEVER RETIRED — the completion semaphore \
                 never reached {payload:#010x} in {RETIRE_TIMEOUT:?} (it holds {seen:?}). \
                 This is an UNMEASURED read, not a wrong value: the engine may have faulted \
                 on the source VA, in which case the host dmesg carries an Xid 31"
            ));
        }
        let got = rm
            .read_words_independently(lane.scratch, LEN, &[0])
            .map_err(|e| format!("read scratch: {e:?}"))?
            .first()
            .copied()
            .unwrap_or(0);
        if got == POISON {
            return Err(format!(
                "⊘ scratch still holds the poison {POISON:#010x} AFTER a retired copy — the \
                 engine released its semaphore without writing the destination. UNMEASURED."
            ));
        }
        Ok(got)
    }

    /// ★★★★★ **P1 — THE RM MAPPING PATH, AND THE INVALIDATE THAT MUST FOLLOW IT.**
    ///
    /// Each round allocates a **fresh** object, fills it with a **fresh** pattern, maps it
    /// at the **same** VA the last round used, has the engine read it, then unmaps and frees
    /// it. So the round-to-round comparison is exactly the question the row is named for:
    ///
    /// ```text
    ///   round r   : VA -> object_r  (pattern_r)   engine must read pattern_r
    ///   round r+1 : VA -> object_r+1 (pattern_r+1) engine must read pattern_r+1
    ///                                              ⊘ reading pattern_r means the PTE and/or
    ///                                                the TLB entry RM should have retired
    ///                                                at the unmap is still live
    /// ```
    ///
    /// ⚠ The freed object's storage may be handed straight back out, so *"the same bytes"*
    /// is not evidence on its own — which is why every round writes a pattern that no other
    /// round in the run can produce.
    fn p1_rm_round(
        rm: &mut HostRmBackend,
        vas: HostHandle,
        lane: &mut Lane,
        va: u64,
        nonce: u32,
        rounds: u32,
    ) -> PathState {
        for r in 0..rounds {
            let obj = match rm.alloc_probe_local(LEN) {
                Ok(h) => h,
                Err(e) => {
                    return PathState::Refused {
                        step: "alloc_probe_local",
                        status: format!("round {r}: {e:?}"),
                    };
                }
            };
            let p = pattern(nonce, 2, r);
            if let Err(e) = rm.fill_words(obj, LEN, p, 0) {
                let _ = rm.free(obj);
                return PathState::Refused {
                    step: "fill",
                    status: format!("round {r}: {e:?}"),
                };
            }
            let got_va = match rm.map_local_at(vas, obj, LEN, Some(va)) {
                Ok(v) => v,
                Err(e) => {
                    let _ = rm.free(obj);
                    return PathState::Refused {
                        step: "map_local_at",
                        status: format!("round {r} at {va:#018x}: {e:?}"),
                    };
                }
            };
            if got_va != va {
                let _ = rm.unmap_local(vas, got_va);
                let _ = rm.free(obj);
                return PathState::Refused {
                    step: "map_local_at placement",
                    status: format!("round {r}: asked {va:#018x}, RM placed {got_va:#018x}"),
                };
            }
            let seen = engine_read_through_va(rm, lane, va);
            let _ = rm.unmap_local(vas, va);
            let _ = rm.free(obj);
            match seen {
                Err(e) => {
                    return PathState::Refused {
                        step: "engine read @P1 VA",
                        status: format!("round {r}: {e}"),
                    };
                }
                Ok(v) if v != p => {
                    // ★ Name the previous round's pattern explicitly when that is what came
                    //   back: *"wrong value"* and *"the mapping we retired is still live"*
                    //   are different defects and must not share a sentence.
                    let stale = r > 0 && v == pattern(nonce, 2, r - 1);
                    return PathState::Mismatch {
                        rounds: r + 1,
                        first_bad: if stale {
                            format!(
                                "★★★★★ STALE RM MAPPING at {va:#018x}: round {r} read round \
                                 {}'s pattern {v:#010x} instead of its own {p:#010x}. The \
                                 unmap did not retire the translation.",
                                r - 1
                            )
                        } else {
                            format!(
                                "round {r} at {va:#018x}: expected {p:#010x}, got {v:#010x} \
                                 (⊘ not any earlier round's pattern either)"
                            )
                        },
                    };
                }
                Ok(_) => {}
            }
        }
        PathState::Verified { rounds }
    }

    /// ★★★★★ **THE STALE-MAPPING RACE.**
    ///
    /// ```text
    ///   fill A with pattern_a (CPU)        fill B with pattern_b (CPU)
    ///   map   VA_X -> A
    ///   engine reads VA_X                  must be pattern_a
    ///   unmap VA_X ;  map VA_X -> B        <- THE REMAP
    ///   engine reads VA_X                  must be pattern_b
    ///                                      ⊘ if it is pattern_a, a STALE MAPPING is caught
    ///                                        RED-HANDED
    /// ```
    ///
    /// ★ Unlike [`p1_rm_round`], **A stays allocated across the remap**. That is the whole
    /// difference and it is what makes the row worth running beside P1: a stale translation
    /// here points at memory that is still live and still holds `pattern_a`, so the wrong
    /// answer is unambiguous rather than dependent on whether the allocator recycled a page.
    ///
    /// Returns the state to record — never a bare bool, because *"refused"*, *"mismatched"*
    /// and *"never ran"* must not collapse into one word.
    fn stale_race(
        rm: &mut HostRmBackend,
        vas: HostHandle,
        lane: &mut Lane,
        va_x: u64,
        nonce: u32,
        quiet: bool,
    ) -> PathState {
        let a = match rm.alloc_probe_local(LEN) {
            Ok(h) => h,
            Err(e) => {
                return PathState::Refused {
                    step: "alloc A",
                    status: format!("{e:?}"),
                };
            }
        };
        let b = match rm.alloc_probe_local(LEN) {
            Ok(h) => h,
            Err(e) => {
                let _ = rm.free(a);
                return PathState::Refused {
                    step: "alloc B",
                    status: format!("{e:?}"),
                };
            }
        };
        let pa = pattern(nonce, 0, 0);
        let pb = pattern(nonce, 1, 0);
        let done = |rm: &mut HostRmBackend, st: PathState| -> PathState {
            let _ = rm.unmap_local(vas, va_x);
            let _ = rm.free(b);
            let _ = rm.free(a);
            st
        };
        if let Err(e) = rm.fill_words(a, LEN, pa, 0) {
            return done(
                rm,
                PathState::Refused {
                    step: "fill A",
                    status: format!("{e:?}"),
                },
            );
        }
        if let Err(e) = rm.fill_words(b, LEN, pb, 0) {
            return done(
                rm,
                PathState::Refused {
                    step: "fill B",
                    status: format!("{e:?}"),
                },
            );
        }
        if !quiet {
            println!(
                "    w392d stale: A pattern={pa:#010x}  B pattern={pb:#010x}  VA={va_x:#018x}"
            );
        }

        // ---- round 1: VA_X -> A, and the engine must see A ---------------------------
        if let Err(e) = rm.map_local_at(vas, a, LEN, Some(va_x)) {
            return done(
                rm,
                PathState::Refused {
                    step: "map A@VA_X",
                    status: format!("{e:?}"),
                },
            );
        }
        let seen_a = match engine_read_through_va(rm, lane, va_x) {
            Ok(v) => v,
            Err(e) => {
                return done(
                    rm,
                    PathState::Refused {
                        step: "engine read @VA_X (round 1)",
                        status: e,
                    },
                );
            }
        };
        if seen_a != pa {
            return done(
                rm,
                PathState::Mismatch {
                    rounds: 1,
                    first_bad: format!(
                        "round 1 at {va_x:#018x}: expected {pa:#010x}, got {seen_a:#010x}"
                    ),
                },
            );
        }

        // ---- THE REMAP: same VA, different object ------------------------------------
        if let Err(e) = rm.unmap_local(vas, va_x) {
            let _ = rm.free(b);
            let _ = rm.free(a);
            return PathState::Refused {
                step: "unmap VA_X",
                status: format!("{e:?}"),
            };
        }
        if let Err(e) = rm.map_local_at(vas, b, LEN, Some(va_x)) {
            let _ = rm.free(b);
            let _ = rm.free(a);
            return PathState::Refused {
                step: "map B@VA_X",
                status: format!("{e:?}"),
            };
        }

        // ---- round 2: the SAME VA must now read B, and MUST NOT read A ----------------
        let seen_b = match engine_read_through_va(rm, lane, va_x) {
            Ok(v) => v,
            Err(e) => {
                return done(
                    rm,
                    PathState::Refused {
                        step: "engine read @VA_X (round 2)",
                        status: e,
                    },
                );
            }
        };
        if seen_b == pa {
            return done(
                rm,
                PathState::Mismatch {
                    rounds: 2,
                    first_bad: format!(
                        "★★★★★ STALE MAPPING CAUGHT RED-HANDED at {va_x:#018x}: after the \
                         remap the read returned A's pattern {pa:#010x}, not B's {pb:#010x}. \
                         The binding we should have retired is still translating."
                    ),
                },
            );
        }
        if seen_b != pb {
            return done(
                rm,
                PathState::Mismatch {
                    rounds: 2,
                    first_bad: format!(
                        "round 2 at {va_x:#018x}: expected B {pb:#010x}, got {seen_b:#010x} \
                         (⊘ neither A nor B — a THIRD value, which is worse than a stale read)"
                    ),
                },
            );
        }
        done(rm, PathState::Verified { rounds: 2 })
    }

    // ═════════════════════════════════════════════════════════════════════════════════════
    // THE ADDRESS MAP. ★ Every region is ≥ 1 GiB from every other, which is twice the
    // ≥ 512 MiB separation `late_map_race` and `map_stress` document needing: a target
    // adjacent to another region can be covered by a large PTE the neighbour's mapping
    // installed, and then a row passes for a reason that has nothing to do with what it
    // tested. ⊘ The base is clear of `--late-map-race` (0x4–0x7), `--rpc-mixed-allocs`
    // (0xC–0x1A) and `--concurrent-fuzz` (0x20 + lane × 8 GiB) so a single invocation can
    // name several arms.
    // ═════════════════════════════════════════════════════════════════════════════════════
    /// The base of everything this arm names.
    const BASE: u64 = 0x0000_0080_0000_0000;
    /// The main lane's channel ring.
    const MAIN_RING: u64 = BASE;
    /// The main lane's scratch — the ONE object the CPU ever reads.
    const MAIN_SCRATCH: u64 = BASE + 0x4000_0000;
    /// P1's single re-used VA. Every round maps a different object here.
    const P1_VA: u64 = BASE + 0x8000_0000;
    /// The stale race's VA.
    const STALE_VA: u64 = BASE + 0xC000_0000;
    /// The first worker's window; each worker gets [`THREAD_STRIDE`] to itself.
    const THREAD_BASE: u64 = BASE + 0x2_0000_0000;
    /// 8 GiB per worker — the same stride `--concurrent-fuzz` uses, for the same reason.
    const THREAD_STRIDE: u64 = 0x2_0000_0000;
    /// RM's default `FERMI_VASPACE_A` limit on this part. A window past it is refused by
    /// name rather than producing a confident red in which every mapping fails.
    const VAS_LIMIT: u64 = 0x0000_0100_0000_0000;

    /// Build one lane: a channel at a dictated ring VA, scheduled, plus a scratch object at
    /// a dictated VA.
    ///
    /// ⊘ The ring's placement is checked against RM's **[OUT]** `dmaOffset` and not against
    /// the value we asked for — a channel RM quietly relocated runs, rings, and then walks
    /// the MMU into nothing.
    fn build_lane(
        rm: &mut HostRmBackend,
        vas: HostHandle,
        engine_type: u32,
        ring_at: u64,
        scratch_at: u64,
    ) -> Result<Lane, String> {
        let (chan, token) = rm
            .alloc_channel_at(vas, engine_type, Some(GpuVa(ring_at)))
            .map_err(|e| format!("alloc_channel_at {ring_at:#018x}: {e:?}"))?;
        let got = rm.channel_ring_va(chan);
        if got != Some(ring_at) {
            let _ = rm.free(chan);
            return Err(format!(
                "ring placement: asked {ring_at:#018x}, RM's dmaOffset says {got:?}"
            ));
        }
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        let scratch = rm
            .alloc_probe_local(LEN)
            .map_err(|e| format!("alloc scratch: {e:?}"))?;
        let va = rm
            .map_local_at(vas, scratch, LEN, Some(scratch_at))
            .map_err(|e| format!("map scratch at {scratch_at:#018x}: {e:?}"))?;
        if va != scratch_at {
            return Err(format!(
                "scratch placement: asked {scratch_at:#018x}, RM placed {va:#018x}"
            ));
        }
        Ok(Lane {
            chan,
            token,
            scratch,
            scratch_va: scratch_at,
            seq: 0,
        })
    }

    /// What one concurrent worker reports back.
    struct WorkerReport {
        /// `None` = the worker verified both of its rows. `Some` = the reason it did not,
        /// and the reason is the row's own `describe()`.
        fault: Option<String>,
        /// The interval the worker's *graded* work occupied, relative to a shared origin.
        /// ⊘ Two workers whose intervals never intersect sampled no concurrency at all.
        span: (u128, u128),
    }

    /// ★★★★★ **ONE CONCURRENT WORKER — its own backend, its own channel, its own VA window,
    /// the SHARED address space.**
    ///
    /// The address space is shared on purpose. A per-worker VAS would leave RM's per-VAS page
    /// tables uncontended, which is exactly the plane a concurrent map/remap test exists to
    /// stress; the per-worker VA **window** is what keeps a violation attributable to one
    /// worker.
    ///
    /// ⚠ Every worker builds its **own** [`HostRmBackend`] over the shared
    /// [`RmConnection`]. The backend carries per-channel GPFIFO slot cursors, so two workers
    /// sharing one would overwrite each other's ring entries and produce a red that is the
    /// harness's rather than the driver's.
    // ⊘ Nine arguments, and none of them is bundleable without hiding something: the
    //   shared connection, the shared VA space and the shared barrier are three
    //   different sharing disciplines, and a struct wrapping them would read as one.
    #[allow(clippy::too_many_arguments)]
    fn thread_worker(
        tid: usize,
        conn: Arc<RmConnection>,
        vas_raw: u64,
        engine_type: u32,
        gpu: u32,
        nonce: u32,
        rounds: u32,
        barrier: &std::sync::Barrier,
        origin: std::time::Instant,
    ) -> WorkerReport {
        let window = THREAD_BASE + (tid as u64) * THREAD_STRIDE;
        let id = IsolateId::new(tid as u32, GpuId(gpu));
        let vas = HostHandle::new(id, vas_raw);
        let mut rm = HostRmBackend::new(
            id,
            conn,
            Arc::new(kayfabe_isolate_host::ChildExports::new()),
        );
        let mut lane = match build_lane(&mut rm, vas, engine_type, window, window + 0x4000_0000) {
            Ok(l) => l,
            Err(e) => {
                // ⊘ Still join the barrier, or every other worker blocks forever on a
                //   failure that has nothing to do with them and the whole arm hangs.
                barrier.wait();
                return WorkerReport {
                    fault: Some(format!("tid {tid}: lane refused: {e}")),
                    span: (0, 0),
                };
            }
        };
        // ★ Everything above is setup and is NOT part of the measured interval; the barrier
        //   is what makes the intervals below actually overlap rather than merely being on
        //   different threads.
        barrier.wait();
        let t0 = origin.elapsed().as_nanos();
        // ⚠ Each worker's nonce is its own, so a value that crossed between two workers'
        //   windows is recognisable as *whose* it is rather than merely wrong.
        let wnonce = nonce ^ ((tid as u32).wrapping_mul(0x9E37_79B9));
        let p1 = p1_rm_round(
            &mut rm,
            vas,
            &mut lane,
            window + 0x8000_0000,
            wnonce,
            rounds,
        );
        let race = stale_race(&mut rm, vas, &mut lane, window + 0xC000_0000, wnonce, true);
        let t1 = origin.elapsed().as_nanos();
        let fault = if !p1.ok() {
            Some(format!("tid {tid} P1: {}", p1.describe()))
        } else if !race.ok() {
            Some(format!("tid {tid} STALE RACE: {}", race.describe()))
        } else {
            None
        };
        let _ = rm.unmap_local(vas, lane.scratch_va);
        let _ = rm.free(lane.scratch);
        let _ = rm.free(lane.chan);
        WorkerReport {
            fault,
            span: (t0, t1),
        }
    }

    // ═════════════════════════════════════════════════════════════════════════════════════
    // ★★★★★ P2 — THE UVM TRANSPORT, VERIFIED BY THE ENGINE
    //
    // Everything below lives in a **second address space**: a `FERMI_VASPACE_A` allocated
    // `IS_EXTERNALLY_OWNED` and then handed to nvidia-uvm with `UVM_REGISTER_GPU_VASPACE`,
    // at which point its page directory is UVM's page tree
    // (`ogkm-580: nv_gpu_ops.c:8855-8875`, `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`).
    //
    // ★★★ THAT IS WHAT MAKES THE ROW MEAN SOMETHING. The copy engine channel is created
    // **inside that space**, so every VA it translates is walked through UVM's tables. The
    // object under test is placed there by `UVM_MAP_EXTERNAL_ALLOCATION` and by nothing
    // else — `NV_ESC_RM_MAP_MEMORY_DMA` is never called on it, and could not be: RM manages
    // no page tables in an externally-owned space. ⇒ the buffer is reachable through the UVM
    // path ALONE, which is the exclusivity the ledger's whole design turns on.
    // ═════════════════════════════════════════════════════════════════════════════════════

    /// P2's channel ring, in the UVM-owned address space.
    const P2_RING: u64 = 0x0000_0090_0000_0000;
    /// P2's scratch — the one object the CPU reads, and the destination of every engine copy.
    const P2_SCRATCH: u64 = 0x0000_0090_4000_0000;
    /// P2's object under test. Every round maps a different allocation here.
    const P2_DATA: u64 = 0x0000_0090_8000_0000;
    /// Where `UVM_REGISTER_CHANNEL` may place the channel's own resources. ⊘ Unused for a
    /// copy engine — see [`super::uvm_raw::Session::register_channel`] — but named and kept
    /// clear of everything else so the call is correct if that changes.
    const P2_CHANRES: u64 = 0x0000_0090_C000_0000;
    /// The length of that region. 1 MiB, which is far more than a GR channel's resource set.
    const P2_CHANRES_LEN: u64 = 0x0010_0000;

    /// ★★★★★ **THE COPY ENGINE P2 MUST USE, AND WHY IT IS NOT `COPY(0)`.**
    ///
    /// `[measured 2026-09-08, `--engines` R13b on this GA106]` the eight `COPY(i)` engine
    /// types route to **four** runlists: `COPY(0)` and `COPY(1)` → runlist **0**, `COPY(2)` →
    /// 1, `COPY(3)` → 2, `COPY(4)` → 8. Runlist 0 is the **graphics** runlist, and
    /// `kchannelGetEngine_GM107` resolves a channel's engine from its runlist, picking *"the
    /// first engine on this runlist"* — so a `COPY(0)` channel is reported as **GR** by every
    /// RM path that asks.
    ///
    /// That has bitten this row twice, with two different statuses, and neither named it:
    /// - `kchannelIsSchedulable_IMPL` refuses `0x40` because `IS_GR(engineDesc)` is true and
    ///   `bIsContextBound` is false;
    /// - `nvGpuOpsRetainChannel` then refuses `0x31` from `kgrctxGetCtxBufferInfo`, because
    ///   `nvGpuOpsGetChannelEngineType` calls it `UVM_GPU_CHANNEL_ENGINE_TYPE_GR` and goes
    ///   looking for **graphics context buffers a copy channel does not have**
    ///   (`pGrCtxBufferMemDesc != NULL` fails at `kernel_graphics_context.c:821`).
    ///
    /// ⇒ An **async** copy engine, on a runlist of its own, is graded as a CE by both paths.
    /// ⊘ This is not a workaround for a driver bug: on this part `COPY(0)` genuinely *is* the
    /// GRCE, and asking for a GR-runlist channel in an externally-owned VA space without
    /// promoting a GR context is a thing RM is right to refuse.
    const P2_COPY_ENGINE: u32 = 2;

    /// ★★★★★ **P2 — grow nvidia-uvm's page tree, then read what it published WITH THE
    /// ENGINE.**
    ///
    /// ⊘⊘ **WHY REGISTRATION ALONE IS NOT THIS ROW, restated because w392c stopped there.**
    /// `UVM_REGISTER_GPU` + `UVM_REGISTER_GPU_VASPACE` hand UVM an address space and set its
    /// page directory. The tree under that directory is **empty**. A census taken after
    /// registration therefore measures a driver that has been *asked* to own a space and has
    /// not yet been asked to map anything into it — which is exactly the shape of an
    /// unmeasured zero.
    ///
    /// Each round below allocates a fresh RM object, fills a fresh un-guessable pattern into
    /// it, has **UVM** publish it at the same VA the last round used, and then has the copy
    /// engine read that VA. Reading the previous round's pattern is a stale UVM mapping;
    /// reading anything else is a wrong value; reading nothing is `UNMEASURED` and says so.
    #[allow(clippy::too_many_lines)]
    fn p2_uvm_round(
        rm: &mut HostRmBackend,
        gpu: u32,
        nonce: u32,
        rounds: u32,
        // ★★★ P3 IS AN OUT-PARAMETER AND NOT A SECOND RETURN VALUE, and that is deliberate:
        // every early exit above leaves it at the caller's `Unexercised`, so a P2 that
        // stopped before P3 could start reports *"never ran"* for P3 rather than inventing a
        // refusal of its own. ⊘ P3 needs P2's UVM session, P2's address space and P2's copy
        // engine — it is the reader for P3's content check — so the two cannot be separate
        // top-level rows without building all of that twice.
        p3_out: &mut PathState,
    ) -> PathState {
        let ctl_fd = rm.host_ctl_fd();
        let client = rm.host_client();
        let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(P2_COPY_ENGINE) else {
            return PathState::Refused {
                step: "engine type",
                status: format!("COPY({P2_COPY_ENGINE}) is not expressible"),
            };
        };

        // 1 ── the address space UVM will take over. ⊘ Externally owned, or RM has already
        //      populated its page tables and truthfully answers that the page table UVM wants
        //      is not available (`NV_ERR_PAGE_TABLE_NOT_AVAIL`, 0x5d).
        let space = match rm.host_alloc_vaspace_externally_owned() {
            Ok(s) => s,
            Err(e) => {
                return PathState::Refused {
                    step: "alloc externally-owned VA space",
                    status: format!("{e:?}"),
                };
            }
        };

        // 2 ── the UVM session. Both of its descriptors stay open for the whole row.
        let sess = match super::uvm_raw::Session::open(gpu, ctl_fd, client) {
            Ok(s) => s,
            Err(e) => {
                return PathState::Refused {
                    step: "UVM session",
                    status: e,
                };
            }
        };
        if let Err(e) = sess.register_vaspace(ctl_fd, client, space) {
            return PathState::Refused {
                step: "UVM_REGISTER_GPU_VASPACE",
                status: e,
            };
        }
        println!(
            "ok    W392D P2 vaspace    = {space:#010x} registered with nvidia-uvm — its page \
             directory is now UVM's page tree"
        );

        // 3 ── the channel's ring, placed BY UVM. ⚠ The object is allocated first because
        //      `UVM_MAP_EXTERNAL_ALLOCATION` dups it by RM handle, which does not exist
        //      until it does.
        let ring_bytes = HostRmBackend::ring_object_bytes();
        let ring = match rm.alloc_ring_object() {
            Ok(h) => h,
            Err(e) => {
                return PathState::Refused {
                    step: "alloc ring object",
                    status: format!("{e:?}"),
                };
            }
        };
        let Ok(ring_raw) = u32::try_from(ring.raw()) else {
            return PathState::Refused {
                step: "ring handle",
                status: format!("{:#x} is not an RM handle", ring.raw()),
            };
        };
        if let Err(e) = sess.create_external_range(P2_RING, ring_bytes) {
            return PathState::Refused {
                step: "UVM_CREATE_EXTERNAL_RANGE (ring)",
                status: e,
            };
        }
        if let Err(e) = sess.map_external(P2_RING, ring_bytes, ctl_fd, client, ring_raw) {
            return PathState::Refused {
                step: "UVM_MAP_EXTERNAL_ALLOCATION (ring)",
                status: e,
            };
        }
        println!("ok    W392D P2 ring       = UVM published the ring object at {P2_RING:#018x}");

        // 4 ── the channel, INSIDE the UVM-owned space.
        let (chan, token) = match rm.alloc_channel_in_uvm_space(space, engine_type, ring, P2_RING) {
            Ok(c) => c,
            Err(e) => {
                return PathState::Refused {
                    step: "alloc_channel_in_uvm_space",
                    status: format!("{e:?}"),
                };
            }
        };
        // ★★★★★ THE STEP THE FIRST RUN OF THIS ROW WAS MISSING. See
        // `Session::register_channel`: without it RM refuses the schedule with `0x40` and a
        // dmesg line naming `bIsContextBound`, and the ONLY userspace door to that flag is
        // this ioctl.
        let Ok(chan_raw) = u32::try_from(chan.raw()) else {
            let _ = rm.free(chan);
            return PathState::Refused {
                step: "channel handle",
                status: format!("{:#x} is not an RM handle", chan.raw()),
            };
        };
        if let Err(e) = sess.register_channel(ctl_fd, client, chan_raw, P2_CHANRES, P2_CHANRES_LEN)
        {
            let _ = rm.free(chan);
            return PathState::Refused {
                step: "UVM_REGISTER_CHANNEL",
                status: e,
            };
        }
        if let Err(e) = rm.schedule(chan) {
            let _ = sess.unregister_channel(client, chan_raw);
            let _ = rm.free(chan);
            return PathState::Refused {
                step: "schedule (UVM space)",
                status: format!("{e:?}"),
            };
        }
        // ★★★ THE RUNLIST IS CHECKED, NOT ASSUMED. The work-submit token's upper half is the
        //     runlist id (`--engines` R13b: `COPY(2)` → token `0x0001_0007`). A zero there
        //     means this channel landed on the GRAPHICS runlist after all, and every refusal
        //     below would then be the GR rule firing on a copy channel — a red that is the
        //     harness's choice of engine and not a driver result.
        let runlist = (token >> 16) as u32;
        if runlist == 0 {
            let _ = rm.free(chan);
            return PathState::Refused {
                step: "engine runlist",
                status: format!(
                    "COPY({P2_COPY_ENGINE}) landed on runlist 0 (token {token:#010x}) — the                      GRAPHICS runlist. ⊘ HARNESS FAULT: pick an async copy engine, or this                      row measures the GR context rule"
                ),
            };
        }
        println!(
            "ok    W392D P2 channel    = COPY({P2_COPY_ENGINE}) bound to the UVM-owned space,              runlist {runlist} (token {token:#010x}) — NOT the graphics runlist"
        );

        // 5 ── the scratch. GPU-written through UVM, CPU-read by handle.
        let scratch = match rm.alloc_probe_local(LEN) {
            Ok(h) => h,
            Err(e) => {
                let _ = rm.free(chan);
                return PathState::Refused {
                    step: "alloc scratch",
                    status: format!("{e:?}"),
                };
            }
        };
        let scratch_raw = u32::try_from(scratch.raw()).unwrap_or(0);
        let fail = |rm: &mut HostRmBackend, step: &'static str, status: String| -> PathState {
            let _ = rm.free(scratch);
            let _ = rm.free(chan);
            PathState::Refused { step, status }
        };
        if let Err(e) = sess.create_external_range(P2_SCRATCH, LEN) {
            return fail(rm, "UVM_CREATE_EXTERNAL_RANGE (scratch)", e);
        }
        if let Err(e) = sess.map_external(P2_SCRATCH, LEN, ctl_fd, client, scratch_raw) {
            return fail(rm, "UVM_MAP_EXTERNAL_ALLOCATION (scratch)", e);
        }
        let mut lane = Lane {
            chan,
            token,
            scratch,
            scratch_va: P2_SCRATCH,
            seq: 0,
        };

        // 6 ── the graded rounds.
        let mut out = PathState::Verified { rounds };
        for r in 0..rounds {
            let obj = match rm.alloc_probe_local(LEN) {
                Ok(h) => h,
                Err(e) => {
                    out = PathState::Refused {
                        step: "alloc round object",
                        status: format!("round {r}: {e:?}"),
                    };
                    break;
                }
            };
            let obj_raw = u32::try_from(obj.raw()).unwrap_or(0);
            let p = pattern(nonce, 3, r);
            if let Err(e) = rm.fill_words(obj, LEN, p, 0) {
                let _ = rm.free(obj);
                out = PathState::Refused {
                    step: "fill round object",
                    status: format!("round {r}: {e:?}"),
                };
                break;
            }
            if let Err(e) = sess.create_external_range(P2_DATA, LEN) {
                let _ = rm.free(obj);
                out = PathState::Refused {
                    step: "UVM_CREATE_EXTERNAL_RANGE (data)",
                    status: format!("round {r}: {e}"),
                };
                break;
            }
            if let Err(e) = sess.map_external(P2_DATA, LEN, ctl_fd, client, obj_raw) {
                let _ = sess.free_range(P2_DATA, LEN);
                let _ = rm.free(obj);
                out = PathState::Refused {
                    step: "UVM_MAP_EXTERNAL_ALLOCATION (data)",
                    status: format!("round {r}: {e}"),
                };
                break;
            }
            let seen = engine_read_through_va(rm, &mut lane, P2_DATA);
            // ⚠ The range is retired BEFORE the next round creates it again — an overlapping
            //   `UVM_CREATE_EXTERNAL_RANGE` is refused by the range tree, and that refusal
            //   would read as *"UVM would not map"* rather than *"we left the last one up"*.
            let _ = sess.free_range(P2_DATA, LEN);
            let _ = rm.free(obj);
            match seen {
                Err(e) => {
                    out = PathState::Refused {
                        step: "engine read @P2 VA",
                        status: format!("round {r}: {e}"),
                    };
                    break;
                }
                Ok(v) if v != p => {
                    let stale = r > 0 && v == pattern(nonce, 3, r - 1);
                    out = PathState::Mismatch {
                        rounds: r + 1,
                        first_bad: if stale {
                            format!(
                                "★★★★★ STALE UVM MAPPING at {P2_DATA:#018x}: round {r} read \
                                 round {}'s pattern {v:#010x} instead of its own {p:#010x}. \
                                 UVM_FREE did not retire the translation.",
                                r - 1
                            )
                        } else {
                            format!(
                                "round {r} at {P2_DATA:#018x}: expected {p:#010x}, got \
                                 {v:#010x} (⊘ not any earlier round's pattern either)"
                            )
                        },
                    };
                    break;
                }
                Ok(_) => {}
            }
        }

        // ── P3, in the address space P2 just proved, with P2's copy engine as its reader.
        //    ⊘ Gated on P2 having verified: a P3 whose readback engine is itself unproven
        //    would report the reader's failure as the RPC bind's.
        if out.ok() {
            println!("--- W392D P3: the RPC bind (GPU_PROMOTE_CTX), with its negative control ---");
            *p3_out = p3_rpc_bind(rm, &sess, space, ctl_fd, client, &mut lane, nonce, rounds);
        } else {
            *p3_out = PathState::Unexercised(format!(
                "P2 did not verify ({}), and P3's readback is P2's copy engine — a P3 run                  behind an unproven reader would report the reader's failure as the RPC                  bind's",
                out.describe()
            ));
        }

        // 7 ── teardown. ⊘ The channel goes before the UVM session drops: freeing it while
        //      the va_space is being torn down would be a channel in an address space whose
        //      page tables are going away. The externally-owned VA space itself is
        //      deliberately NOT freed here — UVM holds its page directory and the process is
        //      about to exit, and an RM free racing UVM's teardown is a worse failure than a
        //      handle that outlives the run.
        let _ = sess.free_range(P2_SCRATCH, LEN);
        let _ = rm.free(lane.scratch);
        let _ = sess.free_range(P2_RING, ring_bytes);
        let _ = sess.unregister_channel(client, chan_raw);
        let _ = rm.free(lane.chan);
        drop(sess);
        out
    }

    /// P3's GR channel ring, in the same UVM-owned address space P2 built.
    const P3_RING: u64 = 0x0000_0091_0000_0000;
    /// Where the GR channel releases its payload, and where P2's copy engine reads it back.
    const P3_TARGET: u64 = 0x0000_0091_4000_0000;
    /// Where `UVM_REGISTER_CHANNEL` places the GR channel's **context buffers**. ★ Unlike
    /// P2's, this range IS used: `uvm_register_channel_under_write` calls `create_va_ranges`
    /// whenever `num_resources > 0`, which is every GR channel.
    const P3_CHANRES: u64 = 0x0000_0092_0000_0000;
    /// 256 MiB — comfortably more than a GA10x GR context's buffer set.
    const P3_CHANRES_LEN: u64 = 0x1000_0000;
    /// `AMPERE_COMPUTE_B` (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7c0.h:32`). ⚠ A
    /// GA10x constant, named here rather than derived: this row runs on the bench's GA106
    /// and a different part would refuse it **loudly** (`NV_ERR_INVALID_CLASS`), which is the
    /// failure mode to prefer over a silent substitution.
    const AMPERE_COMPUTE_B: u32 = 0xC7C0;
    /// `NV_GR_ALLOCATION_PARAMETERS.version` (`ogkm-580: nvos.h:2717`, *"set to 0x2"*).
    const GR_ALLOC_VERSION: u32 = 2;
    /// `sizeof(NV_GR_ALLOCATION_PARAMETERS)` — four `NvU32`.
    const GR_ALLOC_SIZE: u32 = 16;
    /// ⊘ Distinct from [`POISON`], because this one poisons the **source** of P3's readback
    /// while `POISON` poisons the scratch. Two sentinels, so *"the GR channel never wrote"*
    /// and *"the copy never landed"* can never be confused for one another.
    const P3_POISON: u32 = 0xDEAD_BEEF;
    /// `NV_ERR_INVALID_STATE`, which is what `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` answers for a
    /// channel `kchannelIsSchedulable_IMPL` rejects (`kernel_channel.c:2200`).
    const NV_ERR_INVALID_STATE: u32 = 0x40;

    /// ★★★★★ **P3 — THE RPC BIND, WITH ITS OWN NEGATIVE CONTROL IN THE SAME ROW.**
    ///
    /// # What is actually bound over an RPC here, and why it is not a map
    ///
    /// A GR channel in an **externally-owned** address space cannot run until its context
    /// buffers have been declared to RM by `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` — an
    /// `{bufferId, gpuVirtAddr}` declaration carried to the GSP, not a page-table write. It
    /// is the *first* of the three publish sources `mode2_address_table.md` names, and the
    /// one this repo's CLAUDE.md records the C snooping in flight (`nvkvm_snoop_promote_ctx`).
    ///
    /// The only userspace door to it is `UVM_REGISTER_CHANNEL`, which retains the channel,
    /// maps its resources, and then calls `nvGpuOpsBindChannelResources`
    /// (`nv_gpu_ops.c:10884-10903`) — a `GPU_PROMOTE_CTX` whose `promoteEntry[i].gpuVirtAddr`
    /// is each resource's VA.
    ///
    /// # ★★★ THE NEGATIVE CONTROL, AND IT IS WHAT MAKES THE ROW A MEASUREMENT
    ///
    /// ```text
    ///   A  schedule BEFORE the promote  -> MUST be REFUSED 0x40      [no RPC bind exists]
    ///   B  UVM_REGISTER_CHANNEL          -> the promote happens
    ///   C  schedule AFTER  the promote  -> MUST succeed
    ///   D  the channel EXECUTES: an un-guessable payload lands at a VA
    ///   E  P2's copy engine reads that VA back                        [CONTENT]
    /// ```
    ///
    /// ⊘ Without arm A this row would be *"a GR channel ran"*, which says nothing about the
    /// RPC. With it, the run has measured **both** sides of the binding: the same channel,
    /// the same schedule call, refused before the declaration and accepted after it. That is
    /// the `a_refusal_needs_a_negative_control` lesson applied where it belongs.
    ///
    /// ⚠ **And here is what this row does NOT claim.** The payload is released by a *host*
    /// method, so it is not evidence that the GR engine loaded the promoted context. What is
    /// evidence is arm A: RM refuses to schedule the channel at all until `bIsContextBound`,
    /// and the promote is the only thing that sets it. ⇒ The claim is *"the RPC-carried bind
    /// is what made this channel executable, and it then executed and its bytes are right"*,
    /// not *"we read the contents of a context buffer"*. The context buffers are RM's and
    /// this client cannot read them.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn p3_rpc_bind(
        rm: &mut HostRmBackend,
        sess: &super::uvm_raw::Session,
        space: u32,
        ctl_fd: i32,
        client: u32,
        lane: &mut Lane,
        nonce: u32,
        rounds: u32,
    ) -> PathState {
        // 1 ── a GR channel, ring published by UVM exactly as P2's was.
        let ring_bytes = HostRmBackend::ring_object_bytes();
        let ring = match rm.alloc_ring_object() {
            Ok(h) => h,
            Err(e) => {
                return PathState::Refused {
                    step: "alloc GR ring object",
                    status: format!("{e:?}"),
                };
            }
        };
        let ring_raw = u32::try_from(ring.raw()).unwrap_or(0);
        if let Err(e) = sess.create_external_range(P3_RING, ring_bytes) {
            return PathState::Refused {
                step: "UVM_CREATE_EXTERNAL_RANGE (GR ring)",
                status: e,
            };
        }
        if let Err(e) = sess.map_external(P3_RING, ring_bytes, ctl_fd, client, ring_raw) {
            return PathState::Refused {
                step: "UVM_MAP_EXTERNAL_ALLOCATION (GR ring)",
                status: e,
            };
        }
        let (gr_chan, gr_token) = match rm.alloc_channel_in_uvm_space(
            space,
            kayfabe_abi::submit::ENGINE_TYPE_GRAPHICS,
            ring,
            P3_RING,
        ) {
            Ok(c) => c,
            Err(e) => {
                return PathState::Refused {
                    step: "alloc GR channel in UVM space",
                    status: format!("{e:?}"),
                };
            }
        };
        let gr_raw = u32::try_from(gr_chan.raw()).unwrap_or(0);
        println!(
            "ok    W392D P3 GR channel = {gr_raw:#010x} in the UVM-owned space, token \
             {gr_token:#010x} (runlist {})",
            (gr_token >> 16) as u32
        );

        // 2 ── the compute object. ⊘ Without it the channel has NO graphics context at all,
        //      and `UVM_REGISTER_CHANNEL` answers `0x31` from `kgrctxGetCtxBufferInfo`
        //      (`pGrCtxBufferMemDesc != NULL` fails) — measured on this bench, on the run
        //      before this row existed.
        let mut gr_params = [0u8; 16];
        gr_params[0..4].copy_from_slice(&GR_ALLOC_VERSION.to_ne_bytes());
        gr_params[8..12].copy_from_slice(&GR_ALLOC_SIZE.to_ne_bytes());
        if let Err(e) = rm.alloc(gr_chan, ClassId(AMPERE_COMPUTE_B), &gr_params) {
            let _ = rm.free(gr_chan);
            return PathState::Refused {
                step: "alloc AMPERE_COMPUTE_B on the GR channel",
                status: format!("{e:?}"),
            };
        }
        println!(
            "ok    W392D P3 gr object  = AMPERE_COMPUTE_B allocated — the channel now has a context"
        );

        // 3 ── ARM A: THE NEGATIVE CONTROL. No promote has happened, so RM must refuse.
        let before = rm.schedule(gr_chan);
        match &before {
            Err(RmError::Other(s)) if *s == NV_ERR_INVALID_STATE => {
                println!(
                    "★     W392D P3 arm A      = schedule REFUSED {NV_ERR_INVALID_STATE:#x} \
                     BEFORE the promote — the negative control fired, so arm C's success is \
                     attributable to the RPC bind and to nothing else"
                );
            }
            other => {
                // ⊘ A control that did not fire makes arm C uninterpretable, and saying so is
                //   the whole reason arm A exists. It is NOT graded as a pass either way.
                let _ = rm.free(gr_chan);
                return PathState::Refused {
                    step: "arm A negative control",
                    status: format!(
                        "schedule BEFORE the promote answered {other:?}, not \
                         {NV_ERR_INVALID_STATE:#x}. ⊘ UNINTERPRETABLE: if the channel is \
                         schedulable without a promote then arm C proves nothing about the \
                         RPC bind"
                    ),
                };
            }
        }

        // 4 ── ARM B: the promote itself.
        if let Err(e) = sess.register_channel(ctl_fd, client, gr_raw, P3_CHANRES, P3_CHANRES_LEN) {
            let _ = rm.free(gr_chan);
            return PathState::Refused {
                step: "UVM_REGISTER_CHANNEL (GR) — the GPU_PROMOTE_CTX door",
                status: e,
            };
        }
        println!(
            "ok    W392D P3 arm B      = UVM_REGISTER_CHANNEL accepted — the channel's \
             context buffers were mapped at {P3_CHANRES:#018x} and PROMOTED over RPC"
        );

        // 5 ── ARM C: the same call that was refused in arm A.
        if let Err(e) = rm.schedule(gr_chan) {
            let _ = sess.unregister_channel(client, gr_raw);
            let _ = rm.free(gr_chan);
            return PathState::Refused {
                step: "arm C schedule after the promote",
                status: format!("{e:?} — the promote did not make the channel schedulable"),
            };
        }
        println!(
            "★★★   W392D P3 arm C      = the SAME schedule call now SUCCEEDS. ⇒ the \
             RPC-carried bind is what made this channel executable"
        );

        // 6 ── the target, published by UVM, poisoned before anything runs.
        let target = match rm.alloc_probe_local(LEN) {
            Ok(h) => h,
            Err(e) => {
                let _ = sess.unregister_channel(client, gr_raw);
                let _ = rm.free(gr_chan);
                return PathState::Refused {
                    step: "alloc P3 target",
                    status: format!("{e:?}"),
                };
            }
        };
        let target_raw = u32::try_from(target.raw()).unwrap_or(0);
        let done = |rm: &mut HostRmBackend, st: PathState| -> PathState {
            let _ = sess.free_range(P3_TARGET, LEN);
            let _ = rm.free(target);
            let _ = sess.unregister_channel(client, gr_raw);
            let _ = rm.free(gr_chan);
            st
        };
        if let Err(e) = sess.create_external_range(P3_TARGET, LEN) {
            return done(
                rm,
                PathState::Refused {
                    step: "UVM_CREATE_EXTERNAL_RANGE (P3 target)",
                    status: e,
                },
            );
        }
        if let Err(e) = sess.map_external(P3_TARGET, LEN, ctl_fd, client, target_raw) {
            return done(
                rm,
                PathState::Refused {
                    step: "UVM_MAP_EXTERNAL_ALLOCATION (P3 target)",
                    status: e,
                },
            );
        }

        // 7 ── ARMS D and E, once per round.
        for r in 0..rounds {
            let magic = pattern(nonce, 4, r);
            if let Err(e) = rm.fill_words(target, LEN, P3_POISON, 0) {
                return done(
                    rm,
                    PathState::Refused {
                        step: "poison P3 target",
                        status: format!("round {r}: {e:?}"),
                    },
                );
            }
            if let Err(e) = rm.submit_release_at(gr_chan, gr_token, P3_TARGET, magic) {
                return done(
                    rm,
                    PathState::Refused {
                        step: "GR channel release",
                        status: format!("round {r}: {e:?}"),
                    },
                );
            }
            // ⚠ The GR channel and P2's copy engine are two channels with NO ordering
            //   between them, so the readback is polled rather than taken once. A single
            //   read would report `P3_POISON` on a machine that was merely slow, and
            //   *"the GR channel never ran"* is not a thing to conclude from one sample.
            let deadline = std::time::Instant::now() + RETIRE_TIMEOUT;
            let mut seen;
            loop {
                seen = engine_read_through_va(rm, lane, P3_TARGET);
                if matches!(seen, Ok(v) if v == magic) || std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(RETIRE_POLL);
            }
            match seen {
                Err(e) => {
                    return done(
                        rm,
                        PathState::Refused {
                            step: "engine read @P3 target",
                            status: format!("round {r}: {e}"),
                        },
                    );
                }
                Ok(v) if v == P3_POISON => {
                    return done(
                        rm,
                        PathState::Mismatch {
                            rounds: r + 1,
                            first_bad: format!(
                                "round {r} at {P3_TARGET:#018x}: still the poison \
                                 {P3_POISON:#010x} after {RETIRE_TIMEOUT:?} — the GR channel \
                                 was scheduled but NEVER WROTE. ⊘ An UNMEASURED round, not a \
                                 wrong value"
                            ),
                        },
                    );
                }
                Ok(v) if v != magic => {
                    return done(
                        rm,
                        PathState::Mismatch {
                            rounds: r + 1,
                            first_bad: format!(
                                "round {r} at {P3_TARGET:#018x}: expected {magic:#010x}, got \
                                 {v:#010x}"
                            ),
                        },
                    );
                }
                Ok(_) => {}
            }
        }
        done(rm, PathState::Verified { rounds })
    }

    /// A VA nothing in this run ever maps. ⊘ Far above every other region this arm names, and
    /// under the address space's limit so the refusal is the MMU's and not RM's.
    const FALSIFY_VA: u64 = 0x0000_00A0_0000_0000;
    /// The falsifier's own ring and scratch. Its own channel, because the fault kills it.
    const FALSIFY_RING: u64 = 0x0000_00A0_4000_0000;
    const FALSIFY_SCRATCH: u64 = 0x0000_00A0_8000_0000;

    /// ★★★★★ **THE FALSIFIER — CAN THIS CLIENT'S READER SAY "NO"?**
    ///
    /// Every ✔ above is [`engine_read_through_va`] returning the right word. That is worth
    /// exactly as much as the same function's ability to return *nothing* when there is
    /// nothing to read — and a run in which every source VA is mapped never exercises it.
    /// `a_green_test_can_hold_a_wall_in_place`, and `a_census_zero_needs_a_known_positive`
    /// one step further: a reader that cannot fail is not an oracle.
    ///
    /// So: the same call, against a VA **this run never maps**. The required outcome is a
    /// **refusal** — the engine faults, the completion semaphore never reaches the payload,
    /// and the function reports `NEVER RETIRED`. If it returns a *value* instead, then the
    /// scratch was not really poisoned, or the wait was satisfied by something other than
    /// this submission, and **every row above is vacuous**.
    ///
    /// ⚠ **Opt-in, and it is the last thing the process does.** It provokes a real
    /// `Xid 31 FAULT_PDE` and kills its own channel. That is the measurement, not a side
    /// effect — which is why it is a flag and why it runs on a channel and address space
    /// nothing else in the run shares.
    fn falsifier(rm: &mut HostRmBackend, engine_type: u32) -> bool {
        println!(
            "--- W392D FALSIFIER: the SAME engine read against a VA nothing mapped. A \
             REFUSAL is the pass ---"
        );
        let vas = match rm.alloc_vaspace() {
            Ok(v) => v,
            Err(e) => {
                println!("??    W392D falsifier    = no VA space: {e:?}");
                println!("MEAN_FALSIFIER=NOTRUN");
                return false;
            }
        };
        let mut lane = match build_lane(rm, vas, engine_type, FALSIFY_RING, FALSIFY_SCRATCH) {
            Ok(l) => l,
            Err(e) => {
                println!("??    W392D falsifier    = no lane: {e}");
                println!("MEAN_FALSIFIER=NOTRUN");
                let _ = rm.free(vas);
                return false;
            }
        };
        let out = engine_read_through_va(rm, &mut lane, FALSIFY_VA);
        let ok = match &out {
            Err(why) => {
                println!("★★★   W392D falsifier    = the reader REFUSED, as it must: {why}");
                println!(
                    "      ⇒ the poison, the per-call completion payload and the retirement \
                     wait all do work. The ✔ rows above are not free."
                );
                true
            }
            Ok(v) => {
                println!(
                    "⊘⊘⊘   W392D falsifier    = the reader returned {v:#010x} for a VA THIS \
                     RUN NEVER MAPPED. ⊘⊘ EVERY ROW ABOVE IS VACUOUS: the scratch was not \
                     really poisoned, or the wait was satisfied by something other than this \
                     submission"
                );
                false
            }
        };
        println!("MEAN_FALSIFIER={}", if ok { "PASS" } else { "FAIL" });
        // ⊘ The channel is dead — its engine faulted — so the free is best-effort and its
        //   own failure is not a result.
        let _ = rm.free(lane.scratch);
        let _ = rm.free(lane.chan);
        let _ = rm.free(vas);
        ok
    }

    /// Configuration for one `--uvm-mean` run. Every field is printed before the run.
    pub struct Cfg {
        /// The GPU index — the one the UUID is read for and the one RM is opened on.
        pub gpu: u32,
        /// How many concurrent workers the thread phase starts.
        pub threads: usize,
        /// How many map → engine-read → unmap rounds P1 performs at one VA.
        pub p1_rounds: u32,
        /// The run nonce. Printed, so a failure is replayable.
        pub nonce: u32,
        /// Run [`falsifier`] after the ledger. ⊘ Default OFF: it provokes a real `Xid 31`.
        pub falsify: bool,
    }

    /// ★★★★★ **THE MEAN CLIENT'S ENTRY POINT.**
    ///
    /// Returns the ledger's own verdict and nothing else — the process's exit status is that
    /// verdict, so a harness that reads only `$?` and a reader who reads only the ledger can
    /// never disagree.
    #[allow(clippy::too_many_lines)]
    pub fn run(rm: &mut HostRmBackend, conn: &Arc<RmConnection>, cfg: &Cfg) -> bool {
        let mut led = Ledger::new();
        println!(
            "info  W392D config        = gpu {} threads {} p1_rounds {} nonce {:#010x} euid {}",
            cfg.gpu,
            cfg.threads,
            cfg.p1_rounds,
            cfg.nonce,
            kayfabe_linux_raw::geteuid()
        );
        println!(
            "info  W392D the bar       = EVERY row must be ✔. ⊘ A row this build cannot \
             drive is a FAIL BY NAME and holds the whole client red — it does not drop out \
             of the grade"
        );

        let Some(engine_type) = kayfabe_abi::submit::engine_type_copy(0) else {
            println!("FAIL  W392D engine        = COPY0 is not expressible");
            return led.report();
        };
        let last = THREAD_BASE + (cfg.threads.max(1) as u64) * THREAD_STRIDE;
        if last > VAS_LIMIT {
            println!(
                "FAIL  W392D geometry      = {} workers would place a window at {last:#018x}, \
                 past the address space's {VAS_LIMIT:#018x} limit. ⊘ HARNESS FAULT — lower \
                 --mean-threads",
                cfg.threads
            );
            return led.report();
        }

        let vas = match rm.alloc_vaspace() {
            Ok(v) => v,
            Err(e) => {
                println!("FAIL  W392D vaspace       = {e:?} — ⊘ nothing below is a driver result");
                return led.report();
            }
        };
        let mut lane = match build_lane(rm, vas, engine_type, MAIN_RING, MAIN_SCRATCH) {
            Ok(l) => l,
            Err(e) => {
                println!("FAIL  W392D main lane     = {e}");
                println!("      ⊘ No engine exists on this run, so P1 and the STALE RACE are");
                println!("        UNMEASURED rather than failed. The ledger says so by name.");
                led.p1_rm_invalidate = PathState::Refused {
                    step: "build main lane",
                    status: e.clone(),
                };
                led.stale_race = PathState::Refused {
                    step: "build main lane",
                    status: e,
                };
                return led.report();
            }
        };
        println!(
            "ok    W392D main lane     = channel ring at {MAIN_RING:#018x}, scratch at \
             {MAIN_SCRATCH:#018x} — the engine can be asked questions"
        );

        // ── P1 ────────────────────────────────────────────────────────────────────────
        println!(
            "--- W392D P1: the RM mapping path, {} rounds at one VA ---",
            cfg.p1_rounds
        );
        led.p1_rm_invalidate = p1_rm_round(rm, vas, &mut lane, P1_VA, cfg.nonce, cfg.p1_rounds);
        println!("    P1 → {}", led.p1_rm_invalidate.describe());

        // ── THE STALE RACE ────────────────────────────────────────────────────────────
        println!("--- W392D STALE RACE: remap one VA between two LIVE objects ---");
        led.stale_race = stale_race(rm, vas, &mut lane, STALE_VA, cfg.nonce, false);
        println!("    STALE RACE → {}", led.stale_race.describe());

        // ── P2 ────────────────────────────────────────────────────────────────────────
        println!(
            "--- W392D P2: nvidia-uvm publishes the mapping, {} rounds at one VA ---",
            cfg.p1_rounds
        );
        led.p2_uvm_memop =
            p2_uvm_round(rm, cfg.gpu, cfg.nonce, cfg.p1_rounds, &mut led.p3_rpc_bind);
        println!("    P2 → {}", led.p2_uvm_memop.describe());

        println!("    P3 → {}", led.p3_rpc_bind.describe());

        // ── THE THREAD PHASE ──────────────────────────────────────────────────────────
        println!(
            "--- W392D THREADS: {} workers, ONE shared VA space, a window each ---",
            cfg.threads
        );
        let vas_raw = vas.raw();
        let origin = std::time::Instant::now();
        let barrier = Arc::new(std::sync::Barrier::new(cfg.threads));
        let mut handles = Vec::with_capacity(cfg.threads);
        for tid in 0..cfg.threads {
            let conn = Arc::clone(conn);
            let barrier = Arc::clone(&barrier);
            let (gpu, nonce, rounds) = (cfg.gpu, cfg.nonce, cfg.p1_rounds);
            handles.push(std::thread::spawn(move || {
                thread_worker(
                    tid,
                    conn,
                    vas_raw,
                    engine_type,
                    gpu,
                    nonce,
                    rounds,
                    &barrier,
                    origin,
                )
            }));
        }
        let mut spans: Vec<(u128, u128)> = Vec::new();
        for (tid, h) in handles.into_iter().enumerate() {
            led.threads_started += 1;
            match h.join() {
                // ★★★ A PANIC IN A WORKER IS A RESULT, and swallowing it would turn the
                // loudest possible red into a missing row.
                Err(_) => led.thread_faults.push(format!(
                    "tid {tid} PANICKED — a witness assert or an unwrap"
                )),
                Ok(r) => {
                    if let Some(f) = r.fault {
                        led.thread_faults.push(f);
                    } else {
                        led.threads += 1;
                        spans.push(r.span);
                    }
                }
            }
        }
        // ⊘ ZERO OVERLAP VETOES THE ROW. Workers that ran one after another sampled no
        //   concurrency at all, and calling that a concurrent pass reports a property never
        //   measured.
        let mut overlaps = 0usize;
        for (i, a) in spans.iter().enumerate() {
            for b in &spans[i + 1..] {
                if a.0 < b.1 && b.0 < a.1 {
                    overlaps += 1;
                }
            }
        }
        println!(
            "    THREADS → {}/{} clean, {overlaps} overlapping pair(s) of graded intervals",
            led.threads, led.threads_started
        );
        if led.threads >= 2 && overlaps == 0 {
            led.thread_faults.push(format!(
                "NO_CONCURRENCY_OBSERVED — {} workers came back clean but no two of their \
                 graded intervals intersected, so nothing raced. ⊘ NOT a pass",
                led.threads
            ));
        }

        // ── teardown ──────────────────────────────────────────────────────────────────
        let _ = rm.unmap_local(vas, lane.scratch_va);
        let _ = rm.free(lane.scratch);
        let _ = rm.free(lane.chan);
        let _ = rm.free(vas);

        let verdict = led.report();
        // ⊘ AFTER the ledger, never before, and its result is printed on its own line rather
        //   than folded into the verdict. The falsifier does not grade the driver — it grades
        //   THIS CLIENT'S READER — and a run that mixed the two would report an instrument
        //   failure as a driver failure.
        if cfg.falsify {
            let fals = falsifier(rm, engine_type);
            if !fals {
                println!(
                    "⊘⊘⊘   W392D_OUTCOME QUALIFIED = the falsifier FAILED, so the ledger above \
                     is UNINTERPRETABLE whatever it says"
                );
                return false;
            }
        } else {
            println!(
                "info  W392D falsifier    = NOT RUN (pass --mean-falsify). ⊘ The ledger's ✔ \
                 rows are therefore un-negative-controlled ON THIS RUN"
            );
        }
        verdict
    }
}
