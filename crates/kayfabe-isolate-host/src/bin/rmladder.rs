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
use kayfabe_isolate_host::rm::{HostRmBackend, RmConnection};
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
            println!("★     R20 hBinApi        = {:#010x} (NV_OK, params NULL/0)", h.raw());
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

    // ★ R18 runs here and RETURNS, before R7 — a control probe must not be paid for with
    // a channel, a doorbell and a copy engine. It reads; it allocates nothing; and leaving
    // the rest of the ladder unrun keeps the answer attributable to the control alone.
    if let Some(specs) = want_probe {
        probe_ctrl(&mut rm, subdevice, &specs);
        println!("done — probe only");
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
        match rm.alloc_channel(vas, engine) {
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
    match rm.alloc_channel(channels.first().map_or(vas, |c| c.1), EngineKind::Other) {
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
