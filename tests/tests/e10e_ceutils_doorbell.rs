//! ★★★ **E10e item (c) — the whole shell driver, end to end, GPU-free.**
//!
//! `memmgrTestCeUtils`' `MemSet` as the guest actually issues it, on a channel with **no
//! `Vas`**: a GPFIFO ring at a GPU virtual address, a pushbuffer at another, a
//! remap-enabled `LAUNCH_DMA` whose destination is virtual, and a finishPayload semaphore
//! four bytes above the host-FIFO one. Every address is resolved by descending the guest's
//! own GA10x page tables from the root the guest published, out of the emulated
//! framebuffer — `kayfabe_device::ceresolve`, driven through `kayfabe_rt::ceutils`.
//!
//! # ⊘ The three ways this can be green and wrong, and the assertion that catches each
//!
//! 1. **The bytes land at the VIRTUAL address.** §14.14's REFUTED 4, measured. Every
//!    address here is deliberately `phys != va`, so an executor that used `CeSubCopy::dst`
//!    writes a page nothing reads and the readback fails.
//! 2. **The completion advances the WRONG WORD.** `channelWaitForFinishPayload` spins on
//!    `pbGpuVA + finishPayloadOffset`; the host-FIFO semaphore is at
//!    `pbGpuVA + semaOffset`, **four bytes lower** (`ogkm-580: channel_utils.c:250`).
//!    Advancing the lower one logs a completion, satisfies our counters and leaves the
//!    guest spinning forever. So the lower word is **poisoned** and asserted untouched.
//! 3. **A doorbell that could not do the work reports Served** (§14.8). The refusal arms
//!    assert on the *exact* fault, on zero bytes moved, on zero interrupts, and on the
//!    ring cursor **not** having advanced — a cursor advanced through a refusal turns a
//!    loud failure into a silently dropped copy.

use kayfabe_abi::gvaspacepdes::{GMMU_FMT_MAX_LEVELS, PdeLevel, ServerReservedPdes};
use kayfabe_abi::submit;
use kayfabe_device::gvaspub::GvasPublication;
use kayfabe_device::{FbStore, NanoClock, RegPlane, SparseFb, SteppingClock};
use kayfabe_fwd::FwdFault;
use kayfabe_mocks::MockVmm;
use kayfabe_rt::ceutils::{CeUtilsChannel, GpCursor, MethodState, run_submission};
use kayfabe_vmm::{IrqSpec, Vmm};

// =====================================================================================
// The guest's own geometry, from RM's own constants
// =====================================================================================

/// `pChannel->pbGpuVA` for the channel that walls (`[measured 2026-08-08, boot
/// run_p35_84d857d]`).
const PB_GPU_VA: u64 = 0x4_2000_0000;
/// `channelPbSize = CE_METHOD_SIZE_PER_BLOCK × NUM_COPY_BLOCKS = 0x64 × 4096`
/// (`ogkm-580: ce_utils_sizes.h:27, :35`).
const CHANNEL_PB_SIZE: u64 = 0x64 * 4096;
/// `pbGpuVA + channelPbSize` — where RM writes its GPFIFO entries.
const RING_VA: u64 = PB_GPU_VA + CHANNEL_PB_SIZE;
/// `finishPayloadOffset = channelPbSize + GPFIFO_SIZE + 4` (`channel_utils.c:243-250`).
const FINISH_PAYLOAD_OFFSET: u64 = CHANNEL_PB_SIZE + 0x8000 + 4;
/// `semaOffset = channelPbSize + GPFIFO_SIZE` — the HOST semaphore, four bytes lower.
const SEMA_OFFSET: u64 = CHANNEL_PB_SIZE + 0x8000;
/// The first submission's method block: `putIndex = lastSubmittedEntry + 1 = 1`
/// (`channel_utils.c:406, :489`). ★ The device printed exactly this: `gp0=0x420000064`.
const PUSH_VA: u64 = PB_GPU_VA + 0x64;
/// The `MemSet` destination — a framebuffer surface the kernel aliased into this VAS.
const DST_VA: u64 = PB_GPU_VA + 0x10_0000;

/// The 512 MiB leaf's physical base in **guest RAM**. ⊘ Deliberately unequal to the VA and
/// not even congruent to it: `[measured 2026-08-08, boot run_p35_84d857d, rev 84d857d, vast
/// GA106 / 580.159.04 Open]` the walling channel resolved `0x4_2006_4000 → 0x2f2c_3000`,
/// i.e. a different number space entirely.
const LEAF_PHYS: u64 = 0x2f00_0000;

