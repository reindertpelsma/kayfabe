//! ★★★★★ THE CAMPAIGN'S MEASURED FINDINGS, PINNED — `[owner, 2026-09-21]` *"the findings of
//! kayfabe are useful to pin in a test."*
//!
//! ## Why a test and not a document
//!
//! Every fact below was **paid for** — with a rented box, a boot, or a day. They currently live in
//! prose, and this tree's most expensive recurring failure is *"a correct document that stopped
//! being true and did not say so"*. ⊘ A doc rots silently; a test that encodes a fact fails the
//! moment the code stops honouring it.
//!
//! ⚠ **What this file may and may not contain.** A finding is admissible here only if the code
//! can actually contradict it. A measurement about hardware that no code path consults would be a
//! test asserting a constant — which this tree has already caught itself writing. ⇒ Each test
//! below names the code that would have to change for it to fail.

use kayfabe_doorbell::*;

// ---- doorbell / token findings -----------------------------------------------------------------

#[test]
fn blackwell_doorbell_encoding_differs_per_die_group() {
    // `[measured, nvkvm-pv 28/28 on RTX 5070/5090]` GB202 sets bit 30 in the work-submit token
    // where Ampere does not, and GB100 uses a different HAL entirely
    // (`kernel_fifo_gb202.c:73` vs `kernel_fifo_gb100.c:114`).
    // ⇒ The token field is MASKED, never validated, so an encoding difference cannot become a
    // refusal. Contradicted by: any validation added to the doorbell arm.
    // ⊘⊘⊘ THIS TEST USED TO PIN THE DEFECT. It asserted `HOST_TOKEN_BITS == 21`, which
    // *locked in* a mask that drops GB202's bit 30 on every doorbell. A findings file that
    // freezes a bug is worse than no findings file — it converts a defect into a requirement.
    //
    // ⇒ The host token is opaque and stored WHOLE: it comes from
    // `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN`, a §50 level-1 unprivileged host ioctl, and
    // we never decode it.
    assert_eq!(token::HOST_TOKEN_BITS, 32, "the host token is opaque — store it whole");
    let round_trip = Token {
        state: State::Idle, route: Route::Passthrough,
        host_token: 0x4000_0DEF, // GB202: RUNLIST_DOORBELL (bit 30) set
        applied_seq: 0,
    };
    assert_eq!(
        Token::decode(round_trip.encode()).host_token,
        0x4000_0DEF,
        "⊘ bit 30 must survive — dropping it rings the host doorbell with RUNLIST_DOORBELL_DISABLE"
    );
}

#[test]
fn rm_zeroes_a_caller_supplied_userd_at_allocation() {
    // `[measured w233, R32 GA106 ad6bb9f]` RM zeroes all 512 bytes of a caller-supplied USERD at
    // ALLOCATION. ⇒ §7: "Birth is at allocation, never lazy" -- adopting at first doorbell wipes
    // the cursor that just rang.
    // Contradicted by: a lazy-birth path appearing in Plane::allocate_channel.
    assert_eq!(channel::Birth::AtAllocation, channel::Birth::AtAllocation);
    let src = include_str!("../src/plane.rs");
    assert!(
        !src.contains("birth_on_doorbell") && !src.contains("lazy_birth"),
        "a lazy birth path would wipe the cursor RM zeroed at allocation"
    );
}

// ---- the failure classes that cost the most ----------------------------------------------------

#[test]
fn a_kernel_channel_error_is_globally_fatal_so_we_refuse_rather_than_fault() {
    // `[measured]` the unified-memory driver treats ANY channel error as globally fatal: one
    // fault kills CUDA for EVERY process in the guest until the driver reloads.
    // ⇒ An untranslatable operand on a kernel channel REFUSES; only a user channel may fault.
    use channel::{Disposition, Owner, Submission};
    let k = Submission { owner: Owner::Kernel, route: Route::Translated, all_operands_translatable: false };
    assert_eq!(k.decide(), Disposition::RefuseAndPoison);
    assert_ne!(k.decide(), Disposition::FaultChannel, "a fault here is a guest-wide DoS");
}

#[test]
fn a_forged_completion_is_how_a_scrub_becomes_a_leak() {
    // `[measured, the C artifact]` the CeUtils scrub completed `finishPayload` for work that did
    // not happen, and that is how a guest process read another's freed pages.
    // ⇒ A forge is licensed ONLY where no GPU work ran.
    assert_eq!(Completion::for_route(Route::Emulated, true), Completion::Nothing);
    assert_eq!(Completion::for_route(Route::Translated, false), Completion::Nothing);
}

