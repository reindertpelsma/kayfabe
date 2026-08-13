//! ★★★★★ **w281 — reading a guest PUSHBUFFER out of OUR OWN emulated framebuffer.**
//!
//! # What this measures, and why it is a separate file from the ring's
//!
//! `[w279, traces/boots/w279/RESULT.md]` wired the **ring** out of the framebuffer and left
//! the **pushbuffer the ring points at** refused by a hard-coded
//! [`kayfabe_fwd::VidmemRoute::Refuse`], deliberately, so that boot could attribute the
//! ring's bytes to one read. `[w280, traces/boots/w280/RESULT.md]` then measured that the
//! raw CE client's pushbuffer is **`pb=V:0x40000` — vidmem** (while all 16 of `cup2`'s are
//! `pb=S:`), so that gate is on the client's path and on nothing else. This file is the
//! widening's known-positive.
//!
//! # ⊘ Both polarities, because "armed" and "reachable" are different facts
//!
//! The route needs **two** things: the flag ([`kayfabe_fwd::plan_pushbuffer`]'s `vidmem`,
//! wired to `KAYFABE_PUSHBUF_VIDMEM`) **and** a store to read from (route B's `FbSource`).
//! A test that only proved the happy path could not tell *"the widening works"* from
//! *"something else served those bytes"*, so every assertion here is paired with the arm
//! that must still refuse.
//!
//! # ⊘ Why there is no `RingFbNeverWritten` equivalent, asserted rather than assumed
//!
//! The ring needs a residency gate because a never-written ring page is byte-identical to a
//! quiet one and decodes to `NoLiveEntries` — self-concealing (forbidden #2). A pushbuffer
//! has no such degenerate reading: an unwritten page decodes to **zero methods**, which is
//! visible as a count. [`an_unwritten_vidmem_pushbuffer_decodes_to_no_methods`] pins that,
//! so the asymmetry is a measurement and not a claim.

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_arch::{Aperture, PushRange};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, GpFifoRing, RmEvent};
use kayfabe_core::{ChanId, ProcId};
use kayfabe_fwd::{FbBytes, FwdFault};
use kayfabe_mocks::{
    MockArch, MockIsolateFactory, MockPushbuffer, MockVmm, mock_classes as mc, mock_method,
};
use kayfabe_tests::Scenario;
use std::collections::BTreeMap;

const GPU: GpuId = GpuId::ZERO;
const PDB0: Pdb = Pdb(0x4002_0000);
const CLIENT: HClient = HClient(0xC0B_0000);
const CE_VCHID: VChid = VChid(0x31);

/// The ring, only so the channel is well formed. Nothing here reads it.
const RING_VA: GpuVa = GpuVa(0x4_2006_4000);

/// ★ The subject: the guest's pushbuffer VA and the **framebuffer offset** its binding
/// names. ⊘ Deliberately unequal — an identity mapping could not tell a translated read
/// from an untranslated one, nor an FB read from a guest-RAM read.
const PB_VA: GpuVa = GpuVa(0x1_2002_0000);
const PB_FB_PHYS: u64 = 0x0004_0000;
const PB_LEN: u64 = 64;

/// ★★★ A vidmem pushbuffer page nothing ever wrote, at a **different** VA — the arm that
/// shows why the ring's residency gate is not needed here.
const BLANK_VA: GpuVa = GpuVa(0x1_2003_0000);
const BLANK_FB_PHYS: u64 = 0x0005_0000;

/// The distinctive method stream the fixture puts in the framebuffer: a GA10x-shaped
/// `SET_OBJECT` header plus one operand, in `MockArch`'s encoding.
///
/// ⊘ The **value** matters: a read that silently returned zeros, or that read guest RAM at
/// the number `PB_FB_PHYS` happens to share, produces a different decode — which is exactly
/// the silent-wrong-bytes failure the aperture split exists to prevent.
fn pb_words() -> Vec<u32> {
    // One method, one operand — enough that a zeros read cannot imitate it.
    let (header, args) = MockPushbuffer::method(mock_method::SEM_RELEASE, &[0xC0FF_EE33, 0, 1, 0]);
    let mut w = vec![header];
    w.extend(args);
    w
}

/// ★★★★ A framebuffer answering both questions, with `page_written` stored SEPARATELY from
/// the bytes so a never-written page that reads as zeros is representable.
#[derive(Debug, Default)]
struct FakeFb {
    pages: BTreeMap<u64, Vec<u8>>,
}

impl FakeFb {
    fn page(&mut self, phys: u64, bytes: &[u8]) {
        let frame = phys & !0xFFF;
        let p = self.pages.entry(frame).or_insert_with(|| vec![0u8; 4096]);
        let off = (phys - frame) as usize;
        p[off..off + bytes.len()].copy_from_slice(bytes);
    }
}