/// Physical of `va`, given the identity the tree below installs.
const fn phys_of(va: u64) -> u64 {
    LEAF_PHYS + (va - PB_GPU_VA)
}

/// The published page-directory root, in the emulated framebuffer.
const ROOT_FB: u64 = 0x0010_0000;
const PD2_FB: u64 = 0x0011_0000;
const PD1_FB: u64 = 0x0012_0000;

const CLIENT: u32 = 0xc1e0_0006;
const VASPACE: u32 = 0x0a;

/// `GMMU_APERTURE_VIDEO`, from the enum the control's `levels[].aperture` carries
/// (`ogkm-580: gmmu_fmt.h:280-325`).
const AP_VIDEO: u32 = 1;

// =====================================================================================
// GA10x page-table entries, encoded as the guest writes them
// =====================================================================================

/// A single (non-dual) PDE pointing at a **vidmem** sub-table: aperture `1` at bits `2:1`,
/// address at `32:8` shifted 12 (`dev_mmu.h:111-113`).
fn pde_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}

/// A `PD1` slot with the valid bit set is a **512 MiB page** on GA10x, not a directory.
/// Aperture `2` = `SYS_COH` in the **PTE** table (⚠ not the PDE table: a PTE's `0` is
/// video, `kern_gmmu_fmt_gm10x.c:184-201`).
fn leaf_512m_sysmem(phys: u64) -> u64 {
    ((phys >> 12) << 8) | (2 << 1) | 1
}

// =====================================================================================
// The fixture
// =====================================================================================

fn plane_with_tree() -> RegPlane {
    let plane = RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        kayfabe_device::abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");
    plane.set_fb(Box::new(SparseFb::new(
        kayfabe_device::ga10x::GA106.fb_length,
    )));
    plane.set_mmu(Box::new(kayfabe_chips::Ga10xGmmu::new()));

    // The publication the guest made for THIS `(hClient, hVASpace)` — `levels[0]` is the
    // root, and its aperture says the directories are in the framebuffer.
    let mut levels = [PdeLevel {
        phys_address: 0,
        size: 0,
        aperture: 0,
        page_shift: 0,
    }; GMMU_FMT_MAX_LEVELS];
    levels[0] = PdeLevel {
        phys_address: ROOT_FB,
        size: 0x20,
        aperture: AP_VIDEO,
        page_shift: 47,
    };
    plane.gvas_pub_log().note(GvasPublication {
        cmd: kayfabe_abi::gvaspacepdes::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        client: CLIENT,
        object: VASPACE,
        pdes: ServerReservedPdes {
            h_subdevice: 0,
            subdevice_id: 0,
            page_size: 0x20_0000,
            virt_addr_lo: 0x1_0000_0000,
            virt_addr_hi: 0x1_1fff_ffff,
            num_levels: 4,
            levels,
        },
        count: 1,
    });

    // The tree itself, written into the framebuffer the guest built it in. Indices are the
    // format's own bit ranges: PD3 at 47, PD2 at 38, PD1 at 29.
    let pd3 = (PB_GPU_VA >> 47) as usize;
    let pd2 = ((PB_GPU_VA >> 38) & 511) as usize;
    let pd1 = ((PB_GPU_VA >> 29) & 511) as usize;
    plane
        .ce_session(
            CLIENT,
            VASPACE,
            kayfabe_device::ceresolve::Demand::from_doorbell(),
            |ce| {
                let put = |fb: &mut dyn FbStore, at: u64, e: u64| {
                    fb.write(at, &e.to_le_bytes()).expect("inside the FB");
                };
                put(ce.fb(), ROOT_FB + 8 * pd3 as u64, pde_vid(PD2_FB));
                put(ce.fb(), PD2_FB + 8 * pd2 as u64, pde_vid(PD1_FB));
                put(
                    ce.fb(),
                    PD1_FB + 8 * pd1 as u64,
                    leaf_512m_sysmem(LEAF_PHYS),
                );
            },
        )
        .expect("the publication is present, so a session opens");
    plane
}

