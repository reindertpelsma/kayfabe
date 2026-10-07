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

/// The channel, with the guest's producer cursor `GP_PUT` **stated by the caller**.
///
/// ★★★ w386 — `GP_PUT` is *"one past the last GPFIFO entry the guest wrote"*
/// (`ogkm-580: channel_utils.c:406, :523`), so a fixture that has published entry `[0]`
/// passes `1`, and one that has also published `[1]` passes `2`. ⊘ It is a **parameter** and
/// not derived from the ring's contents on purpose: deriving it would make the fixture agree
/// with the walk by construction, which is precisely the property under test.
fn channel_at(gp_put: u32) -> CeUtilsChannel {
    CeUtilsChannel {
        client: CLIENT,
        vaspace: VASPACE,
        ring_va: RING_VA,
        ring_entries: 4096,
        gp_put: Some(gp_put),
    }
}

/// Drive one doorbell's worth of work through the shell driver.
fn ring_once(
    plane: &RegPlane,
    vmm: &mut MockVmm,
    cursor: &mut GpCursor,
    gp_put: u32,
) -> Result<kayfabe_rt::ceutils::CeUtilsRun, kayfabe_rt::ceutils::CeUtilsRefusal> {
    let mut state = MethodState::new();
    ring_once_with(plane, vmm, cursor, &mut state, gp_put)
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
    gp_put: u32,
) -> Result<kayfabe_rt::ceutils::CeUtilsRun, kayfabe_rt::ceutils::CeUtilsRefusal> {
    // ⊘ Delegates to `ring_once_on` rather than repeating the session + commit dance. The
    // adapter's own discipline — commit the advanced cursor AND the accumulator only on
    // success — lives there, in one copy, so the wrap arms and the acceptance arms cannot
    // drift into testing two different disciplines.
    ring_once_on(plane, vmm, cursor, state, channel_at(gp_put))
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

    let run = ring_once(&plane, &mut vmm, &mut cursor, 1).expect("the submission is servable");

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
    ring_once(&plane, &mut vmm, &mut cursor, 1).expect("the first submission runs");
    let irqs_after_first = vmm.irqs.len();

    let err = ring_once(&plane, &mut vmm, &mut cursor, 1).expect_err("no second entry exists");
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

    let err = ring_once(&plane, &mut vmm, &mut cursor, 1).expect_err("the destination is unmapped");
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
        |ce| run_submission(ce, &pb, &mut vmm, channel_at(1), cursor, MethodState::new()),
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

    let err = ring_once(&plane, &mut vmm, &mut cursor, 1).expect_err("nothing in it can run");
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

    let err = ring_once(&plane, &mut vmm, &mut cursor, 1).expect_err("nothing in it can run");
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
    let run1 = ring_once_with(&plane, &mut vmm, &mut cursor, &mut state, 1)
        .expect("the first push binds and runs");
    assert_eq!(run1.launches, 1, "the binding push ran");
    publish_second(&mut vmm, &second);

    let run2 = ring_once_with(&plane, &mut vmm, &mut cursor, &mut state, 2)
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
    ring_once(&plane, &mut vmm, &mut cursor, 1).expect("the first push still runs — it binds");
    publish_second(&mut vmm, &second);
    let before = vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4);

    let err = ring_once(&plane, &mut vmm, &mut cursor, 2)
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

// =====================================================================================
// ★★★★★ THE RING WRAPS — and the zero-scan stop condition expires exactly when it does
// =====================================================================================
//
// The ring walk in `run_submission` decides *"how many entries are new"* by reading forward
// until an entry decodes to nothing. That is sound for the ring's **first lap only**: RM
// zero-initialises the buffer (`TRANSFER_FLAGS_SHADOW_INIT_MEM`), so an unwritten slot is
// zero and decodes to `None`. Once the guest has written **every** slot once, no zero slot
// remains anywhere in the ring, ever again — the `break` stops firing and the walk runs to
// `MAX_ENTRIES_PER_DOORBELL`, consuming **stale** entries from the previous lap.
//
// ⚠ It bites on the LAST doorbell of the first lap, not after it: at `idx == N-1` the very
// next index is `0`, which lap 1 already wrote. `[measured 2026-08-13, w384, real GA106
// Mode-2 guest, `KAYFABE_CE_EXECUTOR=local`]` a 64-entry ring stalled at
// `first_stall_at=63` and a 32-entry ring at `first_stall_at=31`, both pre-registered
// before the run — the wedge tracks the ring's own modulus, which is this arithmetic.

/// A ring small enough to wrap inside one test. ⊘ Nothing below is 8-specific; 8 is chosen
/// so the whole lap is legible, and the defect is keyed on the modulus, not on its value.
const SMALL_RING: u32 = 8;

/// The same channel the fixture already describes, with a ring that can be walked round.
/// ★ The guest's producer cursor after doorbell `d`: one past the slot it just wrote,
/// modulo the ring. ⊘ Written out rather than folded into `small_channel` so the wrap in the
/// PRODUCER is visible beside the wrap in the consumer.
fn gp_put_after(d: u32) -> u32 {
    (d + 1) % SMALL_RING
}

fn small_channel(gp_put: u32) -> CeUtilsChannel {
    CeUtilsChannel {
        client: CLIENT,
        vaspace: VASPACE,
        ring_va: RING_VA,
        ring_entries: SMALL_RING,
        gp_put: Some(gp_put),
    }
}

/// ⊘ `ring_once_with`'s body, with the channel a parameter. Extracted rather than copied so
/// the wrap tests drive **the same** commit-on-success discipline the acceptance above does.
fn ring_once_on(
    plane: &RegPlane,
    vmm: &mut MockVmm,
    cursor: &mut GpCursor,
    state: &mut MethodState,
    chan: CeUtilsChannel,
) -> Result<kayfabe_rt::ceutils::CeUtilsRun, kayfabe_rt::ceutils::CeUtilsRefusal> {
    let pb = kayfabe_chips::Ga10xPushbuffer;
    let out = plane
        .ce_session(
            CLIENT,
            VASPACE,
            kayfabe_device::ceresolve::Demand::from_doorbell(),
            |ce| run_submission(ce, &pb, vmm, chan, *cursor, *state),
        )
        .expect("the publication is present");
    if let Ok(run) = &out {
        *cursor = run.cursor;
        *state = run.state;
    }
    out
}

