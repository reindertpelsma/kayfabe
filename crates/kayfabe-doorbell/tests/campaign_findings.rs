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
    // ⊘⊘⊘ SUPERSEDED w824 — the finding held, and its conclusion was WEAKER than the truth.
    // The old pin allowed a read trap to exist so long as it was scoped to a PHASE. The owner's
    // question ("none of the reads in bar0 had side effects ... so why is it suddenly that read
    // traps are needed in v3") forced the stronger reading, and it is the one the tree already
    // recorded: THE_CONSTRAINTS.md:28, measured w708-w710, "BAR0 write-only bar the counter
    // page", holding across raw client + cup3 + LLM with TRAP_FILLS=0.
    // ⇒ The pin is now that read-trap machinery DOES NOT EXIST. A phase-scoped read trap cannot
    // cost 2.5x if there is no read exit to scope.
    // Contradicted by: `may_trap_read` returning true anywhere, or a read-policy type reappearing.
    for bar in [0u8, 1, 2] {
        for off in [0u64, 0x110, 0x1000, trappolicy::PRAMIN_BASE, trappolicy::PRAMIN_BASE + trappolicy::PRAMIN_LEN, 0xFF_F000] {
            assert!(!trappolicy::may_trap_read(vmm::Bar(bar), off, classgen::Family::Ampere), "read trap at bar{bar}+{off:#x}");
        }
    }
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
fn every_supported_family_can_allocate_every_engine_object_its_chips_list() {
    // ⊘⊘ `[fable w823, A1]` the class table hard-coded GA10x, so an Ada guest's cuCtxCreate
    // (ADA_COMPUTE_A 0xC9C0) was DENIED by default. Every non-Ampere guest had no channel, no
    // compute object, no copy engine and no doorbell page.
    // ⊘⊘ `[fable w824, HIGH 1]` and the fix carried ONE id per kind, hand-picked per die-group.
    // ⇒ Sets, unioned per family by tools/derive_classes.sh from ogkm's own per-chip lists.
    use classgen::{classes_for, Family, Kind};
    for f in [Family::Turing, Family::Ampere, Family::Ada, Family::Hopper, Family::Blackwell] {
        let c = classes_for(f);
        for k in Kind::ALL {
            assert!(!c.of_kind(k).is_empty(), "{f:?} lists no {k:?} class at all");
            for &id in c.of_kind(k) {
                let pol = rmgraph::class_policy(id);
                assert_ne!(
                    pol,
                    rmgraph::ClassPolicy::Deny("not on the allowlist — default deny"),
                    "{f:?} {k:?} ({id:#x}) is denied — that guest cannot start"
                );
                assert_eq!(rmgraph::class_policy_on(f, id), pol, "the per-family form agrees");
            }
        }
    }
}

#[test]
fn ga100_and_gb202_guests_are_not_denied_their_own_engine_classes() {
    // ★★★ THE FAIL-BEFORE TEST for `[fable w824, HIGH 1]`. These are exactly the ids the
    // one-id-per-kind table lacked, read off `g_gpu_class_list.c`'s halGA100 / halGB202 /
    // halGB20B lists. An A100 or an RTX 50xx guest allocates them for cuCtxCreate.
    use rmgraph::{class_policy, ClassPolicy::*};
    for (id, what) in [
        (0xC6C0, "AMPERE_COMPUTE_A (GA100)"),
        (0xC6B5, "AMPERE_DMA_COPY_A (GA100)"),
        (0xCEC0, "BLACKWELL_COMPUTE_B (GB202/GB20B)"),
        (0xCAB5, "BLACKWELL_DMA_COPY_B (GB202/GB20B)"),
        (0xCA6F, "BLACKWELL_CHANNEL_GPFIFO_B (GB202/GB20B)"),
    ] {
        assert_eq!(class_policy(id), EmulateAndHost, "{what} {id:#x} must be admitted");
    }
    // `[fable w824, MEDIUM 4]` the 3D class, per family, with the policy AMPERE_B had.
    for (id, what) in [
        (0xC597, "TURING_A"), (0xC697, "AMPERE_A (GA100)"), (0xC797, "AMPERE_B"),
        (0xC997, "ADA_A"), (0xCB97, "HOPPER_A"), (0xCD97, "BLACKWELL_A"), (0xCE97, "BLACKWELL_B"),
    ] {
        assert_eq!(class_policy(id), Emulate, "{what} {id:#x} is the 3D sibling: modelled, not hosted");
    }
}