/// RM's own `memset` block, method run for method run
/// (`ogkm-580: channel_utils.c:880-990` + `:1029-1033`), with the engine-class completion
/// `channelWaitForFinishPayload` spins on.
///
/// ⚠ `SET_SEMAPHORE_A` is the address's **HIGH** half — the reverse of the host FIFO's
/// `SEM_ADDR_LO` first. Getting it backwards points the release at a different page.
fn memset_block(dst_va: u64, bytes: u32, value: u8, sema_va: u64, payload: u32) -> Vec<u8> {
    let sub = 0u32;
    let hdr = |m, n| submit::method_header_inc(sub, m, n).expect("encodable");
    // `DST_X = CONST_A | COMPONENT_SIZE_ONE | NUM_DST_COMPONENTS_ONE` — and both `_ONE`s
    // are the literal **zero** (the field is size minus one), so RM's scrub map is just
    // the `CONST_A` selector. `LINE_LENGTH_IN` then counts single BYTES.
    let components = submit::ce::REMAP_DST_SEL_CONST_A;
    let flags = submit::ce::LAUNCH_TRANSFER_NON_PIPELINED
        | submit::ce::LAUNCH_FLUSH_ENABLE
        | submit::ce::LAUNCH_REMAP_ENABLE
        | submit::ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD;
    let runs: Vec<(u32, Vec<u32>)> = vec![
        (
            hdr(submit::SET_OBJECT, 1),
            vec![kayfabe_abi::generated::classes::AMPERE_DMA_COPY_B],
        ),
        (
            hdr(submit::ce::SET_REMAP_CONST_A, 1),
            vec![u32::from_le_bytes([value; 4])],
        ),
        (hdr(submit::ce::SET_REMAP_COMPONENTS, 1), vec![components]),
        (
            hdr(submit::ce::OFFSET_OUT_UPPER, 2),
            vec![(dst_va >> 32) as u32, dst_va as u32],
        ),
        (hdr(submit::ce::LINE_LENGTH_IN, 1), vec![bytes]),
        (
            hdr(submit::ce::SET_SEMAPHORE_A, 3),
            vec![
                ((sema_va >> 32) as u32) & submit::ce::SET_SEMAPHORE_A_UPPER_MASK,
                sema_va as u32,
                payload,
            ],
        ),
        (hdr(submit::ce::LAUNCH_DMA, 1), vec![flags]),
    ];
    let mut out = Vec::new();
    for (h, args) in runs {
        out.extend_from_slice(&h.to_le_bytes());
        for a in args {
            out.extend_from_slice(&a.to_le_bytes());
        }
    }
    out
}

/// Guest RAM with the ring entry, the pushbuffer and the two semaphore words in place.
fn guest_ram(block: &[u8]) -> MockVmm {
    let mut vmm = MockVmm::new();
    let entry = submit::gp_entry(PUSH_VA, block.len() as u64).expect("representable");
    vmm.gpa_write(phys_of(RING_VA), &entry.to_le_bytes())
        .expect("the ring's first entry");
    vmm.gpa_write(phys_of(PUSH_VA), block)
        .expect("the method block");
    // ⊘ Both semaphore words start at zero and the LOWER one is poisoned, so "advanced the
    // word four bytes down" is a failure and not a coincidence.
    vmm.gpa_write(
        phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET),
        &0u32.to_le_bytes(),
    )
    .expect("finishPayload");
    vmm.gpa_write(
        phys_of(PB_GPU_VA + SEMA_OFFSET),
        &0xDEAD_BEEFu32.to_le_bytes(),
    )
    .expect("the host semaphore, poisoned");
    vmm
}

fn channel() -> CeUtilsChannel {
    CeUtilsChannel {
        client: CLIENT,
        vaspace: VASPACE,
        ring_va: RING_VA,
        ring_entries: 4096,
    }
}

/// Drive one doorbell's worth of work through the shell driver.
fn ring_once(
    plane: &RegPlane,
    vmm: &mut MockVmm,
    cursor: &mut GpCursor,
) -> Result<kayfabe_rt::ceutils::CeUtilsRun, kayfabe_rt::ceutils::CeUtilsRefusal> {
    let mut state = MethodState::new();
    ring_once_with(plane, vmm, cursor, &mut state)
}

/// ★★★★ The same doorbell, driven against a **channel** accumulator the caller keeps —
/// the adapter's real shape (`CeShellState::states`, keyed and committed exactly like the
/// cursor). ⊘ `ring_once` above is this with a fresh state each time, i.e. the behaviour
/// that walled, kept so the two can be compared in one test file.
fn ring_once_with(
    plane: &RegPlane,
    vmm: &mut MockVmm,
    cursor: &mut GpCursor,
    state: &mut MethodState,
) -> Result<kayfabe_rt::ceutils::CeUtilsRun, kayfabe_rt::ceutils::CeUtilsRefusal> {
    let pb = kayfabe_chips::Ga10xPushbuffer;
    let out = plane
        .ce_session(
            CLIENT,
            VASPACE,
            kayfabe_device::ceresolve::Demand::from_doorbell(),
            |ce| run_submission(ce, &pb, vmm, channel(), *cursor, *state),
        )
        .expect("the publication is present");
    // ★ The adapter's own discipline, reproduced: commit the advanced cursor AND the
    // accumulator ONLY on success. A refusal hands neither back, which is what makes both
    // the skipped entry and the half-applied engine state impossible.
    if let Ok(run) = &out {
        *cursor = run.cursor;
        *state = run.state;
    }
    out
}