/// Doorbell `d`'s pushbuffer, destination, fill byte and payload — **all four distinct per
/// doorbell**, so re-running a stale entry is distinguishable from running the new one by
/// the bytes it moved *and* by the payload it released, not by a count alone.
fn slot_push_va(d: u32) -> u64 {
    PUSH_VA + u64::from(d % SMALL_RING) * 0x1000
}
fn doorbell_dst_va(d: u32) -> u64 {
    DST_VA + u64::from(d) * 0x1000
}
fn doorbell_fill(d: u32) -> u8 {
    0x40u8.wrapping_add(d as u8)
}
/// RM's `finishPayload` counts submissions and never repeats — `putIndex` monotonically
/// increasing (`ogkm-580: channel_utils.c:406`). So payload `d + 1` is what the guest is
/// polling for after doorbell `d`, and a *lower* value appearing there is a stale release.
fn doorbell_payload(d: u32) -> u32 {
    d + 1
}

/// The guest's producer step: write doorbell `d`'s method block, then the GPFIFO entry in
/// slot `d % SMALL_RING` that names it — overwriting whatever the previous lap left there.
fn publish_doorbell(vmm: &mut MockVmm, d: u32) {
    let block = memset_block(
        doorbell_dst_va(d),
        0x40,
        doorbell_fill(d),
        PB_GPU_VA + FINISH_PAYLOAD_OFFSET,
        doorbell_payload(d),
    );
    let va = slot_push_va(d);
    vmm.gpa_write(phys_of(va), &block)
        .expect("this doorbell's method block");
    let e = submit::gp_entry(va, block.len() as u64).expect("representable");
    vmm.gpa_write(
        phys_of(RING_VA + 8 * u64::from(d % SMALL_RING)),
        &e.to_le_bytes(),
    )
    .expect("this doorbell's ring entry");
}

/// Guest RAM with the two semaphore words in place and a ring that is **entirely zero** —
/// the state RM leaves behind (`TRANSFER_FLAGS_SHADOW_INIT_MEM`).
fn guest_ram_zero_ring() -> MockVmm {
    let mut vmm = MockVmm::new();
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

/// ★★★ **THE KNOWN-POSITIVE CONTROL: lap one stops at the first unwritten entry.**
///
/// ⊘ This is the case the zero-scan was written for and the case that works today. It must
/// pass **before and after** any change to the stop condition — a fix that breaks it has
/// traded one wedge for another, on the path `cup3`'s `CUP3_VAL=43` and the eight GR
/// completions actually run over.
#[test]
fn lap_one_stops_at_the_first_unwritten_entry() {
    let plane = plane_with_tree();
    let mut vmm = guest_ram_zero_ring();
    publish_doorbell(&mut vmm, 0);
    let mut cursor = GpCursor::default();
    let mut state = MethodState::new();

    let run = ring_once_on(
        &plane,
        &mut vmm,
        &mut cursor,
        &mut state,
        small_channel(gp_put_after(0)),
    )
    .expect("the one written entry is servable");

    assert_eq!(
        run.entries, 1,
        "the ring is zero past slot 0, so exactly one entry is new: {run:?}"
    );
    assert_eq!(run.launches, 1, "and exactly one launch ran: {run:?}");
    assert_eq!(cursor.next, 1, "the cursor advanced past what it ran");
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        doorbell_payload(0).to_le_bytes(),
        "the finishPayload carries THIS doorbell's payload"
    );
}

/// ★★★★★ **THE RED ROW: every doorbell consumes exactly the one entry the guest wrote —
/// across the wrap.**
///
/// Two full laps, one entry written per doorbell, which is what `channelFillGpFifo` does
/// (`[src] ogkm-580: channel_utils.c:403-443` — RM submits **one** block per call). The
/// assertion is per-doorbell and names its own ordinal, so the failure states *which*
/// doorbell diverged rather than that the run ended wrong.
///
/// ⊘ Three separate assertions, because `entries` alone cannot tell the two consequences
/// apart:
/// 1. `entries == 1` — the cursor stays in step with the guest's producer.
/// 2. the destination bytes are **this** doorbell's fill — a stale `LAUNCH_DMA` re-run
///    would write a previous lap's value into a previous lap's page.
/// 3. the finishPayload holds **this** doorbell's payload — a stale release puts a payload
///    the guest already passed back over the word it is polling, which is the safety half:
///    a completion is only honest if the state after it is the state the guest intended.
#[test]
fn every_doorbell_of_two_full_laps_consumes_exactly_the_entry_the_guest_wrote() {
    let plane = plane_with_tree();
    let mut vmm = guest_ram_zero_ring();
    let mut cursor = GpCursor::default();
    let mut state = MethodState::new();

    for d in 0..(2 * SMALL_RING) {
        publish_doorbell(&mut vmm, d);
        let run = ring_once_on(
            &plane,
            &mut vmm,
            &mut cursor,
            &mut state,
            small_channel(gp_put_after(d)),
        )
        .unwrap_or_else(|e| panic!("doorbell {d} (slot {}) refused: {e:?}", d % SMALL_RING));

        assert_eq!(
            run.entries,
            1,
            "★★★★★ doorbell {d} (slot {}) consumed {} entries; the guest wrote ONE. \
             Past the wrap no slot is zero any more, so a walk that stops at a zero entry \
             never stops: {run:?}",
            d % SMALL_RING,
            run.entries
        );
        assert_eq!(
            run.launches, 1,
            "doorbell {d}: one entry is one LAUNCH_DMA: {run:?}"
        );
        assert_eq!(
            vmm.ram_read(phys_of(doorbell_dst_va(d)), 0x40),
            vec![doorbell_fill(d); 0x40],
            "doorbell {d}: its own destination carries its own fill"
        );
        assert_eq!(
            vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
            doorbell_payload(d).to_le_bytes(),
            "★★★ doorbell {d}: the word the guest polls holds THIS doorbell's payload. A \
             stale re-release puts an OLD payload here — a completion for work that did \
             not happen on this doorbell"
        );
        assert_eq!(
            cursor.next,
            (d + 1) % SMALL_RING,
            "doorbell {d}: the cursor is one past the entry it ran, modulo the ring"
        );
        assert_eq!(
            vmm.ram_read(phys_of(PB_GPU_VA + SEMA_OFFSET), 4),
            0xDEAD_BEEFu32.to_le_bytes(),
            "⊘ doorbell {d}: the host semaphore four bytes lower stays untouched"
        );
    }
}

