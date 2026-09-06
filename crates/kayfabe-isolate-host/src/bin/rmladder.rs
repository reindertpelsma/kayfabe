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
                    let h = w.with_rm(&kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"), |rm| rm.alloc_vaspace());
                    let end = origin.elapsed().as_nanos();
                    spans.lock().expect("spans").push((t, start, end));
                    if let Ok(h) = h {
                        let _ = w.with_rm(&kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb"), |rm| rm.free(h));
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
            "{}  R5b foreign handle  = client B mapping A's raw object {:#010x} into B's \
             OWN address space: {}",
            if foreign_refused { "ok   " } else { "FAIL " },
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
        "info  R5b census          = controls A={control_a} B={control_b}  B-sees-A's-VA-free\
         ={n1_free}  leak A<-B={leak_a_saw_b}  leak B<-A={leak_b_saw_a}  ordering={ordering_ok}\
           foreign handle refused={foreign_refused}"
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
    let mut want_atomics = false;
    let mut want_pce_mask = false;
    let mut want_osdesc: Option<OsDescSeed> = None;
    let mut want_fb_join: Option<OsDescSeed> = None;
    let mut want_dictated_ring = false;
    let mut want_dictated_neg = false;
    let mut want_late_map_race = false;
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
    // ⊘ The DEFAULT IS THE w379 PRIMITIVE, so every committed w379 arm stays byte-comparable
    // to its own predecessors. `--probe-launch-dma` is the only way to change it and the
    // choice is printed before any rung runs — a run that does not say which primitive it
    // used cannot be compared to any other run.
    let mut probe = W381Probe::SemRelease;
    let mut want_guest_pin = false;
    let mut want_guest_ring = false;
    let mut want_executor_vas = false;
    let mut want_executor_alias = false;
    let mut want_fb_view: Option<FbViewJoin> = None;
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
            }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb")) {
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
                Ok(plan) => match w.execute(&plan, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb")) {
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
