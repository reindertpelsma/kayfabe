//! ★★★★★ **w235 / route B — reading a guest GPFIFO ring out of OUR OWN emulated
//! framebuffer, and the blank-page refusal that makes it safe to attempt.**
//!
//! # ⊘ This file measures a ROUTE; it does not endorse one
//!
//! `[w235, 2026-08-11]` The 8 `proc 2` doorbells the boot reports have their ring in the
//! **emulated framebuffer**, not guest RAM — the descent already prints its bytes
//! (`fbRING[p0]@0x1024000=0000c002…`) while `read_gpfifo_ring` refuses them by name
//! (`FwdFault::PushbufferAperture`). Route B reads them from the store we already serve.
//! Route A — influencing the ring's aperture at allocation time so it lands in sysmem — is
//! being measured independently; if it answers YES it is the better route and this is a
//! stepping stone. ⇒ [`kayfabe_fwd::VidmemRoute::Refuse`] is the default at every
//! production call site, and `the_default_route_still_refuses_vidmem` is the assertion
//! that says so.
//!
//! # ★★★ The hazard this file exists for
//!
//! `FbStore::read` answers an address inside the aperture that **nothing ever wrote** with
//! *zeros and `Ok`*, deliberately. A GPFIFO ring is *supposed* to be mostly zeros —
//! `gpfifo_live_entries` stops at the first zero entry because RM zero-initialises the
//! buffer. ⇒ a never-written ring page and a legitimately quiet one are **byte-identical**,
//! and reading the first would report `NoLiveEntries` and look exactly like a correct
//! channel. That is `ce_executor_tree.md`'s **forbidden #2** in its self-concealing form,
//! which is why the gate is residency (*a page nothing ever wrote is not in the map*) and
//! never a byte census.

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, GpFifoRing, RmEvent};
use kayfabe_core::{ChanId, ProcId};
use kayfabe_fwd::{FbBytes, FwdFault, RingLook};
use kayfabe_mocks::{MockArch, MockIsolateFactory, MockVmm, mock_classes as mc};
use kayfabe_tests::Scenario;
use std::collections::BTreeMap;

const GPU: GpuId = GpuId::ZERO;
const PDB0: Pdb = Pdb(0x4002_0000);
const CLIENT: HClient = HClient(0xC0B_0000);
const CE_VCHID: VChid = VChid(0x31);

/// The guest's ring VA, and the **framebuffer offset** its binding names. ⊘ Deliberately
/// not equal: an identity mapping could not tell a translated read from an untranslated
/// one, and could not tell an FB read from a guest-RAM read either.
const RING_VA: GpuVa = GpuVa(0x4_2006_4000);
const RING_FB_PHYS: u64 = 0x0102_4000;
const RING_LEN: u64 = 4096;

/// ★★★★ **A framebuffer that answers BOTH questions** — the bytes, and whether the page
/// was ever written.
///
/// ⊘ The two are stored separately **on purpose**, so the fixture can construct the exact
/// state the production store produces and the gate exists for: a page that reads as zeros
/// and was never written. A mock that inferred `page_written` from *"are the bytes
/// non-zero"* would make the hazard unrepresentable and the falsifier unable to fail.
#[derive(Debug, Default)]
struct FakeFb {
    /// Page-aligned frames this store actually holds, by framebuffer address.
    pages: BTreeMap<u64, Vec<u8>>,
    /// ⊘ When true, [`FbBytes::page_written`] answers `None` — *"this store cannot tell
    /// you"* — which is the `dlen=0` shape and must NOT be read as `false`.
    mute: bool,
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
        // ★ The production semantic, reproduced exactly: an address inside the aperture
        // that was never written reads ZERO and succeeds. That is what makes the byte
        // census useless here and residency load-bearing.
        let frame = phys & !0xFFF;
        match self.pages.get(&frame) {
            Some(p) => {
                let off = (phys - frame) as usize;
                buf.copy_from_slice(&p[off..off + buf.len()]);
            }
            None => buf.fill(0),
        }
        true
    }

    fn page_written(&self, phys: u64) -> Option<bool> {
        if self.mute {
            return None;
        }
        Some(self.pages.contains_key(&(phys & !0xFFF)))
    }
}

/// A guest whose channel declares its ring at [`RING_VA`], bound to **`Aperture::Vidmem`**
/// at [`RING_FB_PHYS`] — the shape the boot's `proc 2` channels actually have.
fn guest_with_a_vidmem_ring() -> (Gpu, MockVmm, ProcId, ChanId) {
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

    // ★★★ THE SUBJECT: the ring's binding names VIDMEM, so `phys` is a framebuffer offset
    // and not a GPA. This is the arm every production call site refuses today.
    {
        let proc = gpu.procs.get_mut(&pid).expect("live");
        let vas = proc.vases.get_mut(&(GPU, PDB0)).expect("the VAS exists");
        vas.table
            .bind(
                PDB0,
                RING_VA,
                RING_LEN,
                kayfabe_mmu::Binding {
                    phys: RING_FB_PHYS,
                    aperture: Aperture::Vidmem,
                    host: None,
                },
            )
            .expect("the vidmem ring binds");
    }

    (gpu, MockVmm::new(), pid, cid)
}