/// ★★★★★ **THE NON-VACUITY ROW: a walk that consumes SEVERAL entries ACROSS the wrap.**
///
/// ⊘ **BREAK THE ROUTE, NOT THE ASSERTION.** Every arm above asserts `entries == 1`, and a
/// "fix" that simply clamped the walk to one entry per doorbell would pass all of them while
/// silently dropping every batched submission. This one is the falsifier: the guest writes
/// **four** entries — slots 6, 7, 0, 1 — and rings **once**. The consumer starts at 6 and the
/// producer is at 2, so the honest answer spans the wrap and is four.
///
/// ★ This is also the shape the two-segment walk exists for. `[6, 7]` then `[0, 1]` is one
/// modular loop here rather than two written-out segments, and this row is what says the
/// modular form is right.
#[test]
fn a_batch_that_straddles_the_wrap_is_consumed_whole() {
    let plane = plane_with_tree();
    let mut vmm = guest_ram_zero_ring();
    let mut cursor = GpCursor::default();
    let mut state = MethodState::new();

    // Bring the ring to a state in which every slot has been written once and the consumer
    // sits at 6 — i.e. past the wrap, where the old zero-terminator is already useless.
    for d in 0..6 {
        publish_doorbell(&mut vmm, d);
        ring_once_on(
            &plane,
            &mut vmm,
            &mut cursor,
            &mut state,
            small_channel(gp_put_after(d)),
        )
        .unwrap_or_else(|e| panic!("warm-up doorbell {d}: {e:?}"));
    }
    assert_eq!(cursor.next, 6, "the consumer is at slot 6");

    // The guest now writes FOUR entries — slots 6, 7, 0, 1 — before ringing once.
    for d in 6..10 {
        publish_doorbell(&mut vmm, d);
    }
    let run = ring_once_on(&plane, &mut vmm, &mut cursor, &mut state, small_channel(2))
        .expect("four new entries are servable");

    assert_eq!(
        run.entries, 4,
        "★★★★★ the walk spans the wrap: slots 6, 7, 0, 1 — a clamp-to-one 'fix' reports 1 \
         here and drops three submissions: {run:?}"
    );
    assert_eq!(run.launches, 4, "and every one of them ran: {run:?}");
    assert_eq!(cursor.next, 2, "the cursor lands exactly on the producer");
    for d in 6..10 {
        assert_eq!(
            vmm.ram_read(phys_of(doorbell_dst_va(d)), 0x40),
            vec![doorbell_fill(d); 0x40],
            "doorbell {d}'s own destination carries its own fill"
        );
    }
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        doorbell_payload(9).to_le_bytes(),
        "★ and the LAST entry's payload is the one left on the polled word — the releases \
         ran in ring order, not hoisted"
    );
}

/// ★★★★★ **THE REFUSAL, BY NAME: no producer cursor ⇒ no walk.**
///
/// ⊘ This is the arm that makes the fix a *decision* rather than a silent preference. When
/// the channel's `USERD GP_PUT` cannot be read — an unreadable descriptor, an `Undeclared`
/// one, or a **`Sysmem`** one whose base is a guest-physical address the framebuffer store
/// must not be asked about — there is no honest answer to *"how many entries are new"*, and
/// the walk says so instead of falling back to the ring's zero-terminator.
///
/// ⚠ **The fallback is what is being refused, and it is not a mild one.** Past the ring's
/// first lap the zero-terminator never fires, so falling back would re-execute retired
/// `LAUNCH_DMA`s and re-release their payloads over the word the guest polls — a completion
/// for work that did not happen on this doorbell.
///
/// ★ `[measured, committed boot logs w267 (8 CE channels) / w269 / w274b / w279 / w281 /
/// w283 / w287]` every copy-engine channel that reaches this walk declared its USERD in the
/// framebuffer (`phys=fb:0x…/0x200`) and its cursors read live (`fbuserd@0x… GET=0 PUT=1`,
/// and `GET=1 PUT=1` once the engine had fetched). So this refusal is not expected to fire on
/// any channel that works today — and if it does, the boot names it rather than guessing.
#[test]
fn a_channel_with_no_producer_cursor_is_refused_by_name_and_signals_nothing() {
    let plane = plane_with_tree();
    let mut vmm = guest_ram_zero_ring();
    publish_doorbell(&mut vmm, 0);
    let mut cursor = GpCursor::default();
    let mut state = MethodState::new();
    let blind = CeUtilsChannel {
        gp_put: None,
        ..small_channel(1)
    };

    let err = ring_once_on(&plane, &mut vmm, &mut cursor, &mut state, blind)
        .expect_err("without GP_PUT there is no honest count of new entries");

    assert!(
        matches!(
            err.fault,
            FwdFault::RingProducerCursorUnknown {
                index: 0,
                entries: SMALL_RING,
                ..
            }
        ),
        "⊘ named for the thing that is missing — not RingBroughtNoEntry, which would say \
         the guest wrote nothing when it wrote an entry we can see: {err:?}"
    );
    assert_eq!(
        cursor.next, 0,
        "⊘ the cursor did not advance through a refusal"
    );
    assert!(vmm.irqs.is_empty(), "⊘ and nothing was signalled");
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        0u32.to_le_bytes(),
        "⊘ and the polled word is untouched"
    );
}