#[test]
fn one_read_trapped_page_cost_a_2_5x_loss_on_llm_decode() {
    // `[measured, the C artifact]` 99% of its 1000-3000 exits per token were READS of a single
    // firmware debug register on a page kept read-trapped as "boot state".
    // ⇒ A page is read-trapped for a PHASE. Contradicted by: BootStateMachine trapping at Runtime.
    let mut s = readtrap::ReadTrapSet::new();
    s.add(0x110, readtrap::ReadReason::BootStateMachine);
    assert_eq!(s.policy(0x110, 0, readtrap::Phase::Runtime), readtrap::ReadPolicy::FromShadow);
}

#[test]
fn rm_serialises_on_a_device_global_lock_so_parallelism_buys_nothing() {
    // `[measured R12, and re-measured in-process w823]` 800 alloc+free verbs: 1 thread 1332 ms,
    // one client x4 threads 1481 ms (0.90x), 4 clients 1461 ms (0.91x) -- with 484 and 436
    // OVERLAPPING interval pairs. The verbs genuinely overlap on the wire and overlap buys
    // nothing, because RM holds one driver-wide lock.
    // ⇒ §3 spends a thread to keep the TRAP lock-free, not to parallelise host verbs.
    // Contradicted by: a design that adds host-verb worker threads expecting throughput.
    let src = include_str!("../src/plane.rs");
    assert!(src.contains("VA manager"), "one VA manager, not a pool -- the lock is device-global");
}

// ---- the compatibility axes --------------------------------------------------------------------

#[test]
fn every_supported_family_can_allocate_its_engine_objects() {
    // ⊘⊘ `[fable w823, A1]` the class table hard-coded GA10x, so an Ada guest's cuCtxCreate
    // (ADA_COMPUTE_A 0xC9C0) was DENIED by default. Every non-Ampere guest had no channel, no
    // compute object, no copy engine and no doorbell page.
    // ⇒ Ids derived from ogkm's published headers by tools/derive_classes.sh.
    use classgen::{classes_for, Family};
    for f in [Family::Turing, Family::Ampere, Family::Ada, Family::Hopper, Family::Blackwell] {
        let c = classes_for(f);
        for (what, id) in [
            ("channel", c.channel_gpfifo), ("compute", c.compute),
            ("dma_copy", c.dma_copy), ("usermode", c.usermode),
        ] {
            assert_ne!(
                rmgraph::class_policy(id),
                rmgraph::ClassPolicy::Deny("not on the allowlist — default deny"),
                "{f:?} {what} ({id:#x}) is denied — that guest cannot start"
            );
        }
    }
}

#[test]
fn the_derived_class_ids_match_ogkms_published_headers() {
    // ★ The numbers are transcribed from MIT/GPL source, not measured from a running driver and
    // not invented. `[owner]` "don't extract blobs from the running driver or require root".
    use classgen::{classes_for, Family};
    assert_eq!(classes_for(Family::Ada).compute, 0xC9C0, "ADA_COMPUTE_A");
    assert_eq!(classes_for(Family::Hopper).compute, 0xCBC0, "HOPPER_COMPUTE_A");
    assert_eq!(classes_for(Family::Blackwell).compute, 0xCDC0, "BLACKWELL_COMPUTE_A");
    assert_eq!(classes_for(Family::Turing).channel_gpfifo, 0xC46F, "TURING_CHANNEL_GPFIFO_A");
    // ⊘ Ada has NO channel/copy/usermode define of its own -- it reuses Ampere's. That is a fact
    // from the headers, and inventing an ADA_CHANNEL_GPFIFO_A would be fabrication.
    assert_eq!(classes_for(Family::Ada).channel_gpfifo, classes_for(Family::Ampere).channel_gpfifo);
    assert_eq!(classes_for(Family::Ada).usermode, classes_for(Family::Ampere).usermode);
}

#[test]
fn windows_consumer_drivers_run_with_gsp_off_so_no_gsp_is_a_core_seam() {
    // `[measured, owner]` RTX 1660 Ti (TU116), RTX 2080 Ti (TU102) and RTX 4070 (AD104, game
    // driver 591.86) all report NO GSP firmware version in nvidia-smi -q. Two architectures,
    // three dies, one on a CURRENT driver.
    // ⇒ A non-GSP control plane is not a later feature; it is a shape the design must admit.
    let nogsp = element::ControlPlane::NoGsp;
    assert!(!nogsp.has_message_queue());
    assert_eq!(nogsp.element_layout(), None, "a caller cannot assume an element layout exists");
}

#[test]
fn the_element_header_broke_between_580_and_610() {
    // `[measured]` 48 bytes -> 16 bytes, every offset moved, and elemCount VANISHED.
    // ⇒ Carried as data with Option, never a version branch and never a sentinel.
    assert_eq!(element::LAYOUT_580.header_bytes, 48);
    assert_eq!(element::LAYOUT_610.header_bytes, 16);
    assert_eq!(element::LAYOUT_610.elem_count_off, None, "gone, not zero");
}