// =====================================================================================
// ★★★ THE ACCEPTANCE
// =====================================================================================

/// ★★★ **The whole chain: ring → pushbuffer → decode → partition → CPU fill → the RIGHT
/// completion word.**
#[test]
fn a_ceutils_memset_on_a_vas_less_channel_fills_the_resolved_page_and_releases_the_finish_payload()
{
    const LEN: u32 = 0x400;
    const VALUE: u8 = 0xAB;
    const PAYLOAD: u32 = 1; // `target=0x1`, the value the guest printed at the wall.
    let plane = plane_with_tree();
    let block = memset_block(
        DST_VA,
        LEN,
        VALUE,
        PB_GPU_VA + FINISH_PAYLOAD_OFFSET,
        PAYLOAD,
    );
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();

    let run = ring_once(&plane, &mut vmm, &mut cursor).expect("the submission is servable");

    assert_eq!(
        run.entries, 1,
        "one GPFIFO entry, and the ring stopped there"
    );
    assert_eq!(run.launches, 1, "one LAUNCH_DMA");
    assert_eq!(
        run.bytes,
        u64::from(LEN),
        "★ LINE_LENGTH_IN counts BYTES for RM's 1-byte element map"
    );
    assert_eq!(run.completions, 1);
    assert_eq!(cursor.next, 1, "the ring cursor advanced past what it ran");

    // ★★★ 1. THE BYTES, at the PHYSICAL address the walk produced — not at the VA.
    assert_eq!(
        vmm.ram_read(phys_of(DST_VA), LEN as usize),
        vec![VALUE; LEN as usize],
        "★ the fill landed at the resolved physical of the destination VA"
    );
    assert_eq!(
        vmm.ram_read(phys_of(DST_VA) + u64::from(LEN), 4),
        vec![0u8; 4],
        "⊘ and it stopped exactly at LINE_LENGTH_IN bytes"
    );

    // ★★★ 2. THE COMPLETION WORD — the finishPayload, and NOT the host semaphore four
    // bytes lower. This is the assertion the whole increment turns on.
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        PAYLOAD.to_le_bytes(),
        "★★★ `channelWaitForFinishPayload` polls THIS word"
    );
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + SEMA_OFFSET), 4),
        0xDEAD_BEEFu32.to_le_bytes(),
        "⊘ the HOST semaphore four bytes lower is UNTOUCHED — advancing it would log a \
         completion and leave the guest spinning forever (#12, displaced by four bytes)"
    );
    assert_eq!(
        run.completion_at,
        Some((
            kayfabe_arch::ids::GpuVa(PB_GPU_VA + FINISH_PAYLOAD_OFFSET),
            kayfabe_arch::CpuPlane::GuestRam,
            phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET),
        )),
        "the report states where the completion landed, in the aperture it landed in"
    );

    // ★★★ 3. THE INTERRUPT, once, and after.
    assert_eq!(vmm.irqs, vec![IrqSpec::Msix(0)]);
}

/// ⊘ **A second doorbell with no new entry is REFUSED, not served over nothing.**
///
/// The cursor has passed the one written entry and the next is zero, which decodes to
/// nothing. §14.8's whole finding is that reporting *Served* here is worse than the wall it
/// replaced, so the driver refuses and the guest's own timeout stays the diagnosis.
#[test]
fn a_doorbell_that_brings_no_new_entry_is_refused_and_signals_nothing() {
    let plane = plane_with_tree();
    let block = memset_block(DST_VA, 0x100, 0x5A, PB_GPU_VA + FINISH_PAYLOAD_OFFSET, 1);
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();
    ring_once(&plane, &mut vmm, &mut cursor).expect("the first submission runs");
    let irqs_after_first = vmm.irqs.len();

    let err = ring_once(&plane, &mut vmm, &mut cursor).expect_err("no second entry exists");
    // ★★★ Named for WHAT HAPPENED. This asserted `PushTooFragmented` — a bound of ours on
    // how many address-table spans one range may cut into — for a ring that produced no
    // range at all, and `[measured 2026-08-09, boots `uvm2_d0fbac0` / `scan1_00865a7` /
    // `vaspan_994bbdc`]` four consecutive boot logs therefore reported the UVM wall as a
    // fragmentation limit that was never reached. ⊘ "Refuse by name" is a claim that the
    // name is TRUE, not that there is one.
    assert!(
        matches!(
            err.fault,
            FwdFault::RingBroughtNoEntry {
                index: 1,
                entries: 4096,
                ..
            }
        ),
        "the empty ring is named for itself, not served and not mislabelled: {err:?}"
    );
    assert_eq!(
        vmm.irqs.len(),
        irqs_after_first,
        "⊘ no completion interrupt for a doorbell that found no work"
    );
    assert_eq!(cursor.next, 1, "the cursor did not move past a refusal");
}