impl FbBytes for FakeFb {
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool {
        let frame = phys & !0xFFF;
        match self.pages.get(&frame) {
            Some(p) => {
                let off = (phys - frame) as usize;
                buf.copy_from_slice(&p[off..off + buf.len()]);
            }
            // The production semantic: inside the aperture, never written ⇒ zeros + success.
            None => buf.fill(0),
        }
        true
    }

    fn page_written(&self, phys: u64) -> Option<bool> {
        Some(self.pages.contains_key(&(phys & !0xFFF)))
    }
}

/// A guest whose channel has a **vidmem** pushbuffer binding at [`PB_VA`], plus a blank
/// vidmem page at [`BLANK_VA`].
fn guest_with_a_vidmem_pushbuffer() -> (Gpu, MockVmm, ProcId, ChanId) {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");

    let root = HObject(0xC0B_0000);
    let dev = HObject(0xC0B_0001);
    let vas = HObject(0xC0B_0010);
    let tsg = HObject(0xC0B_0012);
    let chan = HObject(0xC0B_001A);

    let mut s = Scenario::new();
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: root,
        handle: root,
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(CLIENT),
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: root,
        handle: dev,
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: dev,
        handle: vas,
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client: CLIENT,
        vaspace: vas,
        pdb: PDB0,
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: dev,
        handle: tsg,
        class: mc::TSG,
        facts: AllocFacts {
            h_vaspace: Some(vas),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: tsg,
        handle: chan,
        class: mc::CHANNEL_CE,
        facts: AllocFacts {
            h_vaspace: Some(vas),
            userd_flags: MockArch::userd_flags_for(CE_VCHID),
            gp_fifo_ring: Some(GpFifoRing {
                va: RING_VA.0,
                entries: 256,
            }),
            ..Default::default()
        },
    });
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }

    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB0)).expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the scenario's channel");

    {
        let proc = gpu.procs.get_mut(&pid).expect("live");
        let vas = proc.vases.get_mut(&(GPU, PDB0)).expect("the VAS exists");
        for (va, phys) in [(PB_VA, PB_FB_PHYS), (BLANK_VA, BLANK_FB_PHYS)] {
            vas.table
                .bind(
                    PDB0,
                    va,
                    4096,
                    kayfabe_mmu::Binding::declared_by_guest(phys, Aperture::Vidmem)
                        .expect("a kind the guest can declare"),
                )
                .expect("the vidmem pushbuffer binds");
        }
    }

    (gpu, MockVmm::new(), pid, cid)
}

fn fb_with_the_methods() -> FakeFb {
    let mut fb = FakeFb::default();
    let bytes: Vec<u8> = pb_words().iter().flat_map(|w| w.to_le_bytes()).collect();
    fb.page(PB_FB_PHYS, &bytes);
    fb
}

fn range() -> [PushRange; 1] {
    [PushRange {
        va: PB_VA,
        len: PB_LEN,
    }]
}

/// ⊘⊘ **THE DEFAULT STILL REFUSES.** `vidmem = false` is byte-for-byte the pre-`w281`
/// behaviour, and the refusal is raised in the PLAN phase — under the lock, before a byte
/// is planned, let alone read.
#[test]
fn the_default_route_still_refuses_a_vidmem_pushbuffer() {
    let (gpu, _vmm, pid, cid) = guest_with_a_vidmem_pushbuffer();
    let err = kayfabe_fwd::plan_pushbuffer(&gpu.procs[&pid], cid, &range(), false)
        .expect_err("the default route must refuse vidmem");
    assert!(
        matches!(
            err,
            FwdFault::PushbufferAperture {
                va,
                aperture: Aperture::Vidmem
            } if va == PB_VA
        ),
        "the refusal must name the guest's own VA: {err:?}"
    );
}

/// ★★★ **THE WIDENING — the known-positive.** Armed *and* supplied, the decoder receives
/// the framebuffer's own bytes.
#[test]
fn an_armed_and_supplied_route_reads_the_pushbuffer_out_of_the_framebuffer() {
    let (gpu, mut vmm, pid, cid) = guest_with_a_vidmem_pushbuffer();
    let plan = kayfabe_fwd::plan_pushbuffer(&gpu.procs[&pid], cid, &range(), true)
        .expect("the armed route plans a vidmem run");
    assert!(
        plan.touches_fb(),
        "the plan must SAY it reads our framebuffer — a route that is armed and never taken \
         must not read as taken"
    );
    let mut fb = fb_with_the_methods();
    let bytes = kayfabe_fwd::fetch_pushbuffer(&plan, &mut vmm, Some(&mut fb))
        .expect("the supplied store serves the range");
    let methods = kayfabe_fwd::decode_pushbuffer(&gpu.spine, &bytes);
    // ⊘ Graded on the VALUE, never on "some methods came back": zeros would also decode to
    // *something*, and guest RAM at the number `PB_FB_PHYS` shares would decode to
    // something else again. Only the fixture's own operand distinguishes the three.
    assert!(
        methods.iter().any(|(_, ops)| ops.contains(&0xC0FF_EE33)),
        "the decoded methods must carry the framebuffer's own operand: {methods:?}"
    );
}