/// ★★★★ **A producer cursor outside the ring is ITS OWN refusal, and not the one above.**
///
/// ⊘ *"We could not read it"* and *"we read it and it is not an index into this ring"* are
/// different diagnoses pointing at different files: the first is about the USERD descriptor,
/// the second says one of `GP_PUT`'s source and `gpFifoEntries`' source is describing a
/// different channel — a join defect on our side. Collapsing them would send a reader to the
/// wrong one, which is this tree's most-paid-for error class.
///
/// ⚠ And it is refused rather than wrapped: `gp_put % entries` would turn a disagreement
/// between two of our own projections into a perfectly plausible index.
#[test]
fn a_producer_cursor_outside_the_ring_is_refused_as_its_own_fact() {
    let plane = plane_with_tree();
    let mut vmm = guest_ram_zero_ring();
    publish_doorbell(&mut vmm, 0);
    let mut cursor = GpCursor::default();
    let mut state = MethodState::new();

    let err = ring_once_on(
        &plane,
        &mut vmm,
        &mut cursor,
        &mut state,
        small_channel(SMALL_RING + 3),
    )
    .expect_err("11 is not an index into an 8-entry ring");

    assert!(
        matches!(
            err.fault,
            FwdFault::RingProducerCursorOutOfRange {
                gp_put: 11,
                entries: SMALL_RING,
                ..
            }
        ),
        "⊘ the two numbers that disagree are BOTH carried, so the report names the join \
         rather than an index: {err:?}"
    );
    assert_eq!(cursor.next, 0, "⊘ nothing advanced");
    assert!(vmm.irqs.is_empty());
}

// =====================================================================================
// ★★★★★ w740 — `CeUtilsRefusal::progress`: THE ONLY THING THAT MAKES A RE-RUN LEGAL
// =====================================================================================
//
// `SINGLE_STORE_PLAN.md`'s w735 table named the gap and its consequence: the CPU CE executor
// *"holds `mem` for the whole closure … a store refusal comes back as `refused(..)` — the
// submission is **dropped, not requeued**. A transient 'not armed yet' becomes a dropped
// kernel CeUtils scrub."* w740's doorbell path fixes that by **running the submission
// again** after a lock-free drain has armed the page.
//
// ⊘⊘⊘ That is only sound for a refusal that did NOTHING. A submission that already executed
// a `LAUNCH_DMA` and released its payload would, on a re-run, move the bytes a second time
// and **release the guest's declared payload a second time** — the stale-release defect
// `[measured w386]` already cost this campaign an 8-entry ring that consumed all 8 and wrote
// 8 completions.
//
// ⚠ These two tests are the whole safety argument, and neither can be checked by booting:
// the shim's retry gate is `progress.nothing_happened()`, and a boot where nothing ever
// refused mid-submission would pass with the gate wired backwards.

/// A method block whose destination is **unmapped** — the second half of the two-entry
/// fixture below. ⊘ Same `memset_block` the acceptance uses; only the VA differs, so the
/// refusal under test is the walk's and not a decode artefact.
const UNMAPPED_DST: u64 = PB_GPU_VA + (512 << 20);

/// Publish doorbell `d`'s ring entry naming a block that fills an **unmapped** destination.
fn publish_unmapped_doorbell(vmm: &mut MockVmm, d: u32) {
    let block = memset_block(
        UNMAPPED_DST,
        0x40,
        doorbell_fill(d),
        PB_GPU_VA + FINISH_PAYLOAD_OFFSET,
        doorbell_payload(d),
    );
    let va = slot_push_va(d);
    vmm.gpa_write(phys_of(va), &block)
        .expect("this doorbell's method block");
    let e = submit::gp_entry(va, block.len() as u64).expect("representable");
    vmm.gpa_write(
        phys_of(RING_VA + 8 * u64::from(d % SMALL_RING)),
        &e.to_le_bytes(),
    )
    .expect("this doorbell's ring entry");
}

/// ★★★ **THE RE-RUNNABLE CASE** — refused before a single launch, so every counter is zero
/// and the shim's gate opens.
#[test]
fn a_refusal_before_any_launch_reports_no_progress_and_is_therefore_re_runnable() {
    let plane = plane_with_tree();
    let block = memset_block(
        UNMAPPED_DST,
        0x40,
        0x11,
        PB_GPU_VA + FINISH_PAYLOAD_OFFSET,
        1,
    );
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();

    let err = ring_once(&plane, &mut vmm, &mut cursor, 1).expect_err("the destination is unmapped");
    // ★★★★★ **THE DEFECT THIS TEST CAUGHT, kept as the assertion that caught it.**
    // `launches` is bumped at DECODE, so it is `1` here even though the walk refused before
    // a byte moved. My first gate was `progress == NONE`, which uses it — and would have
    // been **closed on every real case**, making the whole w740 retry inert on hardware
    // while the boot failed identically.
    assert_eq!(
        err.progress.launches, 1,
        "★ one launch DECODED — the counter that must NOT be in the gate: {err:?}"
    );
    assert_eq!(err.progress.bytes, 0, "⊘ and not one byte moved: {err:?}");
    assert_eq!(
        err.progress.completions, 0,
        "⊘ and no payload released: {err:?}"
    );
    assert!(
        err.progress.may_re_run(),
        "★★★ ⇒ the shim's re-run gate is OPEN, which is the whole point of w740"
    );
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        0u32.to_le_bytes(),
        "⊘ corroborated from the guest's side: no payload was released"
    );
}

