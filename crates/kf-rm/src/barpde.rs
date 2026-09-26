//! ★★★ **`UPDATE_BAR_PDE` (fn 70) — the guest hands us its `PDE3[0]` for OUR BAR2 root**
//! (`V3_P4_PORT_MAP.md` §2.2, §1.1 row 3; `THE_ARCHITECTURE_v3.md` §4.3).
//!
//! On the GSP-offload model CPU-RM owns the BAR2 VA range under `PDE3[0]` and the firmware owns
//! `PDE3[1]`; only the firmware's root is bound to hardware. CPU-RM builds its own tree (through
//! the PRAMIN window), reads its own `PDE3[0]` back, and sends that eight-byte value for the
//! firmware to install in slot 0 of **its** root (`kbusPatchBar2Pdb_GSPCLIENT`,
//! `ogkm-580: kern_bus.c:826-880`) — whose address it took from `bar2PdeBase`. **We are the
//! firmware**: the root is a page WE declared (`kf_chip::bar0::FbLayout::bar2_pde_base`), and this
//! link hands the entry to the memory plane, which writes it into that page on the GPU and walks.
//!
//! ★ The reply is HELD until that write has landed and the walk reconciled — the RPC-map
//! synchronisation point (`the_three_synchronization_points` #2; [`kf_gsp::CommandPolicy::
//! holds_for_refresh`]). `rpcUpdateBarPde_v15_00` waits for it (`_issueRpcAndWait`,
//! `ogkm-580: rpc.c:9703-9724`), so the guest cannot build on the root before it exists.
//!
//! The body decode is copied from the old tree's `kayfabe-device/src/bar2.rs:256-278` (pure; ran
//! on hardware). Its log (`BarPdeLog`) is not: v3 carries the statement to the plane that acts on
//! it instead of recording it beside the plane.

use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV_OK`.
const NV_OK: u32 = 0;
/// `NV_ERR_INVALID_ARGUMENT`.
const NV_ERR_INVALID_ARGUMENT: u32 = 0x0000_001F;
/// `NV_RPC_UPDATE_PDE_BAR_1` (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_headers.h:224-227`).
pub const BAR_TYPE_1: u32 = 0;
/// `NV_RPC_UPDATE_PDE_BAR_2`.
pub const BAR_TYPE_2: u32 = 1;
/// `offsetof(UpdateBarPde_v15_00, entryValue)` — `barType` is a 4-byte enum and the member is
/// `NV_ALIGN_BYTES(8)`, so four bytes of padding sit between them
/// (`ogkm-580: src/nvidia/generated/g_sdk-structures.h:743-748`). ⚠ Packing it at 4 reads the low
/// half of the entry as the whole value: a plausible root one 4 GiB region away.
const ENTRY_VALUE_OFF: usize = 8;
/// `offsetof(UpdateBarPde_v15_00, entryLevelShift)`.
const ENTRY_LEVEL_SHIFT_OFF: usize = 16;
/// `sizeof(UpdateBarPde_v15_00)`.
pub const UPDATE_BAR_PDE_BODY_SIZE: usize = 24;

/// Which bus aperture a root belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarAperture {
    /// The framebuffer window.
    Bar1,
    /// The instance window.
    Bar2,
}

/// ★ One published root entry, whole. `level_shift` travels with it (`pRootFmt->virtAddrBitLo`)
/// and is the only statement of which level the entry belongs to.
///
/// ⚠ **Zero is a real message**: `kbusDestroyBar2GpuVaSpace` publishes `entryValue = 0` to unroot
/// the aperture on teardown (`ogkm-580: kern_bus_gm107.c:2137`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedPde {
    /// The aperture.
    pub bar: BarAperture,
    /// `entryValue` — eight raw bytes, as the guest read them out of its own directory.
    pub entry: u64,
    /// `entryLevelShift`.
    pub level_shift: u64,
}

/// Why a body was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarPdeRefusal {
    /// Shorter than [`UPDATE_BAR_PDE_BODY_SIZE`].
    ShortBody {
        /// Bytes that arrived.
        len: usize,
    },
    /// A `barType` outside `{BAR_1, BAR_2}` (including `_INVALID`, 2).
    UnknownBarType {
        /// The value.
        bar_type: u32,
    },
}