/// ⊘ **A destination the guest never mapped is a WALK FAULT by name — no bytes, no
/// completion, no interrupt.**
///
/// `mode2_address_table.md`'s MISS = FAULT, arriving from the page tables rather than from
/// a table lookup. ★ The fill's destination is moved out of the mapped 512 MiB leaf while
/// everything else — ring, pushbuffer, semaphore — still resolves, so the refusal is about
/// the operand and not about the submission being unreadable.
#[test]
fn an_unmapped_fill_destination_faults_by_name_and_writes_no_completion() {
    let plane = plane_with_tree();
    // One 512 MiB region up: reachable VA, and nothing in the tree describes it.
    const UNMAPPED: u64 = PB_GPU_VA + (512 << 20);
    let block = memset_block(UNMAPPED, 0x40, 0x11, PB_GPU_VA + FINISH_PAYLOAD_OFFSET, 1);
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();

    let err = ring_once(&plane, &mut vmm, &mut cursor).expect_err("the destination is unmapped");
    assert!(
        matches!(
            err.fault,
            FwdFault::CeWalk {
                va,
                kind: "Fault"
            } if va == kayfabe_arch::ids::GpuVa(UNMAPPED)
        ),
        "the walk's own refusal, naming the VA it failed on: {err:?}"
    );
    assert!(
        err.detail.is_some_and(|(va, r)| va.0 == UNMAPPED
            && matches!(r, kayfabe_device::ceresolve::CeResolve::Fault(_))),
        "★ and the walk's WHOLE finding travels beside the Copy fault, level included: \
         {err:?}"
    );
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        0u32.to_le_bytes(),
        "⊘ the finishPayload is untouched — a completion for a copy that did not happen is \
         the one thing forbidden outright"
    );
    assert!(vmm.irqs.is_empty(), "⊘ and no interrupt");
    assert_eq!(
        cursor.next, 0,
        "⊘ the cursor did not advance through a refusal"
    );
}

/// ⊘ **A channel whose `(hClient, hVASpace)` published no root gets no session at all** —
/// the one refusal that precedes every byte, and it is a fact about the guest.
#[test]
fn a_channel_with_no_published_root_opens_no_session() {
    let plane = plane_with_tree();
    let pb = kayfabe_chips::Ga10xPushbuffer;
    let mut vmm = MockVmm::new();
    let cursor = GpCursor::default();
    let opened = plane.ce_session(
        CLIENT,
        VASPACE + 1, // a VA space this client never published
        kayfabe_device::ceresolve::Demand::from_doorbell(),
        |ce| run_submission(ce, &pb, &mut vmm, channel(), cursor, MethodState::new()),
    );
    assert!(
        opened.is_none(),
        "⊘ no publication, no session — and therefore no walk from a root we guessed"
    );
    assert!(vmm.irqs.is_empty());
}

// =====================================================================================
// ★★★★ THE NULL THAT DISCRIMINATES — a submission that decoded and launched NOTHING
// =====================================================================================

/// A method block that reads, decodes, and contains **no launch and no release** — with an
/// optional leading `SET_OBJECT` so the two nulls (*"no class was declared"* vs *"this class
/// was declared"*) can be told apart by the fixture rather than by argument.
///
/// The non-launch methods are real GA10x CE register writes (`SET_REMAP_CONST_A`,
/// `LINE_LENGTH_IN`) — bytes the codec reads and correctly reports as
/// [`kayfabe_arch::PushMethod::Opaque`], because they latch state and command nothing.
fn no_launch_block(set_object: Option<u32>) -> Vec<u8> {
    let sub = 0u32;
    let hdr = |m, n| submit::method_header_inc(sub, m, n).expect("encodable");
    let mut runs: Vec<(u32, Vec<u32>)> = Vec::new();
    if let Some(class) = set_object {
        runs.push((hdr(submit::SET_OBJECT, 1), vec![class]));
    }
    runs.push((hdr(submit::ce::SET_REMAP_CONST_A, 1), vec![0x5A5A_5A5A]));
    runs.push((hdr(submit::ce::LINE_LENGTH_IN, 1), vec![0x40]));
    let mut out = Vec::new();
    for (h, args) in runs {
        out.extend_from_slice(&h.to_le_bytes());
        for a in args {
            out.extend_from_slice(&a.to_le_bytes());
        }
    }
    out
}

