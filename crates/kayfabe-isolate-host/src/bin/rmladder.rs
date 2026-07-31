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
    let mut base_f =
        kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real)
            .with_pool_size(1);
    let mut base = kayfabe_isolate::IsolateFactory::spawn(&mut base_f, id(899));
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
    let mut f = kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real)
        .with_pool_size(threads);
    let mut one = kayfabe_isolate::IsolateFactory::spawn(&mut f, id(900));
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
    let mut g = kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real)
        .with_pool_size(1);
    let mut many: Vec<_> = (0..threads)
        .map(|i| kayfabe_isolate::IsolateFactory::spawn(&mut g, id(910 + i as u32)))
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

fn main() -> std::process::ExitCode {
    let mut gpu = 0u32;
    let mut want_concurrency = false;
    let mut want_engines = false;
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
            "--engines" => want_engines = true,
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
    let conn = match RmConnection::open(&dev, GpuId(gpu)) {
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
            let mut factory =
                kayfabe_isolate_host::HostIsolateFactory::new(kayfabe_isolate_host::RmMode::Real);
            let mut isolate = kayfabe_isolate::IsolateFactory::spawn(&mut factory, id);
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