/// Decode one `UpdateBarPde_v15_00` body.
///
/// # Errors
/// [`BarPdeRefusal`].
pub fn decode_update_bar_pde(body: &[u8]) -> Result<PublishedPde, BarPdeRefusal> {
    if body.len() < UPDATE_BAR_PDE_BODY_SIZE {
        return Err(BarPdeRefusal::ShortBody { len: body.len() });
    }
    let u64_at = |o: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&body[o..o + 8]);
        u64::from_le_bytes(b)
    };
    let bar_type = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let bar = match bar_type {
        BAR_TYPE_1 => BarAperture::Bar1,
        BAR_TYPE_2 => BarAperture::Bar2,
        _ => return Err(BarPdeRefusal::UnknownBarType { bar_type }),
    };
    Ok(PublishedPde { bar, entry: u64_at(ENTRY_VALUE_OFF), level_shift: u64_at(ENTRY_LEVEL_SHIFT_OFF) })
}

/// ★ A statement the guest made about its address spaces, carried to the memory plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemStatement {
    /// fn 70.
    BarPde(PublishedPde),
    /// `SET_PAGE_DIRECTORY` / `COPY_SERVER_RESERVED_PDES` — see
    /// [`crate::rmrpc::PageDirStatement`].
    PageDir(crate::rmrpc::PageDirStatement),
    /// ★ P5c: the guest's VA-space OBJECT is gone — its own free, its device's, or its client's.
    /// The plane retires the mirror (our rows unmapped, the host space recycled). ⊘ Never for a
    /// `GPU_DEVICE` reference's own handle: that name is transient (RM frees it right after
    /// publishing the PDEs, `[measured p5c]`), the device-default space outlives it until the
    /// DEVICE goes.
    Retire {
        /// `hClient`.
        client: u32,
        /// The VA-space object.
        vaspace: u32,
    },
    /// ★★★ v3-refusals: the guest's sysmembar (`INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR`,
    /// [`crate::sysmembar`]). The plane performs it as the host sysmembar verb and only then
    /// settles, so the held `NV_OK` is posted after the host GPU has flushed.
    Sysmembar,
}

/// ★ Where statements go: the device's memory plane. Called on the register drainer, so it must
/// only ENQUEUE (the plane's own thread does the work); the reply is held meanwhile.
pub type MemSink = std::sync::Arc<dyn Fn(MemStatement) + Send + Sync>;

/// ★★★ The chain link that answers `UPDATE_BAR_PDE`, hands the entry to the memory plane, and
/// holds the reply until the plane says the root is written and walked.
pub struct BarPdePolicy {
    sink: MemSink,
    held_last: bool,
    /// Accepted publications.
    pub published: u64,
    /// Refused bodies.
    pub refused: u64,
}

impl BarPdePolicy {
    /// A link that sends to `sink`.
    #[must_use]
    pub fn new(sink: MemSink) -> BarPdePolicy {
        BarPdePolicy { sink, held_last: false, published: 0, refused: 0 }
    }
}

impl core::fmt::Debug for BarPdePolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BarPdePolicy").field("published", &self.published).field("refused", &self.refused).finish()
    }
}

impl CommandPolicy for BarPdePolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::UpdateBarPde {
            return None;
        }
        // ⊘ `payload`, not `wire_body()`: the struct has no flexible array and the declared
        // length covers it whole (`ogkm-580: rpc.c:9710-9718`).
        match decode_update_bar_pde(&cmd.payload) {
            Ok(pde) => {
                (self.sink)(MemStatement::BarPde(pde));
                self.published += 1;
                self.held_last = true;
                // ★ An EMPTY body with NV_OK: the command has no `[OUT]` field.
                Some(Reply { rpc_result: NV_OK, body: Vec::new() })
            }
            Err(e) => {
                self.refused += 1;
                self.held_last = false;
                eprintln!("kf-rm: UPDATE_BAR_PDE refused: {e:?} — the aperture stays as it was");
                Some(Reply { rpc_result: NV_ERR_INVALID_ARGUMENT, body: Vec::new() })
            }
        }
    }

    /// ★ Held iff the statement went to the plane — a refused body changed nothing to wait for.
    fn holds_for_refresh(&self, cmd: &RpcCommand) -> bool {
        cmd.function == RpcFunction::UpdateBarPde && self.held_last
    }
}