/// ★★★★ **A submission that decoded but launched nothing is named for THAT, and it carries
/// the class the guest's own `SET_OBJECT` declared.**
///
/// `[measured 2026-08-09, boot s19_1dfde1b_cup2]` this refusal was
/// `FwdFault::NotAnEngine(ClassId(0))`, with the `ClassId(0)` written as a **literal at the
/// raise site**. No class was looked up on this path — `route_engine_object` is the only
/// site that resolves one and it is not on the doorbell path — so the boot report named a
/// lookup that never happened and handed its reader a constant to investigate.
///
/// ⊘ **BREAK THE ROUTE, NOT THE ASSERTION.** This test is one half of a pair: here the
/// guest declares `AMPERE_COMPUTE_B` on a channel the CE driver is serving — the only
/// reading under which a class was ever the question — and the refusal must say so. Its
/// sibling declares nothing, and the refusal must say **that** instead. An implementation
/// that hard-codes either answer fails one of the two; the old literal failed both.
#[test]
fn a_submission_that_launches_nothing_reports_the_class_its_set_object_declared() {
    let plane = plane_with_tree();
    let block = no_launch_block(Some(kayfabe_abi::generated::classes::AMPERE_COMPUTE_B));
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();

    let err = ring_once(&plane, &mut vmm, &mut cursor).expect_err("nothing in it can run");
    let FwdFault::SubmissionDecodedNoWork {
        entries,
        index,
        methods,
        opaque,
        set_object,
    } = err.fault
    else {
        panic!("named for what happened, not for an engine lookup that never ran: {err:?}");
    };
    assert_eq!(entries, 1, "the ring brought exactly the one written entry");
    assert_eq!(index, 0, "and it is the entry the cursor was pointing at");
    assert_eq!(
        methods, 3,
        "SET_OBJECT + the two latching writes were all READ"
    );
    assert_eq!(
        opaque, 2,
        "★ two of the three decoded to nothing and ONE did not — so `we decoded nothing` \
         and `we decoded something, just never a launch` stay different findings"
    );
    assert_eq!(
        set_object,
        Some(kayfabe_arch::ids::ClassId(
            kayfabe_abi::generated::classes::AMPERE_COMPUTE_B
        )),
        "★★★★ the class comes out of the guest's OWN method words — the one honest answer \
         on this path to `what engine is this channel driving`"
    );
    assert_eq!(cursor.next, 0, "the cursor did not move past a refusal");
    assert!(
        vmm.irqs.is_empty(),
        "⊘ no completion interrupt for a doorbell that ran nothing"
    );
}

/// ★★★★ **The sibling: no `SET_OBJECT` at all is `None`, and NOT `Some(ClassId(0))`.**
///
/// The two are the same literal under the old refusal and they are different facts: *"the
/// guest declared no engine object in these bytes"* is a statement about the submission,
/// while *"the guest wrote `SET_OBJECT 0`"* is a statement about a value it chose. ⊘ A null
/// that cannot distinguish *never set* from *set to zero* sends every reader to the wrong
/// question — which is exactly what it did.
#[test]
fn a_submission_with_no_set_object_reports_none_and_not_class_zero() {
    let plane = plane_with_tree();
    let block = no_launch_block(None);
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();

    let err = ring_once(&plane, &mut vmm, &mut cursor).expect_err("nothing in it can run");
    let FwdFault::SubmissionDecodedNoWork {
        methods,
        opaque,
        set_object,
        ..
    } = err.fault
    else {
        panic!("named for what happened: {err:?}");
    };
    assert_eq!(methods, 2, "both latching writes were read");
    assert_eq!(opaque, 2, "and neither decoded to a fact");
    assert_eq!(
        set_object, None,
        "★★★★ ABSENCE, reported as absence. `Some(ClassId(0))` here would be the old \
         literal wearing a new type"
    );
}

// =====================================================================================
// ★★★★ THE ACCUMULATOR IS PER-CHANNEL — UVM binds ONCE and fires forever after
// =====================================================================================