/// ★★★★★ **THE CASE THAT MUST NEVER BE RE-RUN** — entry `[0]` executed and released its
/// payload, entry `[1]` then refused. A re-run would fill `[0]`'s destination again and
/// release payload 1 a second time.
///
/// ⊘ The assertion is on `progress`, **not** on the shim: this pins the fact the shim's gate
/// reads. `[the shim side]` `crate::shim`'s `CE-SUBMIT-ARM … blocked_by_progress=` counts
/// every time this shape arrives there.
#[test]
fn a_refusal_after_a_launch_executed_reports_progress_and_must_never_be_re_run() {
    let plane = plane_with_tree();
    let mut vmm = guest_ram_zero_ring();
    publish_doorbell(&mut vmm, 0); // entry [0]: a real fill, to a mapped destination
    publish_unmapped_doorbell(&mut vmm, 1); // entry [1]: the walk fails here
    let mut cursor = GpCursor::default();
    let mut state = MethodState::new();

    // `GP_PUT = 2` ⇒ the guest published both, so one doorbell must walk both.
    let err = ring_once_on(&plane, &mut vmm, &mut cursor, &mut state, small_channel(2))
        .expect_err("the SECOND entry's destination is unmapped");

    assert_eq!(
        err.progress.launches, 2,
        "two launches DECODED — entry [0]'s, which ran, and entry [1]'s, which refused on \
         its walk: {err:?}"
    );
    assert_eq!(
        err.progress.bytes, 0x40,
        "★ and it moved entry [0]'s 0x40 bytes — the count is the work, not a flag"
    );
    assert_eq!(
        err.progress.completions, 1,
        "★★★★★ AND IT RELEASED A PAYLOAD. This is the number that forbids a re-run: the \
         guest has already been told submission 1 finished"
    );
    assert!(
        !err.progress.may_re_run(),
        "⊘⊘⊘ the shim's re-run gate MUST be closed here"
    );
    // ⊘ Corroborated from the guest's own memory rather than from our counters alone.
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        doorbell_payload(0).to_le_bytes(),
        "entry [0]'s payload really is in the guest's finishPayload word"
    );
    assert_eq!(
        cursor.next, 0,
        "⊘ and the cursor still did not advance — which is exactly why a re-run would \
         re-execute entry [0]"
    );
}

// =====================================================================================
// ★★★★★ w743 — THE PRE-FLIGHT: **NOTHING MOVES UNTIL EVERY VIEW IS ARMED**
//
// `[measured w740]` `blocked_by_progress=68` against `trips=10`; `[measured w742]` `3`
// against `37`, with `gave_up=1`. Those are submissions refused **after** they had already
// moved bytes and released a payload, which `CeProgress::may_re_run` correctly — and
// permanently — refuses to re-run. The arms below are the two halves of the fix:
//
//   * the refusal now arrives with `progress == NONE`, so the shim's drain-and-retry opens;
//   * and it arrives with the demand for **every** page the submission needs already in the
//     want set, so **one** drain arms the whole submission instead of one page per trip.
//
// ⊘ The store here is `DeviceFb` over a byte port, because the pre-flight is inert without
// one — `FbStore::demand_port` is `None` on `SparseFb` and on the `arena` arm, and
// `the_preflight_is_inert_without_a_byte_port` is the control that pins that.
// =====================================================================================

/// A second 512 MiB region of the same VA space, mapped to **video memory** — PD1 index 34
/// where `PB_GPU_VA`'s is 33. ⊘ The fixture above maps everything to guest RAM, where the
/// pre-flight has nothing to ask about; a CE destination in the framebuffer is the shape the
/// raw client actually has (`kayfabe-isolate-host`'s mean path allocates every buffer with
/// `NV01_MEMORY_LOCAL_USER` + `ATTR_CONTIGUOUS_VIDMEM`).
/// ⊘ `PB_GPU_VA + 1 GiB`, and **not** `+ 512 MiB`: that slot is [`UNMAPPED_DST`], and the
/// first draft of this fixture mapped it — which made
/// `the_preflight_leaves_a_walk_refusal_to_the_loop_that_names_it` fail by turning its
/// deliberately unmapped address into a perfectly good framebuffer one. ★ The test caught
/// the fixture, which is the only reason that control is worth having.
const FBDST_VA: u64 = PB_GPU_VA + 0x4000_0000;
/// Where that region lands in the reserved object. ⊘ Far from the page-table pages at
/// `0x10_0000`–`0x12_0000`, so arming a destination cannot accidentally arm a directory.
const FBDST_PHYS: u64 = 0x0400_0000;

/// Physical of a `FBDST_VA`-region VA, in the framebuffer.
const fn fb_phys_of(va: u64) -> u64 {
    FBDST_PHYS + (va - FBDST_VA)
}

/// A PD1 slot that is a 512 MiB page in **video** memory. ⊘ Aperture `0` in the PTE table is
/// video (`kern_gmmu_fmt_gm10x.c:184-201`) — not the `1` a PDE uses, which is the confusion
/// [`leaf_512m_sysmem`]'s own comment names.
fn leaf_512m_vidmem(phys: u64) -> u64 {
    ((phys >> 12) << 8) | 1
}

/// The grain this test port arms at.
const TP_GRAIN: u64 = 0x1000;

/// A byte port over a sparse map of grains — the offline stand-in for
/// `kayfabe_qemu_raw::deviceview::DeviceFbBytePort`.
///
/// ⊘ It keeps the three counters apart that the real port keeps apart, for the real port's
/// reason: a demand raised by an **attempted access** and one raised by a **question** have
/// opposite consequences for a retry, and w743's whole claim is about which came first.
#[derive(Debug, Default)]
struct TestPort {
    bytes: std::sync::Mutex<std::collections::BTreeMap<u64, Vec<u8>>>,
    armed: std::sync::Mutex<std::collections::BTreeSet<u64>>,
    want: std::sync::Mutex<std::collections::BTreeSet<u64>>,
    probes: std::sync::atomic::AtomicU64,
    probe_missed: std::sync::atomic::AtomicU64,
    accesses_refused: std::sync::atomic::AtomicU64,
}