/// ★★★ **The page-directory statements, carried to the memory plane** (`V3_P4_PORT_MAP.md` §2.2,
/// Q10): `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x00801813`, UVM's) and
/// `COPY_SERVER_RESERVED_PDES` (`0x90f10106` / `0x20800a9f`, every RM-managed VA space at
/// construct time) become a ROOT on that VA-space object in the plane's `VasTable`.
///
/// Two behaviours, by control:
/// - **The publications are OBSERVED, never answered.** `InitTablePolicy` answers them (its
///   decode, validation and re-encode) and this link must not take that reply away; it is seated
///   AHEAD of every answerer so it sees the control at all.
/// - **`SET_PAGE_DIRECTORY` is ANSWERED** — `NV_OK`, the params echoed (the guest copies the
///   reply's params back over its caller's struct, `ogkm-580: rpc.c:11085-11090`; the old
///   `SetPageDirPolicy`'s §16.33 shape). ⊘ Refusing it makes RM roll the directory back
///   (`ogkm-580: dma.c:531-551`), so a root carried to the plane with the control refused would
///   be a root the guest abandoned. A sysmem root is refused by name in `rmrpc`
///   (`SetPageDirRootAperture`) and never reaches here as a statement.
///
/// ★ Either way the reply, whoever authors it, is delivered only once the plane has reconciled
/// the new root (Q10: *"a root change schedules a full walk … the guest-visible RPC reply is held
/// until the reconcile lands"*). A control that translates to no statement holds nothing.
pub struct PageDirPolicy {
    abi: kf_abi::versions::DriverAbiTable,
    guest_os: kf_abi::GuestOs,
    sink: MemSink,
    held_last: bool,
    /// ★ P5c: every VA-space object allocated, `(hClient, hVaSpace)` → `(parent device, a
    /// GPU_DEVICE reference)` — what a later free retires.
    vas: std::collections::BTreeMap<(u32, u32), (u32, bool)>,
    /// ★ P6b: `DUP_OBJECT` aliases of a VA-space object, `(dst client, dst handle)` → the
    /// ORIGINAL `(client, handle)`. `DUP_OBJECT` aliases, it does not copy
    /// (`vaspaceapiCopyConstruct` is `vaspaceIncRefCnt` + the same `pVASpace`, `vaspace_api.c:440`;
    /// `THE_ARCHITECTURE_v3.md` §4.3): nvidia-uvm dups a user's VA space into its own client
    /// (`nvUvmInterfaceDupAddressSpace`) and publishes the root through the DUP
    /// (`uvm_va_space.c:1394`), while the user's channels name the ORIGINAL — `[measured p6b3]`
    /// the root landed on `0xc1d00001:0xcaf00036` and the user's channel in `0xc1d0000b:0xcafe0010`
    /// was refused *"has no mirror"*. So a statement through an alias is carried for the original.
    aliases: std::collections::BTreeMap<(u32, u32), (u32, u32)>,
    /// ★ v3-gfx: originals whose own name was freed while a dup still referenced them — RM keeps
    /// the object (refcounted), so the mirror stays until the LAST alias goes. `[measured vgfx
    /// 2026-09-26]` the Vulkan UMD frees its probe client (the original) and keeps rendering in the
    /// dup.
    orphans: std::collections::BTreeSet<(u32, u32)>,
    /// Statements carried.
    pub carried: u64,
    /// ★ P5c: retirements carried.
    pub retired: u64,
}

impl PageDirPolicy {
    /// A link for one guest driver's wire.
    #[must_use]
    pub fn new(abi: kf_abi::versions::DriverAbiTable, guest_os: kf_abi::GuestOs, sink: MemSink) -> PageDirPolicy {
        PageDirPolicy { abi, guest_os, sink, held_last: false, vas: Default::default(), aliases: Default::default(), orphans: Default::default(), carried: 0, retired: 0 }
    }

    /// ★ P5c: observe a VA-space alloc (its parent device and whether it is only a reference to
    /// the device's default space).
    fn observe_alloc(&mut self, cmd: &RpcCommand) {
        let body = cmd.wire_body();
        let Ok(h) = self.abi.decode_rpc_alloc(body) else { return };
        if crate::chanlink::alloc_shape(&self.abi, h.class) != Some(kf_abi::versions::AllocParams::VaSpace) {
            return;
        }
        let device_ref = crate::rmrpc::alloc_params_window(&self.abi, body)
            .and_then(|p| self.abi.decode_vaspace_index(p))
            .is_some_and(|i| i == kf_abi::bringup::NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE);
        self.vas.insert((h.client, h.handle), (h.parent, device_ref));
    }