/// ★★★★★ **THE NEGATIVE CONTROL, and it is the reason the route is safe to attempt.**
///
/// The ring's framebuffer page was **never written**. `FbBytes::read` would hand back 4 KiB
/// of zeros and `true`; `gpfifo_live_entries` would count 0 live entries; the doorbell would
/// report `Served`, and the boot line would be **indistinguishable from a correct quiet
/// channel**. This asserts the read refuses **by name** instead.
///
/// ⊘ Watched RED first: with the `page_written` gate commented out, this returns
/// `Ok(RingLook::Ring([0u8; 4096]))` — a green-looking blank, which is exactly forbidden #2.
#[test]
fn a_ring_page_our_framebuffer_never_wrote_is_refused_by_name() {
    let (gpu, mut vmm, pid, cid) = guest_with_a_vidmem_ring();
    let mut fb = FakeFb::default(); // ⊘ nothing written: no page at RING_FB_PHYS

    let got =
        kayfabe_fwd::read_gpfifo_ring(&gpu.spine, &gpu.procs[&pid], cid, &mut vmm, Some(&mut fb));

    assert_eq!(
        got,
        Err(FwdFault::RingFbNeverWritten {
            va: RING_VA,
            phys: RING_FB_PHYS,
        }),
        "★★★★★ FORBIDDEN #2, read side: a framebuffer page NOTHING EVER WROTE must be \
         refused by name, not read as 4 KiB of zeros. Zeros here are a legitimately quiet \
         ring's own encoding, so the byte census cannot tell the two apart and only \
         residency can. Got {got:?}"
    );
}

/// ★★★ **THE KNOWN-POSITIVE — without it the gate above could be a blanket refusal.**
///
/// The same fixture, the same route, the same address, with the page **written**: the ring
/// must come back, and its bytes must be the ones the framebuffer holds.
///
/// ⊘ §16.85.3's class, applied to this file: *a census's zero means nothing until the
/// census has been shown to produce a non-zero for a known-positive.* A fix that refused
/// every vidmem ring would pass the negative control alone.
#[test]
fn the_same_ring_with_the_page_written_reads_the_framebuffers_bytes() {
    let (gpu, mut vmm, pid, cid) = guest_with_a_vidmem_ring();
    let mut fb = FakeFb::default();
    // The guest's own bytes, as the descent prints them: `0000c002…`.
    let entry = [0x00u8, 0x00, 0xc0, 0x02, 0x00, 0x00, 0x00, 0x00];
    fb.page(RING_FB_PHYS, &entry);

    let got =
        kayfabe_fwd::read_gpfifo_ring(&gpu.spine, &gpu.procs[&pid], cid, &mut vmm, Some(&mut fb))
            .expect("a written framebuffer page reads");

    let RingLook::Ring(bytes) = got else {
        panic!(
            "★ the known-positive must REACH the ring, or the refusal above proves only \
             that we refuse everything. Got {got:?}"
        );
    };
    assert_eq!(
        &bytes[..entry.len()],
        &entry,
        "★★ the bytes must be the FRAMEBUFFER's, at the framebuffer offset the binding \
         named — not guest RAM at the same number, which is the failure this route's \
         `PushSrc` split exists to make unrepresentable"
    );
}

/// ★★★★ **`None` from `page_written` is UNMEASURED, and must not be read as `false`.**
///
/// A store that cannot track origins says *"I cannot tell you"*. Treating that as *"never
/// written"* would refuse live traffic on the strength of an instrument's silence — the
/// `dlen=0` lesson exactly: **an empty capture is evidence of nothing, not evidence of
/// emptiness.** ⊘ And the inverse error is the one this arm guards: a `None` collapsed to
/// `Some(false)` would make the route look broken on every store but one.
#[test]
fn a_store_that_cannot_answer_residency_is_unmeasured_not_refused() {
    let (gpu, mut vmm, pid, cid) = guest_with_a_vidmem_ring();
    let mut fb = FakeFb {
        mute: true,
        ..Default::default()
    };

    let got =
        kayfabe_fwd::read_gpfifo_ring(&gpu.spine, &gpu.procs[&pid], cid, &mut vmm, Some(&mut fb));
    assert!(
        matches!(got, Ok(RingLook::Ring(_))),
        "★★★★ `None` is UNMEASURED. A store with no origin tracking must not be turned \
         into a positive claim that the guest never wrote the page. Got {got:?}"
    );
}

/// ★★★★★ **THE DEFAULT IS STILL REFUSAL** — the switch is opt-in, structurally.
///
/// Same fixture, same vidmem binding, **no framebuffer passed**: the answer must be the
/// pre-w235 one, `FwdFault::PushbufferAperture`. ⊘ This is what makes "default-off" a
/// property of the code rather than a claim in a commit message — and it is the assertion
/// that keeps the owner's open scope question open: nothing reaches the framebuffer route
/// without a caller handing it a store.
#[test]
fn the_default_route_still_refuses_vidmem() {
    let (gpu, mut vmm, pid, cid) = guest_with_a_vidmem_ring();

    let got = kayfabe_fwd::read_gpfifo_ring(&gpu.spine, &gpu.procs[&pid], cid, &mut vmm, None);

    assert_eq!(
        got,
        Err(FwdFault::PushbufferAperture {
            va: RING_VA,
            aperture: Aperture::Vidmem,
        }),
        "★★★★★ with no framebuffer handed in, the vidmem ring must be refused exactly as \
         before this rung existed. A default-off flag that is not off by construction is a \
         default-on flag with a comment. Got {got:?}"
    );
}