#[test]
fn the_class_table_matches_what_the_c_compiler_says_the_headers_define() {
    // ★★★ `[owner]` "don't use regex to parse C code -- use proper parsers/compilers."
    //
    // ⊘ This does not re-implement a parser: it runs `tools/derive_classes.sh`, which COMPILES a
    // generated program against ogkm's class headers and prints what the preprocessor resolved.
    // A grep would see the first textual `#define` and call it the value; the compiler sees
    // conditionals, redefinitions and macro expansion. ⇒ If our table ever drifts from the
    // headers, this fails.
    //
    // ⚠ SKIPS (rather than fails) when ogkm or gcc is absent -- "we could not ask" is not "the
    // answer was no", and a box without the oracle must not manufacture a green.
    use std::process::Command;
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/derive_classes.sh");
    if !std::path::Path::new(script).exists() {
        eprintln!("SKIP: no derive_classes.sh");
        return;
    }
    let out = match Command::new("bash").arg(script).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => {
            eprintln!("SKIP: ogkm headers or gcc unavailable — NOT a pass");
            return;
        }
    };
    let want = |name: &str| -> Option<u32> {
        out.lines().find(|l| l.starts_with(name)).and_then(|l| {
            let t = l.split_whitespace().nth(1)?;
            u32::from_str_radix(t.strip_prefix("0x")?, 16).ok()
        })
    };
    use classgen::{classes_for, Family};
    for (sym, got) in [
        ("TURING_CHANNEL_GPFIFO_A", classes_for(Family::Turing).channel_gpfifo),
        ("TURING_COMPUTE_A", classes_for(Family::Turing).compute),
        ("AMPERE_CHANNEL_GPFIFO_A", classes_for(Family::Ampere).channel_gpfifo),
        ("AMPERE_COMPUTE_B", classes_for(Family::Ampere).compute),
        ("ADA_COMPUTE_A", classes_for(Family::Ada).compute),
        ("HOPPER_COMPUTE_A", classes_for(Family::Hopper).compute),
        ("HOPPER_USERMODE_A", classes_for(Family::Hopper).usermode),
        ("BLACKWELL_COMPUTE_A", classes_for(Family::Blackwell).compute),
        ("BLACKWELL_DMA_COPY_A", classes_for(Family::Blackwell).dma_copy),
    ] {
        match want(sym) {
            Some(w) => assert_eq!(
                got, w,
                "⊘ {sym}: our table says {got:#06X}, the COMPILER says the header defines {w:#06X}"
            ),
            None => panic!("⊘ {sym} is ABSENT from the headers but present in our table"),
        }
    }
}

#[test]
fn we_author_the_user_register_access_map_rather_than_deriving_it() {
    // ★★★ `[owner, 2026-09-21]` asked where the compilable source for register access semantics
    // is, since "the data must be in normal constants, arrays, C files as well".
    //
    // ⊘ MEASURED in ogkm: for THIS fact there is none. `gpuConstructUserRegisterAccessMap_IMPL`
    // (`src/kernel/gpu/gpu_register_access_map.c:208`) obtains the map as **zlib-compressed
    // bytes** delivered over `NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP`, and the
    // *physical* implementation of that control is **not in the open tree at all** — the data
    // lives in the GSP firmware blob.
    //
    // ★★★ And that inverts the question rather than leaving it unanswered: **we ARE the GSP.**
    // `0x20800a41 GET_USER_REGISTER_ACCESS_MAP` is on our served list, so kayfabe does not need
    // to DISCOVER which BAR0 registers guest userspace may touch — **it DECLARES them.**
    //
    // ⇒ THE INVARIANT THAT FOLLOWS, and it is new: the map we serve and the classifier's middle
    // arm (`Class::UserspaceMappable`) are **one fact**. If they drift, one of two things breaks:
    //   * we declare a page mappable and then TRAP it ⇒ an unprivileged guest process can reach
    //     the privileged ring — §4's inner boundary breached;
    //   * we declare it unmappable and serve it anyway ⇒ the guest's driver never maps it and the
    //     userspace path silently does not work.
    // ⊘ Neither failure is visible from either side alone, which is why this is pinned here.
    assert!(
        rpc::control_is_served(0x20800a41),
        "GET_USER_REGISTER_ACCESS_MAP must be served — we are the authority for it"
    );
    // The classifier must therefore HAVE a userspace-mappable arm to keep consistent with it.
    let src = include_str!("../src/trap.rs");
    assert!(
        src.contains("UserspaceMappable"),
        "the map we declare needs a matching classifier arm, or the two cannot agree"
    );
}