/// ⊘⊘ **ARMED IS NOT ENOUGH — the flag without route B's store still refuses**, and it
/// refuses with the same name and the same VA as the unarmed path. This is the
/// *necessary-not-sufficient* half stated as a test rather than as a comment.
#[test]
fn an_armed_route_with_no_store_still_refuses_and_names_the_same_va() {
    let (gpu, mut vmm, pid, cid) = guest_with_a_vidmem_pushbuffer();
    let plan = kayfabe_fwd::plan_pushbuffer(&gpu.procs[&pid], cid, &range(), true)
        .expect("the armed route plans a vidmem run");
    let err = kayfabe_fwd::fetch_pushbuffer(&plan, &mut vmm, None)
        .expect_err("no store ⇒ nothing may be served");
    assert!(
        matches!(
            err,
            FwdFault::PushbufferAperture {
                va,
                aperture: Aperture::Vidmem
            } if va == PB_VA
        ),
        "an armed-but-unsupplied route must refuse by the guest's VA: {err:?}"
    );
}

/// ⊘⊘ **A never-written vidmem pushbuffer yields NO RECOGNISED FACT** — and this test was
/// written asserting the wrong thing, failed, and the *source comment it was checking* was
/// the thing that turned out to be false.
///
/// `[w281, measured]` The claim under test was *"an unwritten page decodes to **zero
/// methods**"*. It decodes to **16 `(0, [])` pairs** — a non-zero count. On GA10x a zero
/// header is `sec_op = GRP0_USE_TERT` / `tert_op = TERT_OP_METHOD` ⇒ `MethodForm::Legacy`,
/// `arg_words = 0`, and `decode_method` answers [`PushMethod::Opaque`] because the form is
/// not `Incrementing`. `MockArch` reproduces the shape.
///
/// ⇒ The property that actually licenses having no `RingFbNeverWritten` equivalent here is
/// about **facts, not counts**: every method a blank page decodes to is `Opaque`, so a blank
/// pushbuffer produces no `SetObject`, no CE span and no semaphore release. It cannot
/// imitate work, which is forbidden #2's real requirement. ⚠ A *count* would have been no
/// discriminator at all — the thing the false claim asserted it was.
#[test]
fn an_unwritten_vidmem_pushbuffer_yields_no_recognised_fact() {
    let (gpu, mut vmm, pid, cid) = guest_with_a_vidmem_pushbuffer();
    let blank = [PushRange {
        va: BLANK_VA,
        len: PB_LEN,
    }];
    let plan = kayfabe_fwd::plan_pushbuffer(&gpu.procs[&pid], cid, &blank, true)
        .expect("the armed route plans the blank range too");
    // The store holds the *written* page and not this one, so `read` fills zeros and succeeds
    // — the production semantic exactly.
    let mut fb = fb_with_the_methods();
    let bytes = kayfabe_fwd::fetch_pushbuffer(&plan, &mut vmm, Some(&mut fb))
        .expect("zeros and success, as the aperture promises");
    let methods = kayfabe_fwd::decode_pushbuffer(&gpu.spine, &bytes);
    // ⊘ The count is NOT zero, and asserting that it was is what failed. Pinned, so the
    // false claim cannot come back as a "simplification".
    assert!(
        !methods.is_empty(),
        "a blank page decodes to null-header pairs, not to nothing — if this ever becomes \
         empty the reasoning below is about a codec that no longer exists"
    );
    let pb = gpu.spine.arch().pushbuffer();
    for (header, args) in &methods {
        assert_eq!(
            pb.decode_method(*header, args),
            kayfabe_arch::PushMethod::Opaque,
            "a blank pushbuffer must yield NO recognised fact — this one decoded \
             ({header:#x}, {args:?}) to something actionable"
        );
    }
}

/// ⊘ **A MISS is still a MISS.** The widening changes the aperture arm and nothing else; an
/// unbound VA faults exactly as before, under the lock, in the plan phase.
#[test]
fn the_widening_does_not_relax_a_miss() {
    let (gpu, _vmm, pid, cid) = guest_with_a_vidmem_pushbuffer();
    let nowhere = [PushRange {
        va: GpuVa(0x7_0000_0000),
        len: PB_LEN,
    }];
    let err = kayfabe_fwd::plan_pushbuffer(&gpu.procs[&pid], cid, &nowhere, true)
        .expect_err("an unbound VA must still fault");
    assert!(
        matches!(err, FwdFault::Address(_)),
        "MISS = FAULT survives the widening: {err:?}"
    );
}