/// UVM's shape, not RM's: a copy block that writes its operands and fires, carrying **no
/// `SET_OBJECT`**, on `NVA06F_SUBCHANNEL_COPY_ENGINE = 4`.
///
/// `[measured 2026-08-09, boot s21_dbf853a_cup2]` the refused submission framed to exactly
/// this — `sub4/m0x400/n4` (the operand quad), `sub4/m0x418=0x20` (`LINE_LENGTH_IN`),
/// `sub4/m0x300` (`LAUNCH_DMA`), then a `sub4/m0x240/n3` semaphore triple and a second
/// `LAUNCH_DMA`. UVM binds the class in `channel_init`'s first push and never again.
fn uvm_copy_block(dst_va: u64, bytes: u32, sema_va: u64, payload: u32) -> Vec<u8> {
    let sub = submit::ce::FIXED_SUBCHANNEL as u32;
    let hdr = |m, n| submit::method_header_inc(sub, m, n).expect("encodable");
    let flags = submit::ce::LAUNCH_TRANSFER_NON_PIPELINED
        | submit::ce::LAUNCH_FLUSH_ENABLE
        | submit::ce::LAUNCH_REMAP_ENABLE
        | submit::ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD;
    let runs: Vec<(u32, Vec<u32>)> = vec![
        (
            hdr(submit::ce::SET_REMAP_CONST_A, 1),
            vec![u32::from_le_bytes([0xC3; 4])],
        ),
        (
            hdr(submit::ce::SET_REMAP_COMPONENTS, 1),
            vec![submit::ce::REMAP_DST_SEL_CONST_A],
        ),
        (
            hdr(submit::ce::OFFSET_OUT_UPPER, 2),
            vec![(dst_va >> 32) as u32, dst_va as u32],
        ),
        (hdr(submit::ce::LINE_LENGTH_IN, 1), vec![bytes]),
        (
            hdr(submit::ce::SET_SEMAPHORE_A, 3),
            vec![
                ((sema_va >> 32) as u32) & submit::ce::SET_SEMAPHORE_A_UPPER_MASK,
                sema_va as u32,
                payload,
            ],
        ),
        (hdr(submit::ce::LAUNCH_DMA, 1), vec![flags]),
    ];
    let mut out = Vec::new();
    for (h, args) in runs {
        out.extend_from_slice(&h.to_le_bytes());
        for a in args {
            out.extend_from_slice(&a.to_le_bytes());
        }
    }
    out
}

/// Publish the ring's **second** entry and its pushbuffer.
///
/// ⚠ Called AFTER the first doorbell, never before, and that ordering is the fixture being
/// faithful rather than tidy: `MAX_ENTRIES_PER_DOORBELL` is 8 and the ring loop reads
/// forward while entries decode, so a ring with both entries already written is consumed by
/// **one** doorbell — which is a real behaviour and the wrong scenario. The bench's own
/// sequence is two doorbells (`[measured 2026-08-09, boot s19_1dfde1b_cup2]`: token
/// `0x00010003` SERVED at `14:15:46.427`, REFUSED at `14:15:46.624`), and the guest wrote
/// entry `[1]` between them.
fn publish_second(vmm: &mut MockVmm, second: &[u8]) -> u64 {
    let second_va = PUSH_VA + 0x1000;
    let e1 = submit::gp_entry(second_va, second.len() as u64).expect("representable");
    vmm.gpa_write(phys_of(RING_VA + 8), &e1.to_le_bytes())
        .expect("the ring's second entry");
    vmm.gpa_write(phys_of(second_va), second)
        .expect("the second method block");
    second_va
}

