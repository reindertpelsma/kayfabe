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
    DeviceExportOutcome, FbViewJoin, HostRmBackend, OsDescSeed, RmConnection, ViewCompare,
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
                    let h = w.with_rm(|rm| rm.alloc_vaspace());
                    let end = origin.elapsed().as_nanos();
                    spans.lock().expect("spans").push((t, start, end));
                    if let Ok(h) = h {
                        let _ = w.with_rm(|rm| rm.free(h));
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
        // w287's known-positive was measured on (`[measured 2026-08-13, vh2]` a sysmem
        // notifier was refused natively in both flag settings tried). ⊘ Not a default —
        // see `NotifierAperture`, which refuses to make either arm a fallback.
        match rm.probe_guest_reachability(
            vas,
            p.sem_va,
            kayfabe_isolate_host::rm::NotifierAperture::Vidmem,
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
/// ⊘⊘ **`notifier_aperture` is the caller's and it decides whether arm 4 measures at
/// all.** See `kayfabe_isolate_host::rm::NotifierAperture`: a VIDMEM notifier decodes to
/// `ErrorNotifier::Unreachable` on the guest path, so **no host notifier is attached** and
/// the arm runs, faults, and reports a quiet notifier that means nothing. It is printed on
/// every run, including the quiet ones, because that silence is the hazard.
fn ce_client(
    rm: &mut HostRmBackend,
    gpu: u32,
    want_fault: bool,
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

    // --- arm 4 (opt-in): ask HARDWARE, not RM's allocator --------------------------------
    let arm4 = if !want_fault {
        println!(
            "info  R33 arm 4           = NOT RUN (pass `--ce-client-fault`). It points a real \
             copy engine at an unmapped VA, which provokes `Xid 31 FAULT_PDE` and kills its \
             own channel — opt-in, and it belongs under scripts/bench/host_xid_watch.sh"
        );
        true
    } else {
        census::phase("R33 arm4 hw-fault");
        // ★ Its OWN address space, allocated after arms 1–3 have already been read: a
        // faulted channel must not be able to retract a verdict already printed.
        match rm.alloc_vaspace() {
            Err(e) => {
                println!("FAIL  R33 arm 4 vaspace   = {e:?}");
                false
            }
            Ok(fvas) => {
                // ⊘⊘ NAMED BEFORE THE VERDICT, because the verdict is meaningless without it:
                // this arm is in a THIRD address space — not arm 1's operand space and not
                // arms 2/3's control space. It is NOT a cross-check of arm 3, and an earlier
                // draft of this rung printed one as if it were.
                println!(
                    "info  R33 arm 4 SPACE     = a THIRD, freshly allocated address space \
                     (range {:#010x}) — NOT arm 1's operand space and NOT arms 2/3's control \
                     space. ⊘ Arms 3 and 4 ask the same question about the same NUMBER in \
                     DIFFERENT address spaces, so they can disagree without either being \
                     wrong, and neither corroborates the other",
                    fvas.raw()
                );
                let out = match rm.probe_guest_reachability(fvas, UNMAPPED_VA, notifier_aperture) {
                    Ok(r) => {
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
                                 NOTHING about whether the address resolves",
                                    r.control.semaphore,
                                    r.control.gp_get,
                                    r.control.gp_put,
                                    r.control_read,
                                    r.control_want
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
    println!("  --- THE SEQUENCE, in order (seq: nr name  size  phase  errno):");
    for r in &c.log {
        println!(
            "      {:>4}: nr {:>3} {:<26} size {:>5}  {:<22} {}",
            r.seq,
            r.nr,
            nv_esc_name(r.magic, r.nr),
            r.size,
            r.phase,
            if r.errno == 0 {
                "ok".to_string()
            } else {
                format!("errno {}", r.errno)
            }
        );
    }
    println!("=== END IOCTL CENSUS ===");
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

fn main() -> std::process::ExitCode {
    let mut gpu = 0u32;
    let mut want_concurrency = false;
    let mut want_engines = false;
    let mut want_census = false;
    let mut want_timer = false;
    let mut want_probe: Option<Vec<(u32, usize)>> = None;
    let mut want_binapi: Option<Vec<(u32, usize)>> = None;
    let mut want_gpu_info = false;
    let mut want_bus_info = false;
    let mut want_atomics = false;
    let mut want_pce_mask = false;
    let mut want_osdesc: Option<OsDescSeed> = None;
    let mut want_fb_join: Option<OsDescSeed> = None;
    let mut want_dictated_ring = false;
    let mut want_dictated_neg = false;
    let mut want_guest_pin = false;
    let mut want_guest_ring = false;
    let mut want_executor_vas = false;
    let mut want_executor_alias = false;
    let mut want_fb_view: Option<FbViewJoin> = None;
    let mut want_ce_client = false;
    let mut want_ce_client_fault = false;
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
        let ok = ce_client(&mut rm, gpu, want_ce_client_fault, notifier_aperture);
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
            match w.execute(&kayfabe_isolate::VerbPlan::Publish {
                host_vas: None,
                len: LEN,
                at: AT,
            }) {
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
                None,
            ) {
                Err(u) => println!("FAIL  R16 ring gate       = refused an empty set at {u:?}"),
                Ok(plan) => match w.execute(&plan) {
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
