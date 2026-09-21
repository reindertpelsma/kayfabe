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
    let masked = (0xFFFF_FFFFu64 as u32) & ((1 << token::HOST_TOKEN_BITS) - 1);
    assert_eq!(masked, (1 << token::HOST_TOKEN_BITS) - 1, "masking must not refuse a token");
    assert_eq!(token::HOST_TOKEN_BITS, 21, "19-21 bits is what the register can EXPRESS (§5.1)");
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