/// ★★★★ **The second doorbell RUNS, because the class the FIRST one bound is still bound.**
///
/// # ⊘ BREAK THE ROUTE, NOT THE ASSERTION
///
/// The mechanism under test is *"the method accumulator survives between doorbells"*, so the
/// acceptance set is both halves: with the accumulator carried the copy runs and the
/// semaphore moves; with it rebuilt per doorbell — `ring_once`, the behaviour that shipped —
/// the **same bytes on the same ring** must decode to nothing. Its sibling below is that
/// negative, and it is what makes this test mean anything at all.
///
/// `[measured 2026-08-09, boot s21_dbf853a_cup2]` the negative is not hypothetical: it is
/// what the bench did. `subchannel_speaks`' unbound arm requires the class to be bound
/// somewhere in the state (`kayfabe-arch/src/lib.rs:1234`), a fresh state has no binding
/// anywhere, so `ce_launch` returned `None` for a `LAUNCH_DMA` that was sitting right there,
/// correctly framed, in the guest's own bytes.
#[test]
fn the_second_doorbell_runs_on_the_class_the_first_one_bound() {
    const LEN: u32 = 0x20; // `LINE_LENGTH_IN=0x20` — the length the bench measured.
    const PAYLOAD: u32 = 7;
    let plane = plane_with_tree();
    // Push 1: RM-shaped, and its ONLY role here is that it carries the `SET_OBJECT`.
    let first = memset_block(DST_VA, 0x40, 0x11, PB_GPU_VA + FINISH_PAYLOAD_OFFSET, 1);
    // Push 2: UVM-shaped — operands, a launch, a semaphore, and NO `SET_OBJECT`.
    let second = uvm_copy_block(
        DST_VA + 0x1000,
        LEN,
        PB_GPU_VA + FINISH_PAYLOAD_OFFSET,
        PAYLOAD,
    );
    let mut vmm = guest_ram(&first);

    let mut cursor = GpCursor::default();
    let mut state = MethodState::new();
    let run1 = ring_once_with(&plane, &mut vmm, &mut cursor, &mut state)
        .expect("the first push binds and runs");
    assert_eq!(run1.launches, 1, "the binding push ran");
    publish_second(&mut vmm, &second);

    let run2 = ring_once_with(&plane, &mut vmm, &mut cursor, &mut state)
        .expect("★★★★ the second push runs on the STILL-BOUND class");
    assert_eq!(run2.launches, 1, "the `LAUNCH_DMA` in its bytes FIRED");
    assert_eq!(
        run2.bytes,
        u64::from(LEN),
        "★ and it moved the bytes `LINE_LENGTH_IN` named"
    );
    assert_eq!(
        vmm.ram_read(phys_of(DST_VA + 0x1000), LEN as usize),
        vec![0xC3; LEN as usize],
        "★ at the resolved physical of the second push's own destination"
    );
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        PAYLOAD.to_le_bytes(),
        "★★★ and the finishPayload carries the SECOND push's payload, after its bytes"
    );
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + SEMA_OFFSET), 4),
        0xDEAD_BEEFu32.to_le_bytes(),
        "⊘ the host semaphore four bytes lower is still untouched"
    );
}

/// ★★★★ **THE NEGATIVE HALF: rebuild the accumulator per doorbell and the very same bytes
/// decode to NOTHING.**
///
/// ⊘ Identical fixture, identical ring, identical pushbuffers — the *only* difference is
/// `ring_once` (a fresh `MethodState` each call) in place of `ring_once_with`. If this ever
/// goes green, the positive above proves nothing: it would mean the second push runs for
/// some reason other than the binding it is supposed to depend on.
///
/// The refusal is asserted **by its fields**, because `SubmissionDecodedNoWork` with
/// `opaque == methods` and `set_object: None` is the exact line the bench printed.
#[test]
fn with_a_per_doorbell_accumulator_the_same_second_push_decodes_to_nothing() {
    let plane = plane_with_tree();
    let first = memset_block(DST_VA, 0x40, 0x11, PB_GPU_VA + FINISH_PAYLOAD_OFFSET, 1);
    let second = uvm_copy_block(DST_VA + 0x1000, 0x20, PB_GPU_VA + FINISH_PAYLOAD_OFFSET, 7);
    let mut vmm = guest_ram(&first);

    let mut cursor = GpCursor::default();
    ring_once(&plane, &mut vmm, &mut cursor).expect("the first push still runs — it binds");
    publish_second(&mut vmm, &second);
    let before = vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4);

    let err = ring_once(&plane, &mut vmm, &mut cursor)
        .expect_err("★ the amnesia is the whole defect: no binding, no launch");
    let FwdFault::SubmissionDecodedNoWork {
        methods,
        opaque,
        set_object,
        index,
        ..
    } = err.fault
    else {
        panic!("the wall the bench hit, reproduced by name: {err:?}");
    };
    assert_eq!(
        index, 1,
        "it is the SECOND ring entry, as the bench reported"
    );
    assert!(methods > 0, "the bytes were read and framed — {methods}");
    assert_eq!(
        opaque, methods,
        "★★★★ and the codec recognized NOT ONE of them, with a `LAUNCH_DMA` sitting in \
         them — `s21_dbf853a_cup2` printed `methods: 7, opaque: 7`"
    );
    assert_eq!(
        set_object, None,
        "because this push declares no class of its own"
    );
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        before,
        "⊘ and nothing was signalled for the work that did not run"
    );
}