    /// ★ P6b: the VA-space object `(client, handle)` names — itself, or the original a dup aliases.
    #[must_use]
    pub fn canonical(&self, client: u32, handle: u32) -> (u32, u32) {
        self.aliases.get(&(client, handle)).copied().unwrap_or((client, handle))
    }

    /// ★ P6b: a `DUP_OBJECT` of a VA-space object we know becomes an alias of the original.
    fn observe_dup(&mut self, cmd: &RpcCommand) {
        let Ok(d) = self.abi.decode_dup(&cmd.payload) else { return };
        let src = self.canonical(d.src_client, d.src_handle);
        if self.vas.contains_key(&src) {
            self.aliases.insert((d.dst_client, d.dst_handle), src);
        }
    }

    /// ★ P5c: a free — retire every VA-space object it takes with it.
    fn observe_free(&mut self, cmd: &RpcCommand) {
        let Ok(f) = self.abi.decode_free(&cmd.payload) else { return };
        let (client, object) = (f.client, f.handle);
        // ★ P6b: an alias's free (or its client's) drops the NAME only — the object lives on
        // under its original handle, and only the original's free retires it. ⊘ A dup's parent
        // device is not tracked, so a device free that takes a dup with it leaves a stale name
        // until the client goes — a name that can only ever resolve to a live original.
        self.aliases.retain(|&(c, h), _| !(c == client && (object == client || h == object)));
        let dying: Vec<(u32, u32)> = self
            .vas
            .iter()
            .filter(|((c, v), (dev, device_ref))| {
                *c == client && (object == client || *dev == object || (*v == object && !*device_ref))
            })
            .map(|(k, _)| *k)
            .collect();
        for (c, v) in dying {
            self.vas.remove(&(c, v));
            // ★ v3-gfx: still referenced by a dup ⇒ the object lives on; retire it with its last alias.
            if self.aliases.values().any(|orig| *orig == (c, v)) {
                self.orphans.insert((c, v));
                continue;
            }
            (self.sink)(MemStatement::Retire { client: c, vaspace: v });
            self.retired += 1;
        }
        let gone: Vec<(u32, u32)> = self.orphans.iter().filter(|o| !self.aliases.values().any(|a| a == *o)).copied().collect();
        for (c, v) in gone {
            self.orphans.remove(&(c, v));
            (self.sink)(MemStatement::Retire { client: c, vaspace: v });
            self.retired += 1;
        }
    }
}

impl core::fmt::Debug for PageDirPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PageDirPolicy").field("carried", &self.carried).finish()
    }
}

/// `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`.
const SET_PAGE_DIRECTORY: u32 = 0x0080_1813;

