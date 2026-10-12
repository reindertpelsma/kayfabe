//! Native P0 probe for the Windows user-ring design, not a VM submission path.
//! Public NVOS46 permissions; USER channel; USERD has no GPU VA mapping.
//! The writable output is a probe fixture standing in for guest-owned output,
//! not permission to put a writable host fence in a Passthrough space.
use kf_abi::submit::{USERD_GP_GET, USERD_GP_PUT, fifo, gp_entry, method_header_inc};
use kf_host::{HostRm, MapNode, MapPerm, VaSpace, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::time::{Duration, Instant};

const BYTES: u64 = 64 << 10;
const GP: u64 = 0x1000;
const PB: u64 = 0x4000;
const SENTINEL: u64 = 0x8000;
const INITIAL: u32 = 0x3141_5926;
const ATTEMPT: u32 = 0x2718_2818;

/// Same immutable commands in both reference cases; only the acquire fixture's
/// initial value/publication time differs. No writable private GPU fence.
fn reference_commands(output_va: u64, ce_class: u32) -> Result<Vec<u32>, String> {
    let mut words = vec![method_header_inc(0, fifo::SEM_ADDR_LO, 5).ok_or("acquire")?];
    words.extend_from_slice(&[
        output_va as u32,
        (output_va >> 32) as u32,
        1,
        0,
        fifo::SEM_EXECUTE_ACQUIRE_32BIT,
    ]);
    words.extend(
        kf_harness::ce_copy_push(
            ce_class,
            output_va + 0x1000,
            output_va + 0x2000,
            4096,
            output_va + 4,
            1,
            false,
        )
        .ok_or("copy encoding")?,
    );
    words.extend([
        method_header_inc(0, fifo::WFI_C56F, 1).ok_or("WFI")?,
        fifo::WFI_SCOPE_ALL_C56F,
        method_header_inc(0, fifo::SET_REFERENCE_C56F, 1).ok_or("reference")?,
        1,
    ]);
    Ok(words)
}

/// Public C56F LEVEL bit only; this bench does not enable any VM acceptance.
fn reference_descriptor(va: u64, bytes: u64, subroutine: bool) -> Result<u64, String> {
    let entry = gp_entry(va, bytes).ok_or("GP encoding")?;
    Ok(entry | (u64::from(subroutine) << 41))
}

fn run_reference(held: bool, split: bool, subroutine: bool) -> Result<(), String> {
    use kf_abi::submit::{ENGINE_TYPE_COPY0, USERD_REFERENCE_C56F};
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let (architecture, implementation, _) = rm.arch_info();
    let chosen =
        kf_chip::choose_host_classes(architecture, implementation, rm.supported_class_ids())?;
    let class = chosen.gpfifo_channel().channel_id().0;
    if class != kf_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A {
        return Err(format!(
            "REFERENCE_CLASS_UNMEASURED class={class:#x}; probe only supports publicC56F"
        ));
    }
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let ring = Buffer::new(&rm, space, Some(Mode::ReadOnly.perm()))?;
    let output = Buffer::new(&rm, space, Some(MapPerm::READ_WRITE))?;
    let userd = Buffer::new(&rm, space, None)?;
    let ring_va = ring.va.ok_or("ring VA absent")?;
    let output_va = output.va.ok_or("output VA absent")?;
    output.store(0, u32::from(!held))?;
    for off in (0..4096).step_by(4) {
        output.store(0x1000 + off, INITIAL ^ off as u32)?;
    }
    let chan = rm
        .birth_channel(
            space,
            ENGINE_TYPE_COPY0,
            kf_host::RingSpec {
                gp_fifo_va: ring_va + GP,
                gp_fifo_entries: 32,
                userd_memory: userd.memory,
                userd_offset: 0,
                err_notifier: 0,
            },
        )
        .map_err(|e| format!("birth: {e:?}"))?;
    let result = (|| {
        chan.assert_user().map_err(str::to_string)?;
        rm.alloc_ce_object(chan, ENGINE_TYPE_COPY0)
            .map_err(|e| format!("CE: {e:?}"))?;
        println!(
            "USER_RING_REFERENCE_SETUP held={held} split={split} first_subroutine={subroutine} class={class:#x} driver={} ring_gpu_read_only=true userd_gpu_mapped=false",
            rm.driver_version()
        );
        let words = reference_commands(output_va, rm.ce_class_id())?;
        for (i, &word) in words.iter().enumerate() {
            ring.store(PB + 4 * i as u64, word)?;
        }
        let split_words = if split { words.len() - 1 } else { words.len() };
        let gp = reference_descriptor(ring_va + PB, 4 * split_words as u64, subroutine)?;
        ring.store(GP, gp as u32)?;
        ring.store(GP + 4, (gp >> 32) as u32)?;
        let published_put = if split {
            // Only change versus the reference: MAIN GP0 ends at the
            // SET_REFERENCE header; MAIN GP1 contains its one argument.
            // Command bytes, permissions, channel and acquire stimulus match.
            let next = gp_entry(ring_va + PB + 4 * split_words as u64, 4)
                .ok_or("continuation GP encoding")?;
            ring.store(GP + 8, next as u32)?;
            ring.store(GP + 12, (next >> 32) as u32)?;
            2
        } else {
            1
        };
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        kf_linux_raw::release_fence();
        userd.store(USERD_GP_PUT, published_put)?;
        kf_linux_raw::release_fence();
        rm.doorbell(chan.token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
        let start = Instant::now();
        let mut early_reference = false;
        let mut held_samples = 0;
        let mut fetched_while_held = false;
        if held {
            // Explicit bench stimulus, not a timer releasing a product gate.
            // The acquire is intentionally unsatisfied for50ms, then published.
            while start.elapsed() < Duration::from_millis(50) {
                early_reference |= userd.load(USERD_REFERENCE_C56F)? != 0;
                fetched_while_held |= userd.load(USERD_GP_GET)? != 0;
                held_samples += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            println!(
                "USER_RING_REFERENCE_HELD samples={held_samples} early_reference={early_reference} fetched={fetched_while_held} acquire={} reference={} ce_semaphore={}",
                output.load(0)?,
                userd.load(USERD_REFERENCE_C56F)?,
                output.load(4)?
            );
            output.store(0, 1)?;
            kf_linux_raw::release_fence();
        }
        let finished = loop {
            if userd.load(USERD_REFERENCE_C56F)? == 1 {
                break true;
            }
            if start.elapsed() > Duration::from_secs(3) {
                break false;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let mut copy_matches = true;
        for off in (0..4096).step_by(4) {
            copy_matches &= output.load(0x2000 + off)? == (INITIAL ^ off as u32);
        }
        println!(
            "USER_RING_REFERENCE_OBSERVATION finished={finished} reference={} copy_matches={copy_matches} ce_semaphore={} GP_GET={} GP_PUT={} elapsed_us={}",
            userd.load(USERD_REFERENCE_C56F)?,
            output.load(4)?,
            userd.load(USERD_GP_GET)?,
            userd.load(USERD_GP_PUT)?,
            start.elapsed().as_micros()
        );
        if !finished
            || !copy_matches
            || early_reference
            || output.load(4)? != 1
            || (held && held_samples == 0)
            || userd.load(USERD_GP_GET)? != published_put
            || userd.load(USERD_GP_PUT)? != published_put
        {
            return Err("reference/copy/order criterion failed".into());
        }
        Ok(())
    })();
    rm.free_channel(chan)
        .map_err(|e| format!("channel free: {e:?}"))?;
    let mut cleanup = Ok(());
    for b in [ring, output, userd] {
        if let Err(e) = b.close(&rm, space) {
            cleanup = Err(e);
        }
    }
    rm.free_vaspace(space);
    println!("USER_RING_REFERENCE_CLEANUP result={cleanup:?}");
    result.and(cleanup)
}

/// Public DMA_NOP padding changes only fixture length; no synthetic completion.
fn padded_storage_prefix(mut words: Vec<u32>) -> Result<Vec<u32>, String> {
    const WORDS: usize = (100 << 10) / 4;
    if words.len() > WORDS {
        return Err("source fixture too large".into());
    }
    words
        .try_reserve_exact(WORDS - words.len())
        .map_err(|_| "source fixture allocation")?;
    words.resize(WORDS, 0);
    Ok(words)
}

/// Native helper test and one-variable HOST-processing witness discriminator.
/// This is an authored bench fixture, never a guest execution adapter.
fn run_storage(
    sync_wait: bool,
    control_barrier: bool,
    phased: bool,
    assembled: bool,
    barrier_level: bool,
    large_source: bool,
    subdevice: bool,
) -> Result<(), String> {
    use kf_abi::submit::ENGINE_TYPE_COPY0;
    use kf_chan::user_storage::Storage;
    let uid = kf_linux_raw::effective_uid();
    println!("USER_STORAGE_IDENTITY uid={uid}");
    if uid != 65534 {
        return Err("native storage probe requires unprivileged uid65534".into());
    }
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let (architecture, implementation, _) = rm.arch_info();
    let chosen =
        kf_chip::choose_host_classes(architecture, implementation, rm.supported_class_ids())?;
    let class = chosen.gpfifo_channel().channel_id().0;
    if class != kf_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A {
        return Err(format!("STORAGE_CLASS_UNMEASURED class={class:#x}"));
    }
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    // Trusted fixture address in a bare native VAS with no guest reservations.
    // Runtime placement must instead use the shared VA-manager exclusion ledger.
    let start = Instant::now();
    let mut storage = if phased {
        let mut storage =
            Storage::prepare(&rm, space, (1 << 40) - (2 << 20)).map_err(|error| error.reason)?;
        let prepare_us = start.elapsed().as_micros();
        let mut max_clear_us = 0;
        let mut pieces = 0;
        loop {
            let t = Instant::now();
            let done = storage.clear_next(&rm)?;
            max_clear_us = max_clear_us.max(t.elapsed().as_micros());
            pieces += 1;
            if done {
                break;
            }
        }
        let t = Instant::now();
        storage.map_initialized(&rm)?;
        let map_us = t.elapsed().as_micros();
        println!(
            "USER_STORAGE_PHASES prepare_us={prepare_us} clear_steps={pieces} max_clear_us={max_clear_us} map_us={map_us}"
        );
        // Diagnostic rejection, never a timeout granting resource reuse.
        if prepare_us > 12_000 || max_clear_us > 12_000 || map_us > 12_000 {
            storage.release(&rm)?;
            rm.free_vaspace(space);
            return Err("USER_STORAGE_PHASE_EXCEEDS_INVALIDATE_BOUND".into());
        }
        storage
    } else {
        Storage::allocate(&rm, space, (1 << 40) - (2 << 20)).map_err(|error| error.reason)?
    };
    let output = Buffer::new(&rm, space, Some(MapPerm::READ_WRITE))?;
    let output_va = output.va.ok_or("output VA absent")?;
    output.store(0, 0)?;
    for off in (0..4096).step_by(4) {
        output.store(0x1000 + off, INITIAL ^ off as u32)?;
    }
    let spec = storage.spec(0)?;
    let chan = if subdevice {
        rm.birth_user_ring_channel(space, ENGINE_TYPE_COPY0, spec, 1)
    } else {
        rm.birth_channel(space, ENGINE_TYPE_COPY0, spec)
    }
    .map_err(|e| format!("birth: {e:?}"))?;
    let result = (|| {
        chan.assert_user().map_err(str::to_string)?;
        rm.alloc_ce_object(chan, ENGINE_TYPE_COPY0)
            .map_err(|e| format!("CE: {e:?}"))?;
        let mut preceding = reference_commands(output_va, rm.ce_class_id())?;
        preceding.truncate(preceding.len() - 4); // No WFI or engine-idle tail in GP0.
        if large_source {
            preceding = padded_storage_prefix(preceding)?;
        }
        if subdevice {
            // The native allocation explicitly authored subDeviceId1 through
            // the public layout. Prove initial ACTIVE and every excluded bit.
            let header = method_header_inc(0, fifo::SET_REFERENCE_C56F, 1).ok_or("reference")?;
            let mut masked = vec![header, 2, 0x10000, header, 99];
            for bit in 1..12 {
                masked.extend([0x10000 | (1 << (bit + 4)), header, 99]);
            }
            masked.extend([0x1fff0, 0x20000, 0x30000, header, 99, 0x20010, 0x30000]);
            masked.extend(preceding);
            masked.push(0x10000); // Final HOST witness must fetch even while inactive.
            preceding = masked;
        }
        let witness_offset = if large_source { 128 << 10 } else { 0x1000 };
        let witness = [
            if subdevice { 0x10010 } else { 0 },
            method_header_inc(0, fifo::SET_REFERENCE_C56F, 1).ok_or("reference")?,
            1,
        ];
        let mut consumption = None;
        let published_put = if assembled {
            use kf_chan::{user_batch, user_consumption, user_stream};
            let snapshot = user_stream::Commands::copy_from(&preceding, user_batch::MAX_OPS)
                .map_err(|e| format!("snapshot: {e:?}"))?;
            let mut state = user_stream::State::default();
            let class = rm.ce_class_id();
            let mut ops = user_stream::feed(&snapshot, &mut state, |value| {
                (value == class).then_some(user_stream::Binding::Hardware { class })
            })
            .map_err(|e| format!("parser: {e:?}"))?;
            state.finish().map_err(|e| format!("packet: {e:?}"))?;
            ops.push(user_stream::Op::GuestReference(1));
            let actions = user_batch::plan(&ops).map_err(|e| format!("plan: {e:?}"))?;
            if actions.len() != 1 {
                return Err("native fixture needs exactly one command cell".into());
            }
            let Some(user_batch::Action::Commands(words)) = actions.first() else {
                return Err("native fixture contains a software action".into());
            };
            let mut ledger = user_consumption::Consumption::new(kf_chan::user_storage::ENTRIES, 1)
                .map_err(|e| format!("ledger: {e:?}"))?;
            let publication = ledger
                .publish(user_consumption::ConsumedSource {
                    generation: 1,
                    guest_get: 1,
                })
                .map_err(|e| format!("publication: {e:?}"))?;
            storage.write_publication(publication, words, false)?;
            println!(
                "USER_STORAGE_ASSEMBLED methods={} words={} immutable=true native_reference=true",
                ops.len(),
                words.len()
            );
            consumption = Some(ledger);
            publication.put
        } else {
            storage.write_authored(0, &preceding)?;
            storage.write_authored(witness_offset, &witness)?;
            // Exercise the exact runtime raw USER descriptor writer as well as
            // native fetching of the larger, still bounded public direct span.
            let va = storage.spec(0)?.gp_fifo_va - kf_chan::user_storage::GP_OFFSET;
            let entry = gp_entry(va, 4 * preceding.len() as u64).ok_or("source GP")?;
            storage.write_user_gp(0, entry)?;
            if control_barrier {
                if barrier_level {
                    // Same public LENGTH0/SYNC_WAIT/NOP, only LEVEL differs.
                    storage.write_user_gp(1, (1 << 63) | (1 << 41))?;
                } else {
                    storage.write_host_barrier(1)?;
                }
                storage.write_host_witness(2)?;
                // Final single-engine bench observation; not a runtime receipt.
                storage.write_gp(
                    3,
                    witness_offset + if subdevice { 0 } else { 4 },
                    if subdevice { 12 } else { 8 },
                    false,
                )?;
                4
            } else {
                storage.write_gp(1, 0x1000, 4 * witness.len() as u64, sync_wait)?;
                2
            }
        };
        println!(
            "USER_STORAGE_SETUP source_bytes={} subdevice={subdevice} barrier_level={barrier_level} sync_wait={sync_wait} control_barrier={control_barrier} class={class:#x} driver={} entries={} ring_gpu_read_only=true page_size=4096 userd_gpu_mapped=false",
            preceding.len() * 4,
            rm.driver_version(),
            kf_chan::user_storage::ENTRIES
        );
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        storage.publish_put(published_put)?;
        rm.doorbell(chan.token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
        let start = Instant::now();
        let mut samples = 0;
        let mut max_get = 0;
        let mut early_reference = false;
        let mut initial_marker_seen = false;
        // Explicit stimulus only: keep the preceding HOST acquire unsatisfied.
        while start.elapsed() < Duration::from_millis(50) {
            max_get = max_get.max(storage.gpu_get()?);
            let reference = storage.gpu_reference()?;
            initial_marker_seen |= reference == 2;
            early_reference |= if subdevice {
                reference != 0 && reference != 2
            } else {
                reference != 0
            };
            samples += 1;
            std::thread::sleep(Duration::from_millis(1));
        }
        println!(
            "USER_STORAGE_HELD samples={samples} max_get={max_get} early_reference={early_reference} initial_marker_seen={initial_marker_seen} acquire={} ce_semaphore={}",
            output.load(0)?,
            output.load(4)?
        );
        output.store(0, 1)?;
        kf_linux_raw::release_fence();
        let finished = loop {
            if storage.gpu_reference()? == 1 && (!assembled || storage.gpu_get()? == published_put)
            {
                break true;
            }
            if start.elapsed() > Duration::from_secs(3) {
                break false;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let mut copy_matches = true;
        for off in (0..4096).step_by(4) {
            copy_matches &= output.load(0x2000 + off)? == (INITIAL ^ off as u32);
        }
        let get = storage.gpu_get()?;
        let reference = storage.gpu_reference()?;
        println!(
            "USER_STORAGE_OBSERVATION finished={finished} reference={reference} copy_matches={copy_matches} ce_semaphore={} GP_GET={get} GP_PUT={published_put} elapsed_us={}",
            output.load(4)?,
            start.elapsed().as_micros()
        );
        if let Some(mut ledger) = consumption {
            let receipts = ledger
                .observe_get(1, get)
                .map_err(|e| format!("receipt: {e:?}"))?;
            if receipts
                != [kf_chan::user_consumption::ConsumedSource {
                    generation: 1,
                    guest_get: 1,
                }]
            {
                return Err("assembled publication has no exact source receipt".into());
            }
            println!("USER_STORAGE_ASSEMBLED_RECEIPT generation=1 guest_get=1 native_get={get}");
        }
        // Earlier two-entry discriminator requires early control commitment.
        // The new control-NOP WAIT receipt instead forbids subsequent GP2
        // commitment while held, independent of GP1 commitment timing.
        if !finished
            || reference != 1
            || !copy_matches
            || output.load(4)? != 1
            || get != published_put
            || samples == 0
            || early_reference
            || (subdevice && !initial_marker_seen)
            || if control_barrier {
                max_get == 0 || max_get >= 3
            } else {
                max_get != if sync_wait { 1 } else { 2 }
            }
        {
            return Err("storage/HOST-witness criterion failed".into());
        }
        Ok(())
    })();
    // Restore the deliberate bench stimulus even if a sample failed. This does
    // not authorize cleanup: native channel free below must still succeed.
    let fixture_release = output.store(0, 1);
    kf_linux_raw::release_fence();
    // A collection deadline never grants release: native channel free must succeed.
    rm.free_channel(chan)
        .map_err(|e| format!("channel free: {e:?}"))?;
    let cleanup = storage.release(&rm);
    println!(
        "USER_STORAGE_CLEANUP result={cleanup:?} released={}",
        storage.released()
    );
    cleanup?;
    assert!(storage.spec(0).is_err());
    assert!(storage.publish_put(0).is_err());
    output.close(&rm, space)?;
    rm.free_vaspace(space);
    result.and(fixture_release)
}

/// Public595.84 class/clc997.h modern semaphore methods. Probe-only C997,
/// not a family/die table. All addresses name the probe's own output fixture.
const PROBE_ADA_A: u32 = 0xc997; // Public595.84 class/clc997.h ADA_A interface ID.

fn engine_hold_commands(output_va: u64) -> Result<Vec<u32>, String> {
    let mut out = vec![
        method_header_inc(0, kf_abi::submit::SET_OBJECT, 1).ok_or("binding")?,
        PROBE_ADA_A,
    ];
    for (address, operation) in [(output_va, 1), (output_va + 4, 0)] {
        // PAYLOAD_LOWER/UPPER, ADDRESS_LOWER/UPPER, REPORT_SEMAPHORE_EXECUTE.
        // STRUCTURE_SIZE=ONE_WORD (14:13); PIPELINE_LOCATION=NONE (5:2).
        out.extend([
            method_header_inc(0, 0x158, 5).ok_or("GR semaphore")?,
            1,
            0,
            address as u32,
            (address >> 32) as u32,
            (1 << 13) | operation,
        ]);
    }
    // HOST-only marker without WFI. Engine remains blocked at the first acquire.
    out.extend([
        method_header_inc(0, fifo::SEM_ADDR_LO, 5).ok_or("HOST marker")?,
        (output_va + 8) as u32,
        ((output_va + 8) >> 32) as u32,
        1,
        0,
        fifo::SEM_EXECUTE_RELEASE_32BIT,
    ]);
    Ok(out)
}

fn run_engine_witness(sync_wait: bool) -> Result<(), String> {
    use kf_abi::submit::ENGINE_TYPE_GRAPHICS;
    use kf_chan::user_storage::Storage;
    let uid = kf_linux_raw::effective_uid();
    println!("USER_ENGINE_WITNESS_IDENTITY uid={uid}");
    if uid != 65534 {
        return Err("probe requires unprivileged uid65534".into());
    }
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let (architecture, implementation, _) = rm.arch_info();
    let chosen =
        kf_chip::choose_host_classes(architecture, implementation, rm.supported_class_ids())?;
    if chosen.gpfifo_channel().channel_id().0 != kf_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A
        || !rm.supported_class_ids().contains(&PROBE_ADA_A)
    {
        return Err("ENGINE_WITNESS_CLASSES_UNMEASURED: needs publicC56F/C997".into());
    }
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let mut storage =
        Storage::allocate(&rm, space, (1 << 40) - (2 << 20)).map_err(|error| error.reason)?;
    let output = Buffer::new(&rm, space, Some(MapPerm::READ_WRITE))?;
    let output_va = output.va.ok_or("output VA absent")?;
    let spec = storage.spec(0)?;
    let witness_va = spec.gp_fifo_va - kf_chan::user_storage::GP_OFFSET + 0x1000;
    let chan = rm
        .birth_channel(space, ENGINE_TYPE_GRAPHICS, spec)
        .map_err(|e| format!("birth: {e:?}"))?;
    let result = (|| {
        chan.assert_user().map_err(str::to_string)?;
        rm.alloc_engine_object(chan, PROBE_ADA_A, None)
            .map_err(|e| format!("GR object: {e:?}"))?;
        let initial_cursor = storage.gpu_top_get()?;
        println!("USER_ENGINE_WITNESS_INITIAL cursor={initial_cursor:?}");
        if initial_cursor != Some(0) {
            return Err("native fixture USERD cursor not initially zero".into());
        }
        let preceding = engine_hold_commands(output_va)?;
        let witness = [
            0,
            method_header_inc(0, fifo::SET_REFERENCE_C56F, 1).ok_or("reference")?,
            1,
        ];
        storage.write_authored(0, &preceding)?;
        storage.write_authored(0x1000, &witness)?;
        storage.write_gp(0, 0, 4 * preceding.len() as u64, false)?;
        storage.write_gp(1, 0x1000, 4 * witness.len() as u64, sync_wait)?;
        println!(
            "USER_ENGINE_WITNESS_SETUP sync_wait={sync_wait} driver={} witness_va={witness_va:#x} witness_bytes=12 ring_gpu_read_only=true userd_gpu_mapped=false",
            rm.driver_version()
        );
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        storage.publish_put(2)?;
        rm.doorbell(chan.token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
        let start = Instant::now();
        let mut samples = 0;
        let mut seen_cursor = None;
        let mut seen_get = 0;
        let mut early_engine = false;
        let mut early_reference = false;
        while start.elapsed() < Duration::from_millis(50) {
            let cursor = storage.gpu_top_get()?;
            let get = storage.gpu_get()?;
            if cursor.is_some_and(|v| v > witness_va && v <= witness_va + 12)
                && output.load(8)? == 1
            {
                seen_cursor = cursor;
                seen_get = get;
            }
            early_engine |= output.load(4)? != 0;
            early_reference |= storage.gpu_reference()? != 0;
            samples += 1;
            std::thread::sleep(Duration::from_millis(1));
        }
        println!(
            "USER_ENGINE_WITNESS_HELD samples={samples} cursor={seen_cursor:?} GP_GET={seen_get} early_engine={early_engine} early_reference={early_reference} acquire={} engine_release={} host_marker={}",
            output.load(0)?,
            output.load(4)?,
            output.load(8)?
        );
        if let Some(cursor) = seen_cursor {
            println!(
                "USER_ENGINE_WITNESS_CURSOR raw_low={:#x} raw_high={:#x}",
                cursor as u32,
                cursor >> 32
            );
        }
        output.store(0, 1)?;
        kf_linux_raw::release_fence();
        let finished = loop {
            if storage.gpu_reference()? == 1 {
                break true;
            }
            if start.elapsed() > Duration::from_secs(3) {
                break false;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let get = storage.gpu_get()?;
        let reference = storage.gpu_reference()?;
        let engine_release = output.load(4)?;
        let host_marker = output.load(8)?;
        println!(
            "USER_ENGINE_WITNESS_OBSERVATION finished={finished} reference={reference} engine_release={engine_release} host_marker={host_marker} GP_GET={get} elapsed_us={}",
            start.elapsed().as_micros()
        );
        if !finished
            || reference != 1
            || engine_release != 1
            || host_marker != 1
            || get != 2
            || samples == 0
            || seen_cursor.is_none()
            || seen_get != 2
            || early_engine
            || early_reference
        {
            return Err("engine/HOST-consumption witness criterion failed".into());
        }
        Ok(())
    })();
    let fixture_release = output.store(0, 1);
    kf_linux_raw::release_fence();
    rm.free_channel(chan)
        .map_err(|e| format!("channel free: {e:?}"))?;
    let cleanup = storage.release(&rm);
    println!(
        "USER_ENGINE_WITNESS_CLEANUP result={cleanup:?} released={}",
        storage.released()
    );
    cleanup?;
    output.close(&rm, space)?;
    rm.free_vaspace(space);
    result.and(fixture_release)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    ReadWrite,
    ReadOnly,
    WriteAttempt,
}

impl Mode {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "read-write" => Ok(Self::ReadWrite),
            "read-only" => Ok(Self::ReadOnly),
            "write-attempt" => Ok(Self::WriteAttempt),
            _ => Err("usage: kf-user-ring-probe read-write|read-only|write-attempt|reference|reference-held|reference-split|reference-split-held".into()),
        }
    }

    fn perm(self) -> MapPerm {
        MapPerm {
            read_only: self != Self::ReadWrite,
            ..MapPerm::READ_WRITE
        }
    }
}

struct Buffer {
    memory: u32,
    // Kept open for the CPU view's lifetime.
    _node: kf_linux_raw::CharDevice,
    cookie: u64,
    cpu: VolatileRegion,
    va: Option<u64>,
}

impl Buffer {
    fn new(rm: &HostRm, space: VaSpace, perm: Option<MapPerm>) -> Result<Self, String> {
        let memory = rm
            .alloc_device_local(BYTES)
            .map_err(|e| format!("memory: {e:?}"))?;
        // Early setup errors leave objects to the native RM client's teardown;
        // no channel has been scheduled yet.
        let va = perm
            .map(|p| rm.map_window_perm(space, memory, BYTES, false, p))
            .transpose()
            .map_err(|e| format!("map: {e:?}"))?;
        if va.is_some_and(|v| v.checked_add(BYTES).is_none_or(|e| e > 1 << 40)) {
            return Err("probe mapping exceeds public40-bit GP/semaphore address width".into());
        }
        let (node, cookie) = rm
            .arm_cpu_view(MapNode::Gpu, memory, 0, BYTES, ViewAccess::ReadWrite)
            .map_err(|e| format!("view: {e:?}"))?;
        let cpu = VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            BYTES,
            CachePolicy::Uncached,
            HostPageSize::query(),
        )
        .map_err(|e| format!("mmap: {e:?}"))?;
        let b = Self {
            memory,
            _node: node,
            cookie,
            cpu,
            va,
        };
        for off in (0..BYTES).step_by(4) {
            b.store(off, 0)?;
        }
        Ok(b)
    }

    fn store(&self, off: u64, value: u32) -> Result<(), String> {
        self.cpu
            .store_u32(HostOffset::new(off), value)
            .map_err(|e| format!("store: {e:?}"))
    }

    fn load(&self, off: u64) -> Result<u32, String> {
        self.cpu
            .load_u32(HostOffset::new(off))
            .map_err(|e| format!("load: {e:?}"))
    }

    // Called only AFTER successful channel free. Try every cleanup operation.
    fn close(self, rm: &HostRm, space: VaSpace) -> Result<(), String> {
        drop(self.cpu);
        let view = rm.release_cpu_view(kf_host::CpuViewRelease {
            h_memory: self.memory,
            p_linear_address: self.cookie,
        });
        let unmap = self.va.map(|va| rm.unmap(space, va, false)).transpose();
        let memory = rm.free(self.memory);
        println!("USER_RING_BUFFER_CLEANUP view={view:?} unmap={unmap:?} memory={memory:?}");
        view.map_err(|e| format!("release view: {e:?}"))
            .and(unmap.map(|_| ()).map_err(|e| format!("unmap: {e:?}")))
            .and(memory.map_err(|e| format!("free memory: {e:?}")))
    }
}

fn release(words: &mut Vec<u32>, va: u64, value: u32) -> Result<(), String> {
    words.push(method_header_inc(0, fifo::SEM_ADDR_LO, 4).ok_or("semaphore header")?);
    words.extend_from_slice(&[va as u32, (va >> 32) as u32, value, 0]);
    words.push(method_header_inc(0, fifo::SEM_EXECUTE, 1).ok_or("execute header")?);
    words.push(fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN);
    Ok(())
}

fn commands(mode: Mode, ring_va: u64, output_va: u64) -> Result<Vec<u32>, String> {
    let mut words = Vec::new();
    if mode == Mode::WriteAttempt {
        release(&mut words, ring_va + SENTINEL, ATTEMPT)?;
    }
    release(&mut words, output_va, 1)?;
    Ok(words)
}

fn run(mode: Mode) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let ring = Buffer::new(&rm, space, Some(mode.perm()))?;
    let output = Buffer::new(&rm, space, Some(MapPerm::READ_WRITE))?;
    let userd = Buffer::new(&rm, space, None)?;
    let ring_va = ring.va.ok_or("ring VA absent")?;
    let output_va = output.va.ok_or("output VA absent")?;
    ring.store(SENTINEL, INITIAL)?;
    let chan = rm
        .birth_channel(
            space,
            kf_abi::submit::ENGINE_TYPE_GRAPHICS,
            kf_host::RingSpec {
                gp_fifo_va: ring_va + GP,
                gp_fifo_entries: 32,
                userd_memory: userd.memory,
                userd_offset: 0,
                err_notifier: 0,
            },
        )
        .map_err(|e| format!("birth: {e:?}"))?;
    let result = (|| {
        chan.assert_user().map_err(str::to_string)?;
        println!(
            "USER_RING_SETUP driver={} mode={mode:?} channel={chan:x?} ring_va={ring_va:#x} output_va={output_va:#x} perm={:?} userd_gpu_mapped=false",
            rm.driver_version(),
            mode.perm()
        );
        let words = commands(mode, ring_va, output_va)?;
        for (i, &word) in words.iter().enumerate() {
            ring.store(PB + 4 * i as u64, word)?;
        }
        let gp = gp_entry(ring_va + PB, 4 * words.len() as u64).ok_or("GP encoding")?;
        ring.store(GP, gp as u32)?;
        ring.store(GP + 4, (gp >> 32) as u32)?;
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        kf_linux_raw::release_fence();
        userd.store(USERD_GP_PUT, 1)?;
        kf_linux_raw::release_fence();
        rm.doorbell(chan.token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
        let start = Instant::now();
        let finished = loop {
            if output.load(0)? == 1 {
                break true;
            }
            if start.elapsed() > Duration::from_secs(3) {
                break false;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let sentinel = ring.load(SENTINEL)?;
        println!(
            "USER_RING_OBSERVATION finished={finished} fence={} sentinel={sentinel:#x} initial={INITIAL:#x} attempted={ATTEMPT:#x} GP_GET={} GP_PUT={} elapsed_us={}",
            output.load(0)?,
            userd.load(USERD_GP_GET)?,
            userd.load(USERD_GP_PUT)?,
            start.elapsed().as_micros()
        );
        // A negative case has no internal PASS: its journal + unchanged bytes
        // must be assessed externally. A deadline is collection, not completion.
        if mode == Mode::WriteAttempt {
            return Err(
                "write-attempt collected; journal attribution required, no application verdict"
                    .into(),
            );
        }
        if !finished || sentinel != INITIAL {
            return Err("GPU tail missing or private bytes changed".into());
        }
        Ok(())
    })();
    // Never revoke memory after a failed channel free. Native RM client teardown
    // owns that case, instead of treating the bench deadline as quiescence.
    rm.free_channel(chan)
        .map_err(|e| format!("channel free: {e:?}"))?;
    let mut cleanup = Ok(());
    for b in [ring, output, userd] {
        if let Err(e) = b.close(&rm, space) {
            cleanup = Err(e);
        }
    }
    rm.free_vaspace(space);
    println!("USER_RING_CLEANUP result={cleanup:?}");
    result.and(cleanup)
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = if args.len() == 1 {
        match args[0].as_str() {
            "engine-witness-control" => run_engine_witness(false),
            "engine-witness-sync" => run_engine_witness(true),
            "storage-assembled-phased" => run_storage(true, true, true, true, false, false, false),
            "storage-subdevice-phased" => run_storage(true, true, true, false, false, false, true),
            "storage-control-barrier-large-phased" => {
                run_storage(true, true, true, false, false, true, false)
            }
            "storage-control-barrier-phased" => {
                run_storage(true, true, true, false, false, false, false)
            }
            "storage-control-barrier-level-phased" => {
                run_storage(true, true, true, false, true, false, false)
            }
            "storage-control-barrier" => run_storage(true, true, false, false, false, false, false),
            "storage-control" => run_storage(false, false, false, false, false, false, false),
            "storage-sync-witness" => run_storage(true, false, false, false, false, false, false),
            "reference" => run_reference(false, false, false),
            "reference-subroutine" => run_reference(false, false, true),
            "reference-held" => run_reference(true, false, false),
            "reference-subroutine-held" => run_reference(true, false, true),
            "reference-split" => run_reference(false, true, false),
            "reference-split-held" => run_reference(true, true, false),
            "reference-subroutine-split-held" => run_reference(true, true, true),
            s => Mode::parse(s).and_then(run),
        }
    } else {
        Err(
            "usage: kf-user-ring-probe read-write|read-only|write-attempt|reference|reference-held|reference-split|reference-split-held"
                .into(),
        )
    };
    match result {
        Ok(()) => println!("USER_RING_PROBE=PASS"),
        Err(e) => {
            eprintln!("USER_RING_PROBE=FAIL {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_storage_prefix_keeps_commands_and_adds_only_public_nops() {
        let original = reference_commands(0x400000, 0xc6b5).unwrap();
        let padded = padded_storage_prefix(original.clone()).unwrap();
        assert_eq!(padded.len() * 4, 100 << 10);
        assert_eq!(&padded[..original.len()], original.as_slice());
        assert!(padded[original.len()..].iter().all(|v| *v == 0));
        assert!((padded.len() * 4) as u64 <= kf_chan::user_storage::MAX_SOURCE_BYTES);
    }
    #[test]
    fn reference_level_probe_changes_only_first_gp_level() {
        let main = reference_descriptor(0x200004000, 128, false).unwrap();
        let sub = reference_descriptor(0x200004000, 128, true).unwrap();
        assert_eq!(main ^ sub, 1 << 41);
        let a = kf_abi::submit::gp_entry_decode(main).unwrap();
        let b = kf_abi::submit::gp_entry_decode(sub).unwrap();
        assert!(!a.subroutine && b.subroutine);
        assert_eq!(a.gpu_va, b.gpu_va);
        assert_eq!(a.len_bytes, b.len_bytes);
        assert_eq!(a.sync_wait, b.sync_wait);
    }

    #[test]
    fn positive_ab_changes_only_gpu_mapping_permission() {
        assert_eq!(
            commands(Mode::ReadWrite, 0x200000000, 0x200010000),
            commands(Mode::ReadOnly, 0x200000000, 0x200010000)
        );
        assert!(!Mode::ReadWrite.perm().read_only);
        assert!(Mode::ReadOnly.perm().read_only);
        assert!(Mode::WriteAttempt.perm().read_only);
    }

    #[test]
    fn attempted_store_is_inside_the_private_mapping_before_real_tail() {
        let words = commands(Mode::WriteAttempt, 0x200000000, 0x200010000).unwrap();
        assert_eq!(words.len(), 14);
        assert_eq!(words[1], SENTINEL as u32);
        assert_eq!(words[3], ATTEMPT);
        assert_eq!(words[8], 0x10000);
        assert_eq!(words[10], 1);
        assert!(PB + 4 * words.len() as u64 <= SENTINEL);
        assert!(SENTINEL + 4 <= BYTES);
    }

    #[test]
    fn malformed_mode_cannot_launch_hardware() {
        assert!(Mode::parse("anything").is_err());
    }

    #[test]
    fn reference_tail_follows_virtual_copy_and_all_scope_wfi() {
        let words = reference_commands(0x200010000, 0xc6b5).unwrap();
        assert_eq!(
            &words[words.len() - 4..],
            &[
                method_header_inc(0, fifo::WFI_C56F, 1).unwrap(),
                fifo::WFI_SCOPE_ALL_C56F,
                method_header_inc(0, fifo::SET_REFERENCE_C56F, 1).unwrap(),
                1,
            ]
        );
        assert_eq!(words[5], fifo::SEM_EXECUTE_ACQUIRE_32BIT);
        assert!(PB + 4 * words.len() as u64 <= SENTINEL);
    }
}