impl TestPort {
    fn arm(&self, at: u64) {
        self.armed.lock().unwrap().insert(at & !(TP_GRAIN - 1));
    }
    /// Read the object past every armed-run check — the oracle a test compares against.
    fn peek(&self, at: u64, len: usize) -> Vec<u8> {
        let base = at & !(TP_GRAIN - 1);
        let off = (at - base) as usize;
        match self.bytes.lock().unwrap().get(&base) {
            Some(p) => p[off..off + len].to_vec(),
            None => vec![0u8; len],
        }
    }
    fn wanted(&self) -> std::collections::BTreeSet<u64> {
        self.want.lock().unwrap().clone()
    }
    /// One drain: arm everything wanted, exactly as `DeviceFbBytePort::drain` does.
    fn drain_all(&self) -> usize {
        let batch: Vec<u64> = self.want.lock().unwrap().iter().copied().collect();
        self.want.lock().unwrap().clear();
        let mut a = self.armed.lock().unwrap();
        for b in &batch {
            a.insert(*b);
        }
        batch.len()
    }
    fn span(&self, at: u64, len: u64) -> Option<(u64, u64)> {
        let end = at.checked_add(len)?;
        if len == 0 {
            return None;
        }
        Some((at & !(TP_GRAIN - 1), (end - 1) & !(TP_GRAIN - 1)))
    }
    fn all_armed(&self, first: u64, last: u64) -> bool {
        let a = self.armed.lock().unwrap();
        let mut b = first;
        while b <= last {
            if !a.contains(&b) {
                return false;
            }
            b += TP_GRAIN;
        }
        true
    }
    fn note_want(&self, first: u64, last: u64) {
        let mut w = self.want.lock().unwrap();
        let mut b = first;
        while b <= last {
            w.insert(b);
            b += TP_GRAIN;
        }
    }
}