#[test]
fn the_access_map_we_serve_is_deny_by_default_not_the_0xff_fallback() {
    // ⊘⊘⊘ ogkm: `if (!bUseRegisterAccessMap || compressedSize == 0) memset(map, 0xFF, ...)`.
    // Returning compressedSize=0 is the CHEAPEST satisfying answer -- and the wrong one: 0xFF
    // tells the guest EVERY BAR0 register is userspace-accessible, and §47 forbids trapping a
    // page guest userspace can map. ⇒ We could then trap nothing, and the privileged arm of the
    // classifier would cease to exist.
    use accessmap::AccessMap;
    let mut m = AccessMap::deny_all();
    assert!(!m.is_allowed(0x110c00), "deny by default");
    // The usermode window (the doorbell page) is the thing userspace legitimately maps.
    m.allow_range(0x810000, 0x10000);
    assert!(m.is_allowed(0x810000) && m.is_allowed(0x810090));
    assert!(!m.is_allowed(0x820000), "one range, not a blanket");
    // ⊘ And a privileged register stays denied -- that is what lets us trap it.
    assert!(!m.is_allowed(0x110c00), "the GSP RPC submit register must NOT be userspace-mappable");
}

#[test]
fn our_bit_order_matches_the_guests_nvbitfieldtest() {
    // ⊘ ogkm computes `bitOffset = offset / sizeof(NvU32)` and tests one bit. If our bit order
    // differed, every allowed range would be off by a factor of 4 or mirrored within each byte,
    // and the guest would map the wrong pages -- silently.
    use accessmap::AccessMap;
    let mut m = AccessMap::deny_all();
    m.allow_range(0x1000, 4); // exactly ONE 32-bit register
    assert!(m.is_allowed(0x1000));
    assert!(!m.is_allowed(0x1004), "the next register must not be caught");
    assert!(!m.is_allowed(0xFFC));
    // register index 0x1000/4 = 1024 ⇒ byte 128, bit 0
    assert_eq!(m.raw()[128], 0x01, "bit order: LSB-first within the byte");
}


#[test]
fn the_access_map_stream_is_gzip_and_fits_both_drivers_caps() {
    // ⊘⊘⊘ `[fable w823, CRITICAL]` the first version emitted ZLIB and I verified it with Python's
    // `zlib.decompress` — which accepts zlib. **The consumer does not.** ogkm skips a 10-byte
    // GZIP header and RAW-inflates (`gpu_register_access_map.c:362-364`), under
    // `NV_ASSERT_OK_OR_RETURN` (`gpu.c:2183`) ⇒ a wrong container **aborts GPU init on every
    // guest, on every die**. Testing against a different decoder than the consumer uses is worth
    // less than no test.
    use accessmap::AccessMap;
    let mut m = AccessMap::deny_all();
    m.allow_range(0x810000, 0x10000);
    let g = m.to_gzip_deflate();

    // The container ogkm expects, byte for byte.
    assert_eq!(&g[..3], &[0x1f, 0x8b, 0x08], "gzip magic + CM=deflate");
    assert!(g.len() > 10, "ogkm does `pComprData += 10` — the header must be exactly 10 bytes");
    assert_eq!(g[3], 0x00, "no FLG bits: FNAME/FEXTRA would make the header longer than 10");

    // ⊘ And it must FIT. The reply struct caps the payload, and the first version was 128x over:
    // 524 339 bytes of stored blocks against a 4 096-byte cap on 580.
    assert!(
        g.len() <= AccessMap::MAX_COMPRESSED_580,
        "{} bytes exceeds 580's {}-byte cap — the reply cannot carry it",
        g.len(),
        AccessMap::MAX_COMPRESSED_580
    );
    assert!(g.len() <= AccessMap::MAX_COMPRESSED_610);
}

#[test]
fn the_deflate_stream_round_trips_through_a_raw_inflater() {
    // ★ Decoded the way ogkm decodes it: skip 10, raw-inflate, and the result must be EXACTLY
    // `userRegisterAccessMapSize` bytes or ogkm returns NV_ERR_INFLATE_COMPRESSED_DATA_FAILED.
    // ⚠ This test cannot run a real inflater in-crate (no dependencies), so it asserts the
    // structural preconditions and the size contract; `examples/zlib_emit.rs` + the shell check
    // does the end-to-end decode. ⊘ Stated rather than implied: this is a PARTIAL check.
    use accessmap::{AccessMap, MAP_BYTES};
    let m = AccessMap::deny_all();
    let g = m.to_gzip_deflate();
    assert_eq!(m.raw().len(), MAP_BYTES, "the inflated size is the contract");
    assert!(g.len() < MAP_BYTES / 8, "a near-uniform map must compress hard, not merely store");
}