impl CommandPolicy for PageDirPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        self.held_last = false;
        match cmd.function {
            // ★ P5c: observed, never answered — the object seat answers both.
            RpcFunction::RmAlloc => {
                self.observe_alloc(cmd);
                return None;
            }
            RpcFunction::Free => {
                self.observe_free(cmd);
                return None;
            }
            RpcFunction::DupObject => {
                self.observe_dup(cmd);
                return None;
            }
            RpcFunction::RmControl => {}
            _ => return None,
        }
        let Ok(crate::rmrpc::Translation::PageDir(mut st)) = crate::rmrpc::translate(&self.abi, self.guest_os, cmd) else {
            return None;
        };
        // ★ P6b: a root published through a dup is the ORIGINAL object's root.
        let (c, v) = self.canonical(st.client.0, st.vaspace.0);
        st.client = kf_arch::ids::HClient(c);
        st.vaspace = kf_arch::ids::HObject(v);
        (self.sink)(MemStatement::PageDir(st));
        self.carried += 1;
        self.held_last = true;
        let is_set = self.abi.decode_rpc_control(&cmd.payload).is_ok_and(|h| h.cmd == SET_PAGE_DIRECTORY);
        is_set.then(|| Reply { rpc_result: NV_OK, body: cmd.payload.clone() })
    }

    fn holds_for_refresh(&self, cmd: &RpcCommand) -> bool {
        cmd.function == RpcFunction::RmControl && self.held_last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn body(bar_type: u32, entry: u64, shift: u64) -> Vec<u8> {
        let mut b = vec![0u8; UPDATE_BAR_PDE_BODY_SIZE];
        b[0..4].copy_from_slice(&bar_type.to_le_bytes());
        b[4..8].copy_from_slice(&[0xAA; 4]); // the padding must not be read
        b[8..16].copy_from_slice(&entry.to_le_bytes());
        b[16..24].copy_from_slice(&shift.to_le_bytes());
        b
    }

    /// `[cap3 #159657]` the guest read `PDE3[0] = 0x2efbc302` through PRAMIN just before fn 70.
    #[test]
    fn the_measured_entry_decodes_whole_and_skips_the_padding() {
        let p = decode_update_bar_pde(&body(BAR_TYPE_2, 0x2_efbc_302, 47)).unwrap();
        assert_eq!(p, PublishedPde { bar: BarAperture::Bar2, entry: 0x2_efbc_302, level_shift: 47 });
        assert_eq!(decode_update_bar_pde(&[0; 23]), Err(BarPdeRefusal::ShortBody { len: 23 }));
        assert_eq!(decode_update_bar_pde(&body(2, 1, 1)), Err(BarPdeRefusal::UnknownBarType { bar_type: 2 }));
    }

    fn cmd(function: RpcFunction, payload: Vec<u8>) -> RpcCommand {
        RpcCommand { function, code: 0x46, sequence: 1, payload, elements: 1, delivered: Vec::new() }
    }

    #[test]
    fn an_accepted_entry_reaches_the_plane_and_its_reply_is_held() {
        let got: Arc<Mutex<Vec<MemStatement>>> = Arc::default();
        let g = got.clone();
        let mut p = BarPdePolicy::new(Arc::new(move |s| g.lock().unwrap().push(s)));
        let c = cmd(RpcFunction::UpdateBarPde, body(BAR_TYPE_2, 0x2_efbc_302, 47));
        let r = p.respond(&c).unwrap();
        assert_eq!((r.rpc_result, r.body.len()), (NV_OK, 0));
        assert!(p.holds_for_refresh(&c));
        assert_eq!(got.lock().unwrap().len(), 1);

        let bad = cmd(RpcFunction::UpdateBarPde, vec![0; 4]);
        assert_eq!(p.respond(&bad).unwrap().rpc_result, NV_ERR_INVALID_ARGUMENT);
        assert!(!p.holds_for_refresh(&bad), "a refusal holds nothing — a held reply nobody releases is a hang");
        assert_eq!(got.lock().unwrap().len(), 1);
        assert!(p.respond(&cmd(RpcFunction::RmAlloc, vec![])).is_none(), "declines everything else");
    }

    /// ★ v3-gfx: the original VA space's free does NOT retire its mirror while a dup still names
    /// it; the last alias's free does.
    #[test]
    fn an_original_freed_under_a_live_dup_retires_with_its_last_alias() {
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        let seen: Arc<Mutex<Vec<MemStatement>>> = Arc::default();
        let s2 = seen.clone();
        let mut p = PageDirPolicy::new(abi, kf_abi::GuestOs::Linux, Arc::new(move |st| s2.lock().unwrap().push(st)));
        let words = |w: &[u32]| w.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let (orig, alias) = ((0xc1d0_0016u32, 0xfade_0003u32), (0xc1d0_001au32, 0xbeef_0300u32));
        p.vas.insert(orig, (0xfade_0001, false));
        p.respond(&cmd(RpcFunction::DupObject, words(&[alias.0, 0xbeef_0003, alias.1, orig.0, orig.1, 0, 0])));
        assert_eq!(p.canonical(alias.0, alias.1), orig);
        p.respond(&cmd(RpcFunction::Free, words(&[orig.0, 0, orig.0, 0])));
        assert!(seen.lock().unwrap().is_empty(), "retired under a live dup: {:?}", seen.lock().unwrap());
        p.respond(&cmd(RpcFunction::Free, words(&[alias.0, 0xbeef_0003, alias.1, 0])));
        assert_eq!(seen.lock().unwrap().as_slice(), &[MemStatement::Retire { client: orig.0, vaspace: orig.1 }]);
    }
}