impl kayfabe_device::DeviceFbPort for TestPort {
    fn read_armed(&self, at: u64, buf: &mut [u8]) -> bool {
        let Some((first, last)) = self.span(at, buf.len() as u64) else {
            return false;
        };
        if !self.all_armed(first, last) {
            self.accesses_refused
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        let mut done = 0usize;
        let mut base = first;
        while base <= last {
            let start = at.max(base);
            let stop = (at + buf.len() as u64).min(base + TP_GRAIN);
            let n = (stop - start) as usize;
            buf[done..done + n].copy_from_slice(&self.peek(start, n));
            done += n;
            base += TP_GRAIN;
        }
        true
    }

    fn write_armed(&self, at: u64, bytes: &[u8]) -> bool {
        let Some((first, last)) = self.span(at, bytes.len() as u64) else {
            return false;
        };
        if !self.all_armed(first, last) {
            self.accesses_refused
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        let mut done = 0usize;
        let mut base = first;
        let mut map = self.bytes.lock().unwrap();
        while base <= last {
            let start = at.max(base);
            let stop = (at + bytes.len() as u64).min(base + TP_GRAIN);
            let n = (stop - start) as usize;
            let page = map
                .entry(base)
                .or_insert_with(|| vec![0u8; TP_GRAIN as usize]);
            let off = (start - base) as usize;
            page[off..off + n].copy_from_slice(&bytes[done..done + n]);
            done += n;
            base += TP_GRAIN;
        }
        true
    }

    fn want(&self, at: u64, len: u64, _by: kayfabe_device::DeviceFbWant) {
        if let Some((first, last)) = self.span(at, len.max(1)) {
            self.note_want(first, last);
        }
    }

    fn probe_or_want(&self, at: u64, len: u64, by: kayfabe_device::DeviceFbWant) -> bool {
        self.probes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Some((first, last)) = self.span(at, len.max(1)) else {
            return false;
        };
        if self.all_armed(first, last) {
            return true;
        }
        self.probe_missed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        kayfabe_device::DeviceFbPort::want(self, at, len, by);
        false
    }

    fn drain(&self) -> kayfabe_device::DeviceFbDrained {
        // ⊘ The offline port never drains itself: `drain_all` is the test's own explicit
        // step, so "a drain happened" is never something the fixture did behind the arm.
        kayfabe_device::DeviceFbDrained::default()
    }

    fn census_line(&self) -> String {
        format!("TESTPORT armed={}", self.armed.lock().unwrap().len())
    }
}

/// The same publication and page tables as [`plane_with_tree`], over a **`DeviceFb` with a
/// byte port**, plus the extra vidmem region [`FBDST_VA`] maps.
///
/// ⊘ The three directory pages are armed by the fixture before it writes them: a guest's
/// page tables are already in video memory before the first doorbell, and *"the fixture
/// could not write the tree"* is not the thing under test.
fn device_plane_with_tree(port: &std::sync::Arc<TestPort>) -> RegPlane {
    let plane = RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        kayfabe_device::abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");
    for p in [ROOT_FB, PD2_FB, PD1_FB] {
        port.arm(p);
    }
    plane.set_fb(Box::new(kayfabe_device::DeviceFb::with_port(
        kayfabe_device::ga10x::GA106.fb_length,
        port.clone() as std::sync::Arc<dyn kayfabe_device::DeviceFbPort>,
    )));
    plane.set_mmu(Box::new(kayfabe_chips::Ga10xGmmu::new()));

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

    let pd3 = (PB_GPU_VA >> 47) as usize;
    let pd2 = ((PB_GPU_VA >> 38) & 511) as usize;
    let pd1 = ((PB_GPU_VA >> 29) & 511) as usize;
    let pd1_fb = ((FBDST_VA >> 29) & 511) as usize;
    assert_ne!(pd1, pd1_fb, "the two regions must be different PD1 slots");
    plane
        .ce_session(
            CLIENT,
            VASPACE,
            kayfabe_device::ceresolve::Demand::from_doorbell(),
            |ce| {
                let put = |fb: &mut dyn FbStore, at: u64, e: u64| {
                    fb.write(at, &e.to_le_bytes())
                        .expect("the fixture armed this directory page");
                };
                put(ce.fb(), ROOT_FB + 8 * pd3 as u64, pde_vid(PD2_FB));
                put(ce.fb(), PD2_FB + 8 * pd2 as u64, pde_vid(PD1_FB));
                put(
                    ce.fb(),
                    PD1_FB + 8 * pd1 as u64,
                    leaf_512m_sysmem(LEAF_PHYS),
                );
                put(
                    ce.fb(),
                    PD1_FB + 8 * pd1_fb as u64,
                    leaf_512m_vidmem(FBDST_PHYS),
                );
            },
        )
        .expect("the publication is present, so a session opens");
    plane
}

/// Doorbell `d`'s destination, in the **framebuffer** region.
fn fb_dst_va(d: u32) -> u64 {
    FBDST_VA + u64::from(d) * 0x1000
}

/// Publish doorbell `d` with a framebuffer destination. ⊘ The ring, the method block and the
/// semaphore stay in guest RAM — exactly one variable changes from [`publish_doorbell`].
fn publish_fb_doorbell(vmm: &mut MockVmm, d: u32) {
    let block = memset_block(
        fb_dst_va(d),
        0x40,
        doorbell_fill(d),
        PB_GPU_VA + FINISH_PAYLOAD_OFFSET,
        doorbell_payload(d),
    );
    let va = slot_push_va(d);
    vmm.gpa_write(phys_of(va), &block)
        .expect("this doorbell's method block");
    let e = submit::gp_entry(va, block.len() as u64).expect("representable");
    vmm.gpa_write(
        phys_of(RING_VA + 8 * u64::from(d % SMALL_RING)),
        &e.to_le_bytes(),
    )
    .expect("this doorbell's ring entry");
}

/// ★★★ **THE CONTROL — the pre-flight is INERT where there is no byte port.**
///
/// ⊘ This is the `arena` arm's property, and it is what makes that arm a control at all: the
/// store answers `FbStore::demand_port() == None`, `want_ours_spans` returns without
/// resolving anything, and the submission runs byte-identically to pre-w743. A test that
/// only checked the device arm could not tell "the pre-flight is correct" from "the
/// pre-flight is running everywhere and happens not to refuse".
#[test]
fn the_preflight_is_inert_without_a_byte_port() {
    const LEN: u32 = 0x400;
    let plane = plane_with_tree();
    let block = memset_block(DST_VA, LEN, 0xAB, PB_GPU_VA + FINISH_PAYLOAD_OFFSET, 0x1234);
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();
    let run = ring_once(&plane, &mut vmm, &mut cursor, 1).expect("the fill is servable");
    assert_eq!(
        run.preflight,
        kayfabe_rt::cpu_ce::CePreflight::default(),
        "⊘ NOT ONE PROBE on a store with no byte port — `probed=0` here is *not asked*, \
         never *nothing was missing*: {:?}",
        run.preflight
    );
    assert_eq!(run.bytes, u64::from(LEN), "and the copy still ran");
    let (passes, ..) = kayfabe_rt::ceutils::preflight_census();
    let _ = passes; // ⊘ process-wide and shared with every other test in this binary.
}

/// ★★★★★ **THE FIX, HALF ONE — a refusal that moved NOTHING, so the retry gate is OPEN.**
///
/// The destination is a framebuffer page with no armed CPU view. `[measured w740/w742]` this
/// is what `FwdFault::CpuCeFb` on the user channels is.
#[test]
fn a_preflight_refusal_moves_nothing_and_is_therefore_re_runnable() {
    let port = std::sync::Arc::new(TestPort::default());
    let plane = device_plane_with_tree(&port);
    let block = memset_block(
        fb_dst_va(0),
        0x40,
        0x5A,
        PB_GPU_VA + FINISH_PAYLOAD_OFFSET,
        7,
    );
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();

    let err = ring_once(&plane, &mut vmm, &mut cursor, 1)
        .expect_err("no CPU view of the destination is armed");

    assert!(
        matches!(err.fault, FwdFault::CpuCeFb { .. }),
        "the refusal keeps the name the boot census counts: {err:?}"
    );
    // ★★★★★ **ZERO LAUNCHES — and that is the whole difference.** The walk-refusal case one
    // screen up reports `launches: 1`, because the launch had already DECODED when its walk
    // failed. Here the refusal is raised before the execute loop is entered at all.
    assert_eq!(
        err.progress,
        kayfabe_rt::ceutils::CeProgress::NONE,
        "⊘ nothing decoded-and-executed, no byte moved, no payload released: {err:?}"
    );
    assert!(
        err.progress.may_re_run(),
        "★★★ ⇒ the shim's re-run gate is OPEN. `[measured w742]` this shape arrived at that \
         gate CLOSED (`blocked_by_progress=3`), which no number of retries can repair"
    );
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        0u32.to_le_bytes(),
        "⊘ corroborated from the guest's side: no payload was released"
    );
    assert_eq!(
        port.peek(fb_phys_of(fb_dst_va(0)), 4),
        vec![0u8; 4],
        "⊘ and from the object's side: not one byte of the destination changed"
    );
    assert_eq!(
        cursor.next, 0,
        "⊘ and the cursor did not advance, so the re-run sees the same entry"
    );
}

/// ★★★★★ **THE FIX, HALF TWO, AND IT IS THE ONE w742 COULD NOT BUY.**
///
/// Entry `[0]`'s destination is armed and entry `[1]`'s is not. `[measured w742]`
/// `blocked_by_progress=3` **is this shape**: pre-w743 entry `[0]` fills its page and
/// releases payload 1, entry `[1]` then refuses, and `CeProgress::may_re_run` closes for
/// ever — correctly, because a re-run would release payload 1 a second time.
///
/// ⊘ **This is the known-positive for the whole increment.** Delete the `3b` pre-flight block
/// in `kayfabe_rt::ceutils::run_submission_body` and this test fails with
/// `completions: 1, bytes: 64` — which is exactly the boot number it was built from.
#[test]
fn a_later_entrys_unarmed_destination_no_longer_costs_the_earlier_entrys_payload() {
    let port = std::sync::Arc::new(TestPort::default());
    let plane = device_plane_with_tree(&port);
    let mut vmm = guest_ram_zero_ring();
    publish_fb_doorbell(&mut vmm, 0);
    publish_fb_doorbell(&mut vmm, 1);
    // Entry [0]'s destination IS armed; entry [1]'s is not. One variable.
    port.arm(fb_phys_of(fb_dst_va(0)));
    let mut cursor = GpCursor::default();
    let mut state = MethodState::new();

    let err = ring_once_on(&plane, &mut vmm, &mut cursor, &mut state, small_channel(2))
        .expect_err("entry [1]'s destination has no armed view");

    assert_eq!(
        err.progress.bytes, 0,
        "★★★★★ entry [0]'s 64 bytes did NOT move. Pre-w743 this was `0x40`: {err:?}"
    );
    assert_eq!(
        err.progress.completions, 0,
        "★★★★★ AND NO PAYLOAD WAS RELEASED. Pre-w743 this was `1` — the number that \
         forbids the re-run for ever: {err:?}"
    );
    assert!(
        err.progress.may_re_run(),
        "⇒ the gate is open where `[measured w740]` `blocked_by_progress=68` had it shut"
    );
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        0u32.to_le_bytes(),
        "⊘ corroborated from the guest's own memory, not from our counters alone"
    );
    assert_eq!(
        port.peek(fb_phys_of(fb_dst_va(0)), 8),
        vec![0u8; 8],
        "⊘ and entry [0]'s destination is untouched, which is what makes the re-run sound"
    );