#[test]
fn a_family_lists_its_predecessors_channel_and_usermode_classes_too() {
    // ⊘ Found by the compiler, missed by the audit: Hopper's list carries AMPERE_CHANNEL_GPFIFO_A
    // and TURING_USERMODE_A; Turing's carries VOLTA_*. A driver may allocate any of them and the
    // host RM will accept; refusing them would diverge from hardware.
    use classgen::{classes_for, Family, Kind};
    assert!(classes_for(Family::Hopper).channel_gpfifo.contains(&0xC56F), "AMPERE_CHANNEL_GPFIFO_A on GH100");
    assert!(classes_for(Family::Turing).usermode.contains(&0xC361), "VOLTA_USERMODE_A on TU10x");
    assert_eq!(classes_for(Family::Blackwell).kind_of(0xC461), Some(Kind::Usermode), "TURING_USERMODE_A on GB");
    // …but the compute/copy/3D classes are NOT inherited: a Turing guest gets no HOPPER_COMPUTE_A.
    assert_eq!(
        rmgraph::class_policy_on(Family::Turing, 0xCBC0),
        rmgraph::ClassPolicy::Deny("engine class not listed by any chip of this family")
    );
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
fn the_class_sets_match_what_the_c_compiler_says_each_chip_lists() {
    // ★★★ `[owner]` "don't use regex to parse C code -- use proper parsers/compilers."
    //
    // ⊘ This does not re-implement a parser: it runs `tools/derive_classes.sh`, which COMPILES
    // ogkm's `g_gpu_class_list.c` against a shim, reads the chips off the object's symbol table,
    // joins ids to names through the preprocessor's macro table, and unions per family. If our
    // table ever drifts from what the chips list -- an id missing, an id extra, a chip moved --
    // this fails. ⇒ Set EQUALITY per (family, kind), not membership of a hand-picked few.
    //
    // ⚠ SKIPS (rather than fails) when ogkm or gcc is absent -- "we could not ask" is not "the
    // answer was no", and a box without the oracle must not manufacture a green.
    use std::collections::BTreeSet;
    use std::process::Command;
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/derive_classes.sh");
    if !std::path::Path::new(script).exists() {
        eprintln!("SKIP: no derive_classes.sh");
        return;
    }
    let out = match Command::new("bash").arg(script).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => {
            eprintln!("SKIP: ogkm or gcc unavailable — NOT a pass");
            return;
        }
    };
    use classgen::{Family, Kind, FAMILIES};
    let fam = |s: &str| match s {
        "Turing" => Family::Turing, "Ampere" => Family::Ampere, "Ada" => Family::Ada,
        "Hopper" => Family::Hopper, "Blackwell" => Family::Blackwell, o => panic!("unknown family {o}"),
    };
    // The generator also emits kinds this crate's table does not model (v3 `kf-chip` added
    // twod / inline_to_memory / video_encoder / video_decoder, 2026-09-26): those rows are not
    // this table's to check, so they are skipped by name — an unknown NAME still panics.
    const NOT_MODELLED_HERE: &[&str] = &["twod", "inline_to_memory", "video_encoder", "video_decoder"];
    let kind = |s: &str| match s {
        "channel_gpfifo" => Some(Kind::ChannelGpfifo), "compute" => Some(Kind::Compute), "dma_copy" => Some(Kind::DmaCopy),
        "usermode" => Some(Kind::Usermode), "threed" => Some(Kind::ThreeD),
        o if NOT_MODELLED_HERE.contains(&o) => None,
        o => panic!("unknown kind {o}"),
    };
    let mut derived: std::collections::BTreeMap<(Family, Kind), BTreeSet<u32>> = Default::default();
    let mut chips: std::collections::BTreeMap<Family, BTreeSet<String>> = Default::default();
    let mut n = 0;
    for l in out.lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        match t.first() {
            Some(&"CLASS") => {
                let Some(k) = kind(t[2]) else { continue };
                let id = u32::from_str_radix(t[3].trim_start_matches("0x"), 16).unwrap();
                derived.entry((fam(t[1]), k)).or_default().insert(id);
                n += 1;
            }
            Some(&"FAMILY") => {
                chips.entry(fam(t[1])).or_default().extend(t[2..].iter().map(|s| s.to_string()));
            }
            _ => {}
        }
    }
    assert!(n >= 40, "the script printed {n} CLASS rows — that is not the class list");
    for c in FAMILIES.iter() {
        for k in Kind::ALL {
            let ours: BTreeSet<u32> = c.of_kind(k).iter().copied().collect();
            let theirs = derived.get(&(c.family, k)).cloned().unwrap_or_default();
            assert_eq!(
                ours, theirs,
                "⊘ {:?} {k:?}: our table {ours:#x?} vs what the COMPILER says the chips list {theirs:#x?}",
                c.family
            );
        }
        let ours: BTreeSet<String> = c.chips.iter().map(|s| s.to_string()).collect();
        assert_eq!(&ours, chips.get(&c.family).unwrap(), "{:?} chip membership", c.family);
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


#[test]
fn the_run_scanner_is_not_capped_at_the_match_length() {
    // ⊘ A DEBUGGED BUG, pinned. The first deflate emitted 6 620 bytes for a map that fits in
    // 3 320 — because the run SCANNER stopped at 258, the maximum *match* length. That re-emitted
    // a literal every 258 bytes and doubled the output.
    //
    // ⚠ The 258 limit belongs on each back-reference, never on the run. ★ This test would not
    // have caught the boot-fatal container bug, but it catches the *silent* half: a stream that
    // still inflates correctly and merely stops fitting the reply struct, which is the failure
    // that would reappear years later as "this die does not boot".
    use accessmap::AccessMap;
    let m = AccessMap::deny_all(); // 512 KiB of a single repeated byte — the best case for RLE
    let g = m.to_gzip_deflate();
    assert!(
        g.len() < 4096,
        "a uniform 512 KiB map compressed to {} bytes; a capped run scanner shows up here first",
        g.len()
    );
    // And the ratio itself, as the canary: >100x on uniform input.
    assert!(m.raw().len() / g.len() > 100, "ratio {}x", m.raw().len() / g.len());
}

// ---- fable w824: the timer is written, not read; and per HAL -----------------------------------

#[test]
fn turing_plus_rm_writes_the_legacy_ptimer_and_reads_the_vf_pair() {
    // `[measured in ogkm 610.43.02]` `tmrReadTimeLoReg_TU102`/`_HiReg_TU102` read
    // NV_VIRTUAL_FUNCTION_TIME_0/1 = 0x30080/0x30084 (`timer_tu102.c:135-165`,
    // `tu102/dev_vm.h:224,226`) for every non-Tegra chip (`g_objtmr_nvoc.c:499-503`).
    // `tmrSetCurrentTime_GV100` WRITES NV_PTIMER_TIME_1 (0x9410) then _0 (0x9400)
    // (`timer_gv100.c:71-72`) once at boot (`kernel_gsp.c:5039`) and at resume
    // (`gpu_suspend.c:236`), after testing PLM 0x9430 bit 4 (`:56`).
    // ⇒ The refusal is on the WRITE arm and the VF pair is never refused.
    // Contradicted by: a read-path refusal (the old shape), or a VF_TIME offset in any refusal set.
    use timer::*;
    assert_eq!((VF_TIME_0, VF_TIME_1), (0x30080, 0x30084));
    assert_eq!(TIMER_GV100.refused_writes, &[0x9400, 0x9410]);
    for t in [TIMER_GV100, TIMER_GH100, TIMER_GB10B] {
        assert!(!t.is_refused_write(VF_TIME_0) && !t.is_refused_write(VF_TIME_1), "{:?}", t.hal);
    }
    // ⊘ A COMPILE-TIME gate, not a text one: the match below is exhaustive only while
    // `ReadSource` has no trapping variant. (A `contains("RefuseByName")` gate matched the doc
    // comment that DESCRIBES the old defect -- visibility, not reachability.)
    // ⊘⊘ w824: the legacy timer page is no longer *a shadow chosen over a trap* -- there is no
    // trap arm to choose against. The PLM value is recomputed on the trapped WRITE that moves it.
    match trappolicy::ReadSource::ComputedShadow {
        trappolicy::ReadSource::Shadow => {}
        trappolicy::ReadSource::ComputedShadow => {}
    }
    assert!(!trappolicy::may_trap_read(vmm::Bar(0), 0x9400, classgen::Family::Ampere));
    // And the write side must actually be consulted on the privileged arm -- in code, not prose.
    let trap: String = include_str!("../src/trap.rs")
        .lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    assert!(trap.contains("self.timer.is_refused_write(off)"), "the refusal must be consulted on the write path");
}

#[test]
fn the_ptimer_write_is_refused_by_name_and_the_plm_shadow_lets_ogkm_succeed() {
    // ★ The decision, pinned (see `timer::TimerRegs`): drop the write, answer
    // WRITE_PROTECTION_LEVEL0 = ENABLE (bit 4, `gv100/dev_timer.h:29-30`) so ogkm takes its `if`
    // branch and returns NV_OK rather than NV_ASSERT(0) + NV_ERR_PRIV_SEC_VIOLATION
    // (`timer_gv100.c:77-81`). An offset is not an option because the VF pair is a read-only
    // memslot over live host time with no exit to add it in.
    // Contradicted by: serving PLM = DISABLE, or accepting the write as Plain.
    let vmm = Vmm::new();
    let p = Plane::for_family(&vmm, 4, 0x3, classgen::Family::Ampere, timer::GA106_BAR0_BYTES);
    let priv_ = Class::Privileged { readable: true, semantics: WriteSemantics::Plain };
    assert_eq!(p.trap_write(priv_, 0, 0x9410, 0x1234, 4), Action::RefusedByName);
    assert_eq!(p.trap_write(priv_, 0, 0x9400, 0x5678, 4), Action::RefusedByName);
    assert_eq!(p.ring.occupancy(), 0, "nothing queued for the drainer to apply to the host");
    assert_eq!(p.timer.plm_shadow(), Some((0x9430, timer::PLM_WRITE_PROTECTION_LEVEL0_ENABLE)));
}

#[test]
fn hopper_and_blackwell_set_time_through_the_sci_offset_and_gb10b_writes_nothing() {
    // `[measured in ogkm 610.43.02]` `g_objtmr_nvoc.c:420-445` dispatches tmrSetCurrentTime by
    // HAL: GV100 for TU/GA/AD; GB10B for GB10B|GB20B|GB20C; GH100 for everything else (GH100,
    // GB100/102/110/112, GB202-207, GR100/102). GH100 writes NV_PGC6_SCI_SYS_TIMER_OFFSET_1/0
    // (0x118df8/0x118df4, `gh100/dev_gc6_island.h:35,41`, `timer_gh100.c:89-91`); GB10B keeps
    // the offset in software (`timer_gb10b.c:46-73`) and writes NO register.
    // ⊘ The audit said "Hopper/Blackwell use NV_PGC6_SCI_SEC_TIMER"; the integrated Blackwell
    // parts do not -- found while reading the dispatch, pinned so it is not re-derived.
    use timer::*;
    assert_eq!(timer_regs_for(classgen::Family::Hopper), TIMER_GH100);
    assert_eq!(timer_regs_for(classgen::Family::Blackwell), TIMER_GH100);
    assert_eq!(TIMER_GH100.refused_writes, &[0x118df4, 0x118df8]);
    assert!(TIMER_GB10B.refused_writes.is_empty());
    // The GV100 pair is NOT refused on the GH100 HAL: those offsets are not the timer there.
    assert!(!TIMER_GH100.is_refused_write(0x9400));
    let vmm = Vmm::new();
    let p = Plane::for_family(&vmm, 4, 0x3, classgen::Family::Hopper, timer::GA106_BAR0_BYTES);
    let priv_ = Class::Privileged { readable: true, semantics: WriteSemantics::Plain };
    assert_eq!(p.trap_write(priv_, 0, 0x118df4, 1, 4), Action::RefusedByName);
    assert_ne!(p.trap_write(priv_, 0, 0x9400, 1, 4), Action::RefusedByName);
}

#[test]
fn the_element_layout_is_selected_by_the_mctp_header_ogkm_itself_validates() {
    // `[measured in ogkm 610.43.02]` word 0 is built with MCTP_HEADER_VERSION 3:0 = 1
    // (`mctp_format.h:40,79`; `message_queue_cpu.c:505-511`) and rejected on receive unless 1
    // (`:739-746`); word 1 carries type 0x7e / vendor 0x10de (`:750`). At 580 word 0 is
    // authTagBuffer[0..4] (`message_queue_priv.h:45`), zero outside CC.
    // Contradicted by: any `driver_major >= N` selection returning.
    // Code lines only: the module doc legitimately NAMES the old `layout_for(driver_major)`.
    let code: String = include_str!("../src/element.rs")
        .lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    assert!(!code.contains("driver_major"), "version-sniffing must stay gone");
    let mut e = [0u8; 16];
    e[0..4].copy_from_slice(&element::MCTP_WORDS_610[0].to_le_bytes());
    e[4..8].copy_from_slice(&element::MCTP_WORDS_610[1].to_le_bytes());
    assert_eq!(element::detect_layout(&e), Some(element::LAYOUT_610));
    assert_eq!(element::detect_layout(&[0u8; 48]), Some(element::LAYOUT_580));
}

#[test]
fn bar0_size_comes_from_card_info_and_the_ga106_value_is_a_named_default() {
    // `[fable w824, LOW 5]` §50 level 1: NV_ESC_CARD_INFO.reg_size (`nv-ioctl.h:63`, from
    // `nv->regs->size` at `nv.c:2384`, dispatched `nv.c:2593` under NV_CTL_DEVICE_ONLY, no admin
    // check). Both consumers now take the value; 16 MiB is named as GA106's, not as BAR0's.
    // Contradicted by: a constructor that ignores its bar0_bytes argument.
    use accessmap::AccessMap;
    let m = AccessMap::deny_all_for(64 << 20);
    assert_eq!(m.raw().len(), accessmap::map_bytes_for(64 << 20));
    assert_eq!(m.raw().len(), 4 * accessmap::MAP_BYTES);
    // ⊘⊘ MEASURED while writing this test, and it is a finding the audit missed: the fixed-Huffman
    // encoder costs 13 bits per 258-byte run (length code 285 = 8 bits + distance code = 5), so a
    // deny-all map compresses LINEARLY in BAR0 size: 16 MiB → 3 320 B, 64 MiB → 13 223 B. That
    // fits 610's 16 384-byte cap and does NOT fit 580's 4 096 -- a 580 guest on a device with
    // BAR0 above ~20 MiB could not be served this map at all, and the only fallback ogkm offers
    // is compressedSize=0 ⇒ the 0xFF "everything is userspace-mappable" map that §47 forbids.
    // ⇒ Pinned as measured. Lifting it needs a dynamic-Huffman block (≈6 bits/run), not a tweak.
    let g = m.to_gzip_deflate();
    assert!(g.len() <= AccessMap::MAX_COMPRESSED_610, "64 MiB deny-all map compressed to {}", g.len());
    assert!(
        g.len() > AccessMap::MAX_COMPRESSED_580,
        "if this now FITS 580's cap the encoder improved — move the bound, and re-check the 16 MiB size"
    );
    assert_eq!(
        AccessMap::deny_all_for(64 << 20).raw().len(),
        accessmap::map_bytes_for(64 << 20),
        "one bit per 32-bit register — the inflated size ogkm demands"
    );
    assert_eq!(timer::GA106_BAR0_BYTES, accessmap::GA106_BAR0_BYTES, "one default, two consumers");
}

// ---- owner ruling w823: where a trap may exist at all ------------------------------------------

#[test]
fn bar2_is_never_trapped_and_bar1_only_for_the_doorbell_page() {
    // `[owner]` "no traps for bar1/2 (except doorbell in bar1)".
    use trappolicy::{may_trap_write, DoorbellPlacement};
    use vmm::Bar;
    let pre_hopper = DoorbellPlacement::Bar0 { offset: 0x90 };
    let hopper = DoorbellPlacement::Bar1 { page_base: 0x9_0000 };

    // BAR2: never, under any placement.
    for d in [pre_hopper, hopper] {
        for off in [0u64, 0x1000, 0x10_0000, 0x1FF_F000] {
            assert!(!may_trap_write(Bar(2), off, d), "BAR2 must never trap ({off:#x})");
        }
    }
    // BAR1 with the doorbell elsewhere: never.
    for off in [0u64, 0x9_0000, 0x10_0000] {
        assert!(!may_trap_write(Bar(1), off, pre_hopper), "BAR1 must not trap pre-Hopper");
    }
    // BAR1 on Hopper+: exactly the doorbell page, and nothing either side of it.
    assert!(may_trap_write(Bar(1), 0x9_0000, hopper));
    assert!(may_trap_write(Bar(1), 0x9_FFFF, hopper), "the whole 64 KiB page");
    assert!(!may_trap_write(Bar(1), 0x8_FFFF, hopper), "⊘ not the page below");
    assert!(!may_trap_write(Bar(1), 0xA_0000, hopper), "⊘ not the page above");
}

#[test]
fn bar0_may_trap_writes_but_never_in_pramin() {
    // `[owner]` "write trap allowed in bar0 (not in pramin)". PRAMIN is a BRING-UP aperture:
    // trapping it puts a boot-time loop through the privileged ring.
    use trappolicy::{may_trap_write, DoorbellPlacement, PRAMIN_BASE, PRAMIN_LEN};
    use vmm::Bar;
    let d = DoorbellPlacement::Bar0 { offset: 0x90 };
    assert!(may_trap_write(Bar(0), 0x110c00, d), "the GSP RPC submit register is trappable");
    assert!(!may_trap_write(Bar(0), PRAMIN_BASE, d), "⊘ PRAMIN start");
    assert!(!may_trap_write(Bar(0), PRAMIN_BASE + PRAMIN_LEN - 4, d), "⊘ PRAMIN end");
    assert!(may_trap_write(Bar(0), PRAMIN_BASE - 4, d), "just below PRAMIN is fine");
    assert!(may_trap_write(Bar(0), PRAMIN_BASE + PRAMIN_LEN, d), "just above PRAMIN is fine");
}

#[test]
fn no_read_is_trapped_on_the_product_target_and_hopper_blackwell_are_bounded() {
    // ⊘⊘⊘ `[owner]` "no read trap everywhere" -- and this SUPERSEDES §5's 524-page read-trap
    // allowlist. §5 itself measured the cost: 99% of the C artifact's exits per token were READS
    // of ONE firmware debug register, costing 2.5x on LLM decode. An allowlist only has to be
    // wrong about one page to pay that.
    //
    // ★ The latch case does not need an exit: a latch is SET BY A WRITE, and writes are trapped,
    // so the value a read must return is computed into the shadow at write time -- which §5
    // already does for the timer page.
    //
    // ⊘⊘⊘ AND THIS TEST'S CLAIM WAS TOO STRONG — CORRECTED w824, NOT PATCHED. Its title says
    // "anywhere" and it asserted exactly that, family-blind. `[fable w824]` produced a real
    // counterexample: the falcon PIO auto-increment data port, where each READ advances a
    // hardware cursor and ogkm then ASSERTS the cursor moved. ⇒ "no read trap anywhere" is true
    // of the PRODUCT TARGET and false of Hopper/Blackwell, and a test that keeps asserting the
    // strong form would have to be deleted the day Hopper is supported — which is how a green
    // test holds a wall in place.
    use trappolicy::may_trap_read;
    use vmm::Bar;

    // ★ The product target: no read exit, anywhere, at any offset, on any BAR.
    for family in [classgen::Family::Turing, classgen::Family::Ampere, classgen::Family::Ada] {
        for bar in [0u8, 1, 2] {
            for off in [0u64, 0x9000, 0x110c00, 0x70_0000, 0x81_0000, 0x8F_2000, 0x84_0000] {
                assert!(
                    !may_trap_read(Bar(bar), off, family),
                    "{family:?} BAR{bar}+{off:#x} must not read-trap"
                );
            }
        }
    }

    // ⊘ Hopper/Blackwell: read exits exist, and they are EXACTLY the named boot pages. The bound
    // is the claim now — an unbounded exception would be the same rot under a new name.
    for family in [classgen::Family::Hopper, classgen::Family::Blackwell] {
        let holes: Vec<u64> = memmap::holes_for(family).iter().map(|(p, _)| *p).collect();
        for off in [0u64, 0x9000, 0x110c00, 0x70_0000, 0x81_0000] {
            assert!(!may_trap_read(Bar(0), off, family), "{family:?} {off:#x}");
        }
        for page in &holes {
            assert!(may_trap_read(Bar(0), *page, family), "{family:?} {page:#x} should read-trap");
            assert!(may_trap_read(Bar(0), page + 0xFFC, family), "the whole page, not one register");
        }
        // ★ Never outside BAR0, whatever the family.
        for bar in [1u8, 2] {
            for page in &holes {
                assert!(!may_trap_read(Bar(bar), *page, family), "{family:?} BAR{bar}");
            }
        }
    }
}

// ---- w824: the read trap is a DELETE, and the non-GSP half of the argument --------------------

#[test]
fn the_vmm_registers_write_regions_only_and_pramin_is_not_among_them() {
    // `[owner w824]` "no read trap everywhere, and write trap allowed in bar0 (not in pramin, but
    // is allowed only if doorbell is mapped in bar1 and then only that page)".
    // ⇒ Asserted on the LIST THE VMM ACTUALLY REGISTERS, not on the predicate — a correct
    // predicate consulted by nobody is the orphan class this suite exists to catch.
    use trappolicy::{doorbell_for, trap_regions, DoorbellPlacement, PRAMIN_BASE, PRAMIN_LEN};
    use classgen::Family;

    for f in [Family::Turing, Family::Ampere, Family::Ada, Family::Hopper, Family::Blackwell] {
        let d = doorbell_for(f);
        let regions = trap_regions(d, 16 << 20);

        // ⊘ PRAMIN is in NO region, under any family.
        for r in &regions {
            let overlaps = r.bar.0 == 0
                && r.base < PRAMIN_BASE + PRAMIN_LEN
                && PRAMIN_BASE < r.base + r.len;
            assert!(!overlaps, "{f:?}: region {r:?} overlaps PRAMIN");
        }
        // ⊘ BAR2 is in NO region, under any family.
        assert!(regions.iter().all(|r| r.bar.0 != 2), "{f:?}: BAR2 registered");

        // ★ BAR1 appears exactly when the doorbell is there, and then only as that one page.
        let bar1: Vec<_> = regions.iter().filter(|r| r.bar.0 == 1).collect();
        match d {
            DoorbellPlacement::Bar0 { .. } => {
                assert!(bar1.is_empty(), "{f:?}: BAR1 trapped with a BAR0 doorbell");
            }
            DoorbellPlacement::Bar1 { page_base } => {
                assert_eq!(bar1.len(), 1, "{f:?}: BAR1 must contribute exactly one region");
                assert_eq!((bar1[0].base, bar1[0].len), (page_base, 0x1_0000));
            }
        }
    }
    // ★ And the BAR0 total is the BAR minus PRAMIN exactly — no silent under- or over-trapping.
    let r = trap_regions(doorbell_for(Family::Ampere), 16 << 20);
    let bar0: u64 = r.iter().filter(|r| r.bar.0 == 0).map(|r| r.len).sum();
    assert_eq!(bar0, (16 << 20) - PRAMIN_LEN, "BAR0 traps everything but PRAMIN");
}

#[test]
fn a_write_the_design_forbids_never_reaches_the_classifier() {
    // ⊘ The structural gate runs ABOVE the three-way classifier. A VMM that registers a region we
    // never asked for (or a future BAR2 fill path) must be a no-op, not a classifier decision.
    // Contradicted by: trap_write dispatching on PRAMIN or BAR2.
    let vmm = Vmm::new();
    let p = Plane::for_family(&vmm, 4, 0x3, classgen::Family::Ampere, timer::GA106_BAR0_BYTES);
    let priv_ = Class::Privileged { readable: true, semantics: WriteSemantics::Plain };
    for (bar, off) in [(0u8, 0x0070_0000u32), (0, 0x0077_FFFC), (2, 0x1000), (1, 0x9_0000)] {
        assert_eq!(
            p.trap_write(priv_, bar, off, 0xdead_beef, 4),
            Action::None,
            "bar{bar}+{off:#x} is outside the sanctioned set and must be a no-op"
        );
    }
    // ★ KNOWN-POSITIVE, and it took two tries to make it one. The first version used an offset
    // just past PRAMIN's end and asserted the action was not `None` — but a plain privileged
    // write to an unremarkable register IS `None`, so that assertion failed against a CORRECT
    // gate. `Action::None` is overloaded: "structurally refused" and "classified, nothing to do"
    // are the same value. ⇒ The known-positive must use an offset where the classifier produces a
    // DISTINGUISHABLE action, and `Action::None` alone can never witness the gate.
    assert_eq!(
        p.trap_write(priv_, 0, 0x9400, 0xdead_beef, 4),
        Action::RefusedByName,
        "⊘ if this is None too, the gate is refusing everything and the test above proves nothing"
    );
    // ★ And the span past PRAMIN is genuinely registered — the gate lets it through, the
    // classifier simply has nothing to do there.
    assert!(trappolicy::may_trap_write(
        vmm::Bar(0),
        0x0080_0000,
        trappolicy::doorbell_for(classgen::Family::Ampere)
    ));
}

#[test]
fn the_cpu_may_move_a_register_and_nothing_larger() {
    // `[owner w824]` "confirm that CPU copies are dead and dead from gpu vidmem ... anything
    // larger than some trivial integer size from mmio reads then".
    //
    // ⊘⊘⊘ §46, AND THE MEASUREMENT THAT MAKES IT A CONSTRAINT RATHER THAN A PREFERENCE:
    // `[w797]` turning the CPU executor off took the 30-arm suite from 18 PASS to 15. The six
    // arms that "regressed" had been passing BECAUSE OUR CPU WAS DOING THE GPU'S WORK. ⇒ 15/30
    // is the HONEST number and 18 was the flattering one — the CPU executor buys green arms with
    // a lie the content ledger cannot see (both executors write identical bytes).
    use channel::{may_cpu_move, CpuMoveRefusal, CPU_MOVE_MAX_BYTES};

    // ★ A register-sized answer is fine — that is what an MMIO read IS.
    for len in [1u64, 2, 4, 8] {
        assert!(may_cpu_move(len, false).is_ok(), "{len} bytes is a register, not a copy");
    }
    // ⊘ One byte past a 64-bit access is DATA, and data is the engine's job.
    assert_eq!(
        may_cpu_move(9, false).unwrap_err().name(),
        "cpu_move_too_large",
        "9 bytes is no longer a register access"
    );
    // ⚠ KNOWN-POSITIVE for the bound itself: the CeUtils scrub measured FOUR bytes `[w740]`, so
    // it passes — and a scrub that grew to a page would be refused, which is the case that
    // matters. If this ever starts failing, the scrub changed shape and §46 needs re-deriving.
    assert!(may_cpu_move(4, false).is_ok(), "the measured 4-byte CeUtils scrub");
    assert!(may_cpu_move(4096, false).is_err(), "a page-sized 'scrub' is the GPU's work");

    // ★★★ And real card vidmem is refused at ANY size, including a register.
    for len in [1u64, 4, 8, 4096] {
        assert_eq!(
            may_cpu_move(len, true).unwrap_err().name(),
            "cpu_move_names_real_vidmem",
            "card vidmem is refused at {len} bytes — size does not buy access"
        );
    }
    assert_eq!(CPU_MOVE_MAX_BYTES, 8, "the widest single MMIO access");
}