    // ★★★ THE SECOND PROPERTY: **one drain, not one page per trip.** The want set already
    // holds entry [1]'s grain, put there by the question and not by an attempted access.
    let wanted = port.wanted();
    assert!(
        wanted.contains(&(fb_phys_of(fb_dst_va(1)) & !(TP_GRAIN - 1))),
        "entry [1]'s destination grain must be wanted: {wanted:x?}"
    );
    assert_eq!(
        port.accesses_refused
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "⊘⊘ AND NOT ONE ACTUAL ACCESS WAS REFUSED — every miss came from a QUESTION. \
         `[measured w742]` the accesses are what put `bytes` and `completions` on the \
         refusal in the first place"
    );

    // One drain arms everything that was wanted, and the re-run is served whole.
    let armed = port.drain_all();
    assert!(armed >= 1, "the drain armed {armed} runs");
    let run = ring_once_on(&plane, &mut vmm, &mut cursor, &mut state, small_channel(2))
        .expect("★★★★★ the re-run is served, both entries, after ONE drain");
    assert_eq!(run.launches, 2, "both entries ran: {run:?}");
    assert_eq!(run.bytes, 0x80, "both 64-byte fills moved: {run:?}");
    for d in 0..2u32 {
        assert_eq!(
            port.peek(fb_phys_of(fb_dst_va(d)), 4),
            vec![doorbell_fill(d); 4],
            "doorbell {d}'s fill byte is in the reserved object"
        );
    }
    assert_eq!(
        vmm.ram_read(phys_of(PB_GPU_VA + FINISH_PAYLOAD_OFFSET), 4),
        doorbell_payload(1).to_le_bytes(),
        "★ and the LAST payload is the one the guest is polling for"
    );
}

/// ★★ **THE PRE-FLIGHT ASKS ABOUT THE COMPLETION WORD TOO**, not only the copy operands.
///
/// ⊘ A release is the one write whose refusal mid-record is unrecoverable: the guest may
/// already have read half a payload. Here the copy's destination is armed and the semaphore
/// page is not, so a pre-flight that only looked at `CeSpan`s would let the copy run and then
/// refuse on the release — `bytes > 0`, gate shut, exactly the w740 shape one field over.
#[test]
fn the_preflight_asks_about_the_completion_word_and_not_only_the_copy() {
    let port = std::sync::Arc::new(TestPort::default());
    let plane = device_plane_with_tree(&port);
    // ★ The semaphore is in the FRAMEBUFFER region this time; the destination is armed.
    let sema_va = FBDST_VA + 0x8_0000;
    let block = memset_block(fb_dst_va(0), 0x40, 0x5A, sema_va, 7);
    let mut vmm = guest_ram(&block);
    port.arm(fb_phys_of(fb_dst_va(0)));
    let mut cursor = GpCursor::default();

    let err = ring_once(&plane, &mut vmm, &mut cursor, 1)
        .expect_err("the completion word has no armed view");
    assert_eq!(
        err.progress,
        kayfabe_rt::ceutils::CeProgress::NONE,
        "⊘ refused before the copy ran, not after it: {err:?}"
    );
    assert_eq!(
        port.peek(fb_phys_of(fb_dst_va(0)), 4),
        vec![0u8; 4],
        "⊘ and the ARMED destination is still untouched — the pre-flight refused the whole \
         submission, not just its release"
    );
    assert!(
        port.wanted()
            .contains(&(fb_phys_of(sema_va) & !(TP_GRAIN - 1))),
        "the completion word's grain is in the want set"
    );

    port.drain_all();
    let run = ring_once(&plane, &mut vmm, &mut cursor, 1).expect("served after one drain");
    assert_eq!(run.completions, 1, "one payload released: {run:?}");
    assert_eq!(
        port.peek(fb_phys_of(sema_va), 4),
        7u32.to_le_bytes().to_vec(),
        "★ and it is in the reserved object, at the word the guest polls"
    );
}

/// ⊘ **A WALK refusal is deliberately NOT the pre-flight's business** — it is refused by the
/// execute loop, by name, and the pre-flight must not widen into diagnosing it. This is the
/// no-op control for the skip in the `3b` block: an unmapped destination on a device store
/// still reports the walk's own fault, not `CpuCeFb`.
#[test]
fn the_preflight_leaves_a_walk_refusal_to_the_loop_that_names_it() {
    let port = std::sync::Arc::new(TestPort::default());
    let plane = device_plane_with_tree(&port);
    let block = memset_block(
        UNMAPPED_DST,
        0x40,
        0x11,
        PB_GPU_VA + FINISH_PAYLOAD_OFFSET,
        1,
    );
    let mut vmm = guest_ram(&block);
    let mut cursor = GpCursor::default();
    let err = ring_once(&plane, &mut vmm, &mut cursor, 1).expect_err("the destination is unmapped");
    assert!(
        !matches!(err.fault, FwdFault::CpuCeFb { .. }),
        "⊘ the pre-flight must not rename a walk fault: {err:?}"
    );
    assert_eq!(
        err.progress.launches, 1,
        "★ and the loop was entered — `launches` is bumped at DECODE: {err:?}"
    );
    assert!(err.progress.may_re_run(), "still re-runnable: {err:?}");
}
