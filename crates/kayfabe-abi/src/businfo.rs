//! `NV2080_CTRL_CMD_BUS_GET_INFO_V2` (`0x20801823`) — and ★★★ the first value this port
//! serves that **is not a fact about the chip at all**.
//!
//! ## ⊘⊘ The refutation this module exists to record
//!
//! `execution_plane_increments.md` §14.29 named the wall and then wrote down what it did
//! **not** know: *"`[unmeasured]` what `0x2d` must carry. `0x03003020` is what one part
//! answered."* Two things in that sentence are now measured, and both change the answer.
//!
//! ### 1. `0x03003020` was a mis-transcription; the value is `0x00302000`
//!
//! `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:46` carries the reply bytes
//! `2d000000 00203000`, i.e. index `0x2d`, data **`0x00302000`** little-endian.
//! `[measured 2026-08-08]` `rmladder --bus-info-sweep` (R22) replays that exact six-entry
//! request on a real GA106 and reproduces the whole reply — all six entries, byte for byte
//! — including `0x2d = 0x00302000`. ⚠ Worth naming rather than fixing quietly: the wrong
//! word was one byte-boundary off, and it **decodes plausibly** (`GEN=gen4`), which is
//! exactly why nobody caught it by reading it.
//!
//! ### 2. ★★★ It is LINK STATE, and that is MEASURED — not inferred from a field name
//!
//! `[measured 2026-08-08, vh, RTX 3060 `GPU-d0913685`, driver 580.159.04]` The **same
//! part**, minutes apart, answered two different words:
//!
//! | link | `current_link_speed` | `nvidia-smi gen.current` | `0x2d` |
//! |---|---|---|---|
//! | idle | 2.5 GT/s | 1 | `0x00302000` |
//! | under `pcie_link_load` | 8.0 GT/s | 3 | `0x00322000` |
//!
//! The delta is `0x0002_0000` — bits 19:16, `CURR_LEVEL`, `GEN1 → GEN3`. Sixteen reads at
//! each state, one distinct value in each.
//!
//! ⊘ **Sixteen identical reads on an idle box would have "confirmed" a constant.** The idle
//! sweep produced exactly that and it was worth nothing: an idle link is a constant link.
//! The measurement only exists because the link was made to move
//! (`scripts/rpctrace/pcie_link_load.c`).
//!
//! ### The word carries THREE generations, and only one of them is the die's
//!
//! Decoding `0x00302000` with `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_*`
//! (`ogkm-580: ctrl2080bus.h:355-390`), against `nvidia-smi` on the same box in the same
//! second:
//!
//! | field | bits | value | `nvidia-smi` | what it describes |
//! |---|---|---|---|---|
//! | `GPU_GEN` | 23:20 | `gen4` | `pcie.link.gen.gpumax = 4` | ★ **the die** — the only chip fact |
//! | `GEN` | 15:12 | `gen3` | `pcie.link.gen.max = 3` | the negotiated ceiling — **the slot** |
//! | `CURR_LEVEL` | 19:16 | `gen1`→`gen3` | `pcie.link.gen.current = 1`→`3` | **the live link** |
//!
//! ⇒ **A chip-family row cannot state this word.** Two of its three fields are properties of
//! the machine the card is plugged into and of what it is doing right now. Transcribing
//! `0x00302000` into a `GA106` row would claim that every GA106 everywhere sits in a Gen3
//! slot idling at 2.5 GT/s — wrong on a Gen4 board, and wrong on the **same box thirty
//! seconds later**. That is the `0x20802a08` mistake with a different id, and §14.29 stopped
//! rather than make it.
//!
//! ## ★ What this port serves instead, and the seam it leaves open
//!
//! One named field on the chip row — [`crate::businfo::PcieGen`], *the die's own maximum
//! generation*, which **is** a per-family fact (GA106 is a PCIe 4.0 part; `GPU_GEN = gen4`
//! is the measurement above) — and the word is **derived** from it by
//! [`PcieGenInfo::fully_trained`]: the emulated link is presented as trained at the die's own
//! generation, so `GPU_GEN == GEN == CURR_LEVEL`.
//!
//! ⊘ **This is a statement about the link THIS PORT presents, not a transcription of a
//! rented box's link.** It is the same on every host by construction, it is derived for
//! every architecture from one enum rather than tabulated per chip
//! (`derive_what_you_cannot_query_then_oracle_it`), and it cannot be wrong "on a different
//! slot, riser or bifurcation" because it is not describing one.
//!
//! ⚠ **The residual, named rather than elided:** the guest's DMA really does traverse the
//! *host's* link, and this reply does not describe that link. The truthful upgrade is to
//! read the host device's `current_link_speed` / `max_link_speed` — world-readable sysfs, no
//! privilege, no RM ioctl — and fold them into `GEN`/`CURR_LEVEL`. ⊘ It is not done here
//! because the **shipping archive has no host GPU binding at all**: `host-isolates` is off
//! by default (`kayfabe-qemu-raw/Cargo.toml:87`) and [`InitTablePolicy`] holds a
//! `&'static ChipProfile` and nothing else, so there is no host device to ask. The seam is
//! [`PcieGenInfo`]'s three independent fields: the day the device knows its host, two of
//! them get filled from it and this module's shape does not change.
//!
//! [`InitTablePolicy`]: https://docs.rs/kayfabe-device
//!
//! ## ⚠ The trap that fires here for the SECOND time — and it is `GPU_GET_INFO_V2`'s
//!
//! Of the six indices in libcuda's failing request — `0x0f` `BUS_NUMBER`, `0x10`
//! `DEVICE_NUMBER`, `0x2c` `DOMAIN_NUMBER`, `0x2d` `PCIE_GEN_INFO`, `0x03`
//! `PCIE_GPU_LINK_CAPS`, `0x06` `PCIE_DOWNSTREAM_LINK_CAPS` — **exactly one is forwarded**.
//! `getBusInfos`'s first `switch` sets `bSendRpc = IS_VIRTUAL(pGpu) || IS_GSP_CLIENT(pGpu)`
//! for `PCIE_GEN_INFO` and twelve siblings; everything else is answered from the guest's own
//! kernel state (`ogkm-580: kern_bus_ctrl.c:283-470`). A port that answered all six from the
//! ioctl trace would overwrite five values the guest had already computed.
//!
//! ### ⊘⊘ ★★★ CORRECTION 2026-08-09 — "or from its own config space" was WRONG, and the
//! error was load-bearing
//!
//! This paragraph used to end *"…from the guest's own kernel state **or from its own config
//! space**"*, and that half-sentence made `0x03 PCIE_GPU_LINK_CAPS` look like somebody
//! else's problem — a plane QEMU already emulates. It is not. `kbifGetGpuLinkCapabilities`
//! reads it with `GPU_BUS_CFG_RD32`, and on Maxwell and later that macro is
//! `GPU_REG_RD32(DEVICE_BASE(NV_PCFG) + index)` — **BAR0 + `0x88084`**, the register
//! aperture (`ogkm-580: kern_gpu_gm107.c:176-190`). ⊘ Nothing reads PCI configuration space.
//!
//! ⚠ The cost of the wrong half-sentence, `[measured 2026-08-09]`: `0x88084` was unclaimed
//! and read `0`, `MAX_SPEED = 0` is not a legal encoding, and `cuInit` returned 3 for four
//! days with the failing status visible nowhere but in `UVM_REGISTER_GPU`'s `params.rmStatus`.
//! ⇒ [`PcieLinkCaps`], and the answer lives in the chip's BAR0 register table, not here.
//! ★ A comment naming the plane a value comes from is a **claim**, and this one was never
//! checked against the macro it was describing.
//!
//! ★★★ **And unlike `GPU_GET_INFO_V2` there is no forward bit to key on — because there does
//! not need to be.** `kbusSendBusInfo_IMPL` forwards **one entry at a time**, in a *fresh*
//! `NV2080_CTRL_BUS_GET_INFO_V2_PARAMS` with `busInfoListSize = 1` and the entry copied into
//! slot 0 (`ogkm-580: kern_bus.c:1065-1101`). So the six-entry struct is the **ioctl**, and
//! what reaches a GSP is a **one-entry RPC per forwarded index**. Arriving here IS the
//! marker. ⇒ Every entry in a request this port sees is one the guest kernel decided it
//! could not answer, so every entry is filled — and an index with no derivation is refused
//! **by name**, never zero-filled.
//!
//! ⇒ [`BusInfoError::UnmeasuredIndex`], and the whole call, which is RM's own shape:
//! `getBusInfos` forwards under `NV_CHECK_OK_OR_RETURN` (`:333`) and returns the first
//! failure for the entire request.

/// `NV2080_CTRL_CMD_BUS_GET_INFO_V2` (`ogkm-580: ctrl2080bus.h:509`).
pub const NV2080_CTRL_CMD_BUS_GET_INFO_V2: u32 = 0x2080_1823;

/// `NV2080_CTRL_BUS_INFO_MAX_LIST_SIZE` — 52, and like `GPU_INFO`'s it is **both** the array
/// length and the exclusive upper bound on a legal index
/// (`ogkm-580: ctrl2080bus.h:341`, `INDEX_MAX = SYSMEM_CONNECTION_TYPE = 0x33`).
pub const BUS_INFO_MAX_LIST_SIZE: usize = 0x34;

/// `sizeof(NV2080_CTRL_BUS_GET_INFO_V2_PARAMS)` = `4 + 8 * 52` = 420.
///
/// `[measured]` on the wire as `size=420` in both `BUS_GET_INFO_V2` ioctls of
/// `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:44,46`.
pub const BUS_GET_INFO_V2_PARAMS_SIZE: usize = 4 + 8 * BUS_INFO_MAX_LIST_SIZE;

/// `NV2080_CTRL_BUS_INFO_INDEX_PCIE_GEN_INFO` (`ogkm-580: ctrl2080bus.h:329`) — the one
/// index of the six that is RPC-forwarded on a GSP client.
pub const BUS_INFO_INDEX_PCIE_GEN_INFO: u32 = 0x2d;

/// A PCI Express generation as `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_GEN_*` encodes it.
///
/// ⚠ **The encoding is off by one from the name and that is the whole hazard**:
/// `..._GEN_GEN1` is `0`, so a field left at its default decodes to *"Gen 1"* rather than to
/// *"unstated"* — the `two_encodings_agreeing_on_the_first_values` shape. There is no
/// numeric sentinel available, so this is an enum with no zero-valued *absent* variant:
/// a row that has not been stated cannot compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PcieGen {
    /// `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_GEN_GEN1` = 0.
    Gen1,
    /// `..._GEN2` = 1.
    Gen2,
    /// `..._GEN3` = 2.
    Gen3,
    /// `..._GEN4` = 3. ★ GA106's `GPU_GEN`, `[measured]`.
    Gen4,
    /// `..._GEN5` = 4.
    Gen5,
    /// `..._GEN6` = 5.
    Gen6,
}

impl PcieGen {
    /// The four-bit field value (`GEN1 == 0`).
    #[must_use]
    pub const fn field(self) -> u32 {
        match self {
            Self::Gen1 => 0,
            Self::Gen2 => 1,
            Self::Gen3 => 2,
            Self::Gen4 => 3,
            Self::Gen5 => 4,
            Self::Gen6 => 5,
        }
    }

    /// The generation number as a human writes it (`Gen4 -> 4`).
    ///
    /// ★★ **Not only for messages** — see [`Self::max_speed_field`], where this *is* the
    /// wire encoding of a different field of the very same 32-bit word.
    #[must_use]
    pub const fn number(self) -> u32 {
        self.field() + 1
    }

    /// The same generation in `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_MAX_SPEED`'s encoding
    /// (`ogkm-580: ctrl2080bus.h:357-363`) — `_2500MBPS = 1` … `_64000MBPS = 6`.
    ///
    /// ★★★ **One word, two encodings of "which PCIe generation", off by one from each
    /// other.** `..._GEN_GEN1` is `0` while `..._MAX_SPEED_2500MBPS` is `1`, and both sets of
    /// fields live in the *same* `NV_XVE_LINK_CAPABILITIES` word: `GEN` 15:12, `CURR_LEVEL`
    /// 19:16 and `GPU_GEN` 23:20 use the zero-based one, `MAX_SPEED` 3:0 uses the one-based
    /// one. ⊘ Never reach for [`Self::field`] when filling `MAX_SPEED`: `Gen4` would go out
    /// as `3` = 8 GT/s — a *legal* encoding, so nothing would refuse it and the link would
    /// simply be understated by one generation forever.
    ///
    /// ★ The mirror hazard is worse and is why this returns a distinct method rather than a
    /// documented convention: writing [`Self::field`] into `MAX_SPEED` for `Gen1` yields
    /// **0**, which is not a legal `MAX_SPEED` at all — `calculatePCIELinkRateMBps`'s
    /// `default` arm (`ogkm-580: nv_gpu_ops.c:2077-2079`) answers `NV_ERR_INVALID_STATE` and
    /// prints *"Unknown PCIe speed"*. `[measured 2026-08-09, boot lc1446 @ 69f8817]` that is
    /// exactly what an unserved (zero) register did, and it is what stopped `cuInit`.
    #[must_use]
    pub const fn max_speed_field(self) -> u32 {
        self.number()
    }

    /// Decode a four-bit field. `None` for `6..=15`, which the header does not define.
    #[must_use]
    pub fn from_field(v: u32) -> Option<Self> {
        Some(match v {
            0 => Self::Gen1,
            1 => Self::Gen2,
            2 => Self::Gen3,
            3 => Self::Gen4,
            4 => Self::Gen5,
            5 => Self::Gen6,
            _ => return None,
        })
    }
}

/// `PCIE_GEN_INFO`'s three generations, as three fields rather than one `u32`.
///
/// ★ The split **is** the design. One `u32` on a chip row would be a per-machine reading
/// wearing a chip-family label; three named fields make it impossible to state the slot's
/// or the link's generation as though it were the die's, and they are the seam through which
/// two of the three later get filled from a real host device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcieGenInfo {
    /// `GPU_GEN` 23:20 — ★ **the die**. `[measured]` `gen4` on GA106, agreeing with
    /// `nvidia-smi pcie.link.gen.gpumax = 4`. The one field a chip family may state.
    pub gpu_gen: PcieGen,
    /// `GEN` 15:12 — the negotiated ceiling, `min(die, slot)`. `[measured]` `gen3` on the
    /// bench, whose root port advertises 8 GT/s, while the die is `gen4`. **The slot's.**
    pub negotiated_gen: PcieGen,
    /// `CURR_LEVEL` 19:16 — the **live** link speed. `[measured]` to move `gen1 -> gen3` on
    /// one part under load and back again. Nothing may cache this as a constant.
    pub current_gen: PcieGen,
}

/// `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_GEN` 15:12.
const GEN_SHIFT: u32 = 12;
/// `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_CURR_LEVEL` 19:16.
const CURR_LEVEL_SHIFT: u32 = 16;
/// `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_GPU_GEN` 23:20.
const GPU_GEN_SHIFT: u32 = 20;
/// Every one of the three is four bits wide.
const GEN_FIELD_MASK: u32 = 0xf;

impl PcieGenInfo {
    /// The link this port presents: trained at the die's own generation, so all three
    /// fields agree.
    ///
    /// ⊘ Deliberately **not** "the values a real GA106 answered". Those were `gen4 / gen3 /
    /// gen1` — two of them facts about one rented slot at one instant. This says something
    /// weaker and true everywhere: *the emulated link runs at the generation the emulated
    /// die supports.*
    #[must_use]
    pub fn fully_trained(gpu_gen: PcieGen) -> Self {
        Self {
            gpu_gen,
            negotiated_gen: gpu_gen,
            current_gen: gpu_gen,
        }
    }

    /// Pack the three fields. Every other bit is zero, which is what a real GA106 returns:
    /// `[measured]` `0x00302000` and `0x00322000` both have `MAX_SPEED`, `MAX_WIDTH`, `ASPM`
    /// and `SPEED_CHANGES` clear — `PCIE_GEN_INFO` populates the generation fields only,
    /// even though it borrows `LINK_CAP`'s layout.
    #[must_use]
    pub fn encode(self) -> u32 {
        (self.negotiated_gen.field() << GEN_SHIFT)
            | (self.current_gen.field() << CURR_LEVEL_SHIFT)
            | (self.gpu_gen.field() << GPU_GEN_SHIFT)
    }

    /// Unpack a measured word — the inverse of [`Self::encode`], and how the two committed
    /// real-GA106 readings are checked against the field layout.
    ///
    /// `None` if any of the three fields names a generation the header does not define.
    #[must_use]
    pub fn decode(word: u32) -> Option<Self> {
        Some(Self {
            negotiated_gen: PcieGen::from_field((word >> GEN_SHIFT) & GEN_FIELD_MASK)?,
            current_gen: PcieGen::from_field((word >> CURR_LEVEL_SHIFT) & GEN_FIELD_MASK)?,
            gpu_gen: PcieGen::from_field((word >> GPU_GEN_SHIFT) & GEN_FIELD_MASK)?,
        })
    }
}

/// `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_MAX_SPEED` 3:0 (`ogkm-580: ctrl2080bus.h:357`).
const MAX_SPEED_SHIFT: u32 = 0;
/// `NV2080_CTRL_BUS_INFO_PCIE_LINK_CAP_MAX_WIDTH` 9:4 (`ogkm-580: ctrl2080bus.h:364`) —
/// six bits, so 16 lanes fits and 64 would not.
const MAX_WIDTH_SHIFT: u32 = 4;
/// See [`MAX_SPEED_SHIFT`].
const MAX_SPEED_MASK: u32 = 0xf;
/// See [`MAX_WIDTH_SHIFT`].
const MAX_WIDTH_MASK: u32 = 0x3f;

/// The lane count the emulated link is presented with.
///
/// ⊘ Deliberately **one constant here, not a chip-row field.** A row would invite a
/// per-family transcription of whatever width the rented card happened to train at, and this
/// is not describing a card: it is a property of the link *this port presents*, which is the
/// same on every host by construction — the same argument [`PcieGenInfo::fully_trained`]
/// already makes for the generation. x16 because that is the widest a PCIe endpoint
/// negotiates and because a narrower claim would understate the bandwidth RM derives from it
/// for no gain. `[measured]` a real GA106 answered `MAX_WIDTH = 16` through this same field
/// (`traces/real_ga106/rmladder_r22_businfo_loaded_real_ga106.txt`, index `0x03` =
/// `0x00454d03`), so the presented link is not wider than the part being emulated.
pub const PRESENTED_LINK_WIDTH: u32 = 16;

/// `NV_XVE_LINK_CAPABILITIES` — the PCI Express *Link Capabilities* word, as the guest
/// kernel reads it out of the register aperture.
///
/// ## ★★★ Why this is a register and not a control, and why that matters
///
/// `NV2080_CTRL_BUS_INFO_INDEX_PCIE_GPU_LINK_CAPS` (index `0x03`) looks like a sibling of
/// [`BUS_INFO_INDEX_PCIE_GEN_INFO`] (`0x2d`) and is not one. `getBusInfos`'s `bSendRpc`
/// switch (`ogkm-580: kern_bus_ctrl.c:296-330`) names thirteen indices that a GSP client
/// forwards, and `0x03` is **not among them**. It is answered inside the guest, by
/// `kbifControlGetPCIEInfo_IMPL` -> `kbifGetGpuLinkCapabilities`
/// (`ogkm-580: kernel_bif.c:1063-1076, 879-903`), which does
/// `GPU_BUS_CFG_RD32(pGpu, NV_XVE_LINK_CAPABILITIES)` — and on Maxwell and later that macro
/// reads **the register aperture**, `DEVICE_BASE(NV_PCFG) + 0x84` = BAR0 + `0x88084`
/// (`ogkm-580: kern_gpu_gm107.c:176-190`; `dev_nv_xve.h:104`;
/// `dev_nv_pcfg_xve_regmap.h:27`), ⊘ **not** PCI configuration space.
///
/// ⇒ Serving it correctly over the RPC plane is impossible; nothing is ever asked. The word
/// has to be present in BAR0 or the guest computes its own answer from zeros.
///
/// ## ★★★ What that costs, `[measured 2026-08-09]`
///
/// Boot `lc1446` @ `69f8817`: BAR0 + `0x88084` read **`0x00000000`** (unclaimed; the whole
/// `NV_PCFG` window is). `MAX_SPEED` = 0 is not one of the six legal encodings, so
/// `calculatePCIELinkRateMBps` took its `default` arm, printed *"Unknown PCIe speed"* and
/// returned `NV_ERR_INVALID_STATE` (`ogkm-580: nv_gpu_ops.c:2077-2079`). That status
/// propagates out of `getPCIELinkRateMBps` -> `nvGpuOpsGetGpuInfo` (`:7220`) ->
/// `nvUvmInterfaceGetGpuInfo` -> `uvm_gpu_retain_by_uuid`, and lands in
/// `UVM_REGISTER_GPU`'s `params.rmStatus` as **`0x40`** — which boot `us1445` read directly
/// (`scripts/bench/uvm_ioctl_trace.c`). libcuda then tore its context down and `cuInit`
/// returned 3.
///
/// ⚠ **UVM prints nothing on this path**, and the ioctl returns 0 at the syscall boundary.
/// Neither `strace` nor `dmesg` names it; only reading `params.rmStatus` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcieLinkCaps {
    /// `MAX_SPEED` 3:0, held as a generation and encoded by [`PcieGen::max_speed_field`].
    pub max_gen: PcieGen,
    /// `MAX_WIDTH` 9:4, in lanes.
    pub max_width: u32,
}

impl PcieLinkCaps {
    /// The link this port presents: the die's own maximum generation, at
    /// [`PRESENTED_LINK_WIDTH`] lanes.
    ///
    /// ⊘ Deliberately **not** the word a real GA106 answered. That was `0x00454d03`, whose
    /// `MAX_SPEED` is `3` = 8 GT/s — one generation *below* the `gen4` die, because an
    /// NVIDIA endpoint advertises the capability it trained to in the slot it is in. A
    /// chip-family row stating `0x00454d03` would claim every GA106 everywhere sits in a
    /// Gen3 slot; this says the weaker true thing instead, exactly as
    /// [`PcieGenInfo::fully_trained`] does for `PCIE_GEN_INFO`.
    #[must_use]
    pub const fn fully_trained(max_gen: PcieGen) -> Self {
        Self {
            max_gen,
            max_width: PRESENTED_LINK_WIDTH,
        }
    }

    /// Pack the word. Every other bit — ASPM support, L0s/L1 exit latency, port number — is
    /// left zero: they are optional-capability advertisements, and RM reads only these two
    /// fields out of this register (`ogkm-580: nv_gpu_ops.c:2110-2113`).
    #[must_use]
    pub const fn encode(self) -> u32 {
        ((self.max_gen.max_speed_field() & MAX_SPEED_MASK) << MAX_SPEED_SHIFT)
            | ((self.max_width & MAX_WIDTH_MASK) << MAX_WIDTH_SHIFT)
    }

    /// Unpack a word — the inverse of [`Self::encode`], and how a measured reading is
    /// checked against the field layout.
    ///
    /// `None` when `MAX_SPEED` is not one of the six encodings the header defines, which is
    /// precisely the condition `calculatePCIELinkRateMBps` refuses. ★ So a `None` here and
    /// an `NV_ERR_INVALID_STATE` there are the same predicate, written twice on purpose.
    #[must_use]
    pub fn decode(word: u32) -> Option<Self> {
        let speed = (word >> MAX_SPEED_SHIFT) & MAX_SPEED_MASK;
        Some(Self {
            // `MAX_SPEED` is one-based; `PcieGen::from_field` is zero-based.
            max_gen: PcieGen::from_field(speed.checked_sub(1)?)?,
            max_width: (word >> MAX_WIDTH_SHIFT) & MAX_WIDTH_MASK,
        })
    }
}

/// Why a `BUS_GET_INFO_V2` request could not be answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusInfoError {
    /// `busInfoListSize` is zero or larger than the array it indexes. RM applies the same
    /// bound, both halves of it (`ogkm-580: bus_ctrl.c`'s caller checks
    /// `busInfoListSize > NV2080_CTRL_BUS_INFO_MAX_LIST_SIZE`), and a guest-supplied count
    /// used as a loop bound over a buffer is checked before it is used, never after.
    ListSize {
        /// What the guest declared.
        asked: u32,
        /// [`BUS_INFO_MAX_LIST_SIZE`].
        max: usize,
    },
    /// The params buffer is shorter than [`BUS_GET_INFO_V2_PARAMS_SIZE`].
    ShortParams {
        /// What arrived.
        len: usize,
        /// What the struct is.
        need: usize,
    },
    /// An index at or past [`BUS_INFO_MAX_LIST_SIZE`].
    IndexOutOfRange {
        /// The index the guest asked for.
        index: u32,
        /// [`BUS_INFO_MAX_LIST_SIZE`].
        max: usize,
    },
    /// ★★★ The guest kernel forwarded an index this port has **no derivation for**, and it
    /// is refused by name rather than filled in.
    ///
    /// ⊘ Zero is not available as a fallback here: `PCIE_LINK_CAP_GEN_GEN1 == 0`, so a
    /// zero-filled entry is not *"unknown"* — it is the positive claim *"Gen 1"*.
    UnmeasuredIndex {
        /// The forwarded index.
        index: u32,
    },
}

impl core::fmt::Display for BusInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ListSize { asked, max } => write!(
                f,
                "busInfoListSize {asked} is not in 1..={max} — the guest's own count is not \
                 a bound this port may take on trust"
            ),
            Self::ShortParams { len, need } => {
                write!(f, "{len}-byte params, {need} needed for the struct")
            }
            Self::IndexOutOfRange { index, max } => {
                write!(f, "bus info index {index:#x} is not below {max:#x}")
            }
            Self::UnmeasuredIndex { index } => write!(
                f,
                "bus info index {index:#x} was forwarded to physical RM and this port has no \
                 derivation for it; refused by name rather than answered zero, which on this \
                 control would read as a positive claim of Gen 1"
            ),
        }
    }
}

impl core::error::Error for BusInfoError {}

/// Answer a `BUS_GET_INFO_V2` RPC: **the request, edited**.
///
/// Every entry the request declares is filled from `answers`; the tail past
/// `busInfoListSize` is left exactly as it arrived, because real RM returns it untouched:
/// `[measured 2026-08-08, real GA106 `GPU-d0913685`, `rmladder --bus-info-sweep` R22,
/// `traces/real_ga106/rmladder_r22_businfo_sweep_real_ga106.txt`]` all 52 indices come back
/// `tail=untouched` with the 408 bytes past the declared entry seeded `0xCD`.
///
/// ⊘ **Every declared entry is filled**, with no forward-bit test, because
/// `kbusSendBusInfo_IMPL` only ever puts entries the guest kernel could not answer into an
/// RPC (`ogkm-580: kern_bus.c:1065-1101`). Arriving here is the marker.
///
/// # Errors
///
/// Every variant of [`BusInfoError`]. [`BusInfoError::UnmeasuredIndex`] refuses the **whole**
/// call rather than the one entry, which is RM's shape: `getBusInfos` forwards under
/// `NV_CHECK_OK_OR_RETURN` and returns the first failure for the entire request
/// (`ogkm-580: kern_bus_ctrl.c:333`).
pub fn answer_bus_get_info_v2(
    request: &[u8],
    answers: &[(u32, u32)],
) -> Result<Vec<u8>, BusInfoError> {
    let Some(body) = request.get(..BUS_GET_INFO_V2_PARAMS_SIZE) else {
        return Err(BusInfoError::ShortParams {
            len: request.len(),
            need: BUS_GET_INFO_V2_PARAMS_SIZE,
        });
    };
    let mut out = body.to_vec();

    let count = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
    if count == 0 || count as usize > BUS_INFO_MAX_LIST_SIZE {
        return Err(BusInfoError::ListSize {
            asked: count,
            max: BUS_INFO_MAX_LIST_SIZE,
        });
    }

    for i in 0..count as usize {
        // In range by construction: `count <= 52` and the buffer is `4 + 8 * 52` long.
        let at = 4 + 8 * i;
        let index = u32::from_le_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]);
        if index as usize >= BUS_INFO_MAX_LIST_SIZE {
            return Err(BusInfoError::IndexOutOfRange {
                index,
                max: BUS_INFO_MAX_LIST_SIZE,
            });
        }
        let Some(&(_, value)) = answers.iter().find(|&&(idx, _)| idx == index) else {
            return Err(BusInfoError::UnmeasuredIndex { index });
        };
        out[at + 4..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

/// Read the `(index, data)` pairs back out of a params buffer — for tests and for the trace
/// differential.
///
/// # Errors
///
/// [`BusInfoError::ShortParams`] or [`BusInfoError::ListSize`].
pub fn decode_bus_info_pairs(params: &[u8]) -> Result<Vec<(u32, u32)>, BusInfoError> {
    let Some(body) = params.get(..BUS_GET_INFO_V2_PARAMS_SIZE) else {
        return Err(BusInfoError::ShortParams {
            len: params.len(),
            need: BUS_GET_INFO_V2_PARAMS_SIZE,
        });
    };
    let count = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    if count == 0 || count as usize > BUS_INFO_MAX_LIST_SIZE {
        return Err(BusInfoError::ListSize {
            asked: count,
            max: BUS_INFO_MAX_LIST_SIZE,
        });
    }
    Ok((0..count as usize)
        .map(|i| {
            let at = 4 + 8 * i;
            (
                u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]),
                u32::from_le_bytes([body[at + 4], body[at + 5], body[at + 6], body[at + 7]]),
            )
        })
        .collect())
}

/// Build a request the way `kbusSendBusInfo` builds one — for tests only.
///
/// `entries` are `(index, data)` pairs written verbatim, so a test can replay libcuda's own
/// buffer with its stale `data` words intact.
#[must_use]
pub fn build_request(entries: &[(u32, u32)]) -> Vec<u8> {
    let mut p = vec![0u8; BUS_GET_INFO_V2_PARAMS_SIZE];
    p[0..4].copy_from_slice(&(u32::try_from(entries.len()).unwrap_or(u32::MAX)).to_le_bytes());
    for (i, &(index, data)) in entries.iter().enumerate().take(BUS_INFO_MAX_LIST_SIZE) {
        let at = 4 + 8 * i;
        p[at..at + 4].copy_from_slice(&index.to_le_bytes());
        p[at + 4..at + 8].copy_from_slice(&data.to_le_bytes());
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[measured 2026-08-08, real GA106 `GPU-d0913685`, driver 580.159.04]` — the idle
    /// reading, from `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:46` AND from
    /// `rmladder --bus-info-sweep` replaying that request, which agree byte for byte.
    const MEASURED_IDLE: u32 = 0x0030_2000;
    /// The same part, seconds later, with `pcie_link_load` on the link.
    const MEASURED_LOADED: u32 = 0x0032_2000;

    /// The struct's shape, stated so a mistyped constant fails here rather than on a boot.
    #[test]
    fn the_params_struct_is_the_one_on_the_wire() {
        assert_eq!(BUS_GET_INFO_V2_PARAMS_SIZE, 420);
        assert_eq!(BUS_INFO_MAX_LIST_SIZE, 0x34);
        assert_eq!(NV2080_CTRL_CMD_BUS_GET_INFO_V2, 0x2080_1823);
        assert_eq!(BUS_INFO_INDEX_PCIE_GEN_INFO, 0x2d);
        // The three fields must not overlap and must all be four bits.
        assert_eq!(GEN_SHIFT + 4, CURR_LEVEL_SHIFT);
        assert_eq!(CURR_LEVEL_SHIFT + 4, GPU_GEN_SHIFT);
    }

    /// ⚠ `GEN1 == 0`, so a zero word is a positive claim of Gen 1 rather than an absence.
    /// This is the whole reason `PcieGen` is an enum and the refusal is by name.
    #[test]
    fn a_zero_word_decodes_to_gen1_everywhere_and_not_to_unstated() {
        let z = PcieGenInfo::decode(0).expect("all three fields are legal");
        assert_eq!(z.gpu_gen, PcieGen::Gen1);
        assert_eq!(z.negotiated_gen, PcieGen::Gen1);
        assert_eq!(z.current_gen, PcieGen::Gen1);
        assert_eq!(PcieGen::Gen1.field(), 0);
        assert_eq!(PcieGen::Gen1.number(), 1);
        // …and the undefined encodings are refused rather than folded into Gen6.
        for v in 6..16 {
            assert_eq!(
                PcieGen::from_field(v),
                None,
                "field {v} names no generation"
            );
        }
    }

    /// ★★★ THE MEASUREMENT. The same physical part answered two different words, and the
    /// difference is exactly `CURR_LEVEL`. This is the test that makes "no chip row may
    /// state it" a fact rather than an opinion.
    #[test]
    fn one_part_answered_two_words_and_only_the_live_link_field_moved() {
        let idle = PcieGenInfo::decode(MEASURED_IDLE).expect("legal");
        let loaded = PcieGenInfo::decode(MEASURED_LOADED).expect("legal");

        // The die does not change when the link trains up.
        assert_eq!(idle.gpu_gen, PcieGen::Gen4);
        assert_eq!(loaded.gpu_gen, PcieGen::Gen4);
        // Nor does the slot's ceiling.
        assert_eq!(idle.negotiated_gen, PcieGen::Gen3);
        assert_eq!(loaded.negotiated_gen, PcieGen::Gen3);
        // ★ The live link does.
        assert_eq!(idle.current_gen, PcieGen::Gen1);
        assert_eq!(loaded.current_gen, PcieGen::Gen3);
        assert_ne!(MEASURED_IDLE, MEASURED_LOADED);
        assert_eq!(MEASURED_LOADED - MEASURED_IDLE, 2 << CURR_LEVEL_SHIFT);

        // ⊘ And the die's own generation is NOT the slot's, on the very box the reading
        // came from — so even the two "static-looking" fields disagree with each other.
        assert_ne!(idle.gpu_gen, idle.negotiated_gen);

        // The decode is a real inverse: both words re-encode to themselves.
        assert_eq!(idle.encode(), MEASURED_IDLE);
        assert_eq!(loaded.encode(), MEASURED_LOADED);
    }

    /// What this port serves: derived from one die fact, all three fields agreeing.
    #[test]
    fn the_served_word_is_derived_from_one_field_and_is_not_the_measured_word() {
        let served = PcieGenInfo::fully_trained(PcieGen::Gen4);
        assert_eq!(served.gpu_gen, PcieGen::Gen4);
        assert_eq!(served.negotiated_gen, PcieGen::Gen4);
        assert_eq!(served.current_gen, PcieGen::Gen4);
        assert_eq!(served.encode(), 0x0033_3000);
        // ⊘ It is deliberately NOT either measured word: those describe one rented slot at
        // one instant, and this describes the link this port presents.
        assert_ne!(served.encode(), MEASURED_IDLE);
        assert_ne!(served.encode(), MEASURED_LOADED);
        // ★ …and the derivation moves with the die, for every architecture, from one enum.
        assert_eq!(
            PcieGenInfo::fully_trained(PcieGen::Gen3).encode(),
            0x0022_2000,
            "a Turing part derives its own word from the same one line"
        );
        assert_eq!(
            PcieGenInfo::fully_trained(PcieGen::Gen5).encode(),
            0x0044_4000
        );
    }

    /// ★★★ The RPC this port actually answers: ONE entry, because `kbusSendBusInfo` sends
    /// one at a time. The six-entry struct in the trace is the ioctl, not the wire.
    #[test]
    fn the_single_entry_rpc_is_answered_and_the_reply_is_the_request_edited() {
        let served = PcieGenInfo::fully_trained(PcieGen::Gen4).encode();
        let answers = [(BUS_INFO_INDEX_PCIE_GEN_INFO, served)];
        // `kbusSendBusInfo` zeroes its params and copies the entry into slot 0; the guest's
        // `data` is whatever was in the caller's entry, so seed it with something a reply
        // could not have produced.
        let mut req = build_request(&[(BUS_INFO_INDEX_PCIE_GEN_INFO, 0xdead_beef)]);
        for b in &mut req[12..] {
            *b = 0xCD;
        }
        let rep = answer_bus_get_info_v2(&req, &answers).expect("served");
        assert_eq!(
            decode_bus_info_pairs(&rep).expect("well formed"),
            [(BUS_INFO_INDEX_PCIE_GEN_INFO, served)]
        );
        assert!(
            rep[12..].iter().all(|&b| b == 0xCD),
            "the reply must be the request EDITED — real GSP returns the tail verbatim"
        );
        assert_eq!(rep.len(), BUS_GET_INFO_V2_PARAMS_SIZE);
    }

    /// ⊘ The five kernel-answered indices are refused BY NAME if they ever arrive, rather
    /// than being answered from the trace. `[measured]` values for them exist
    /// (`0x0f=0 0x10=7 0x2c=0 0x03=0x00453d03 0x06=0`) and every one is a fact about one
    /// rented machine's bus topology.
    #[test]
    fn a_kernel_answered_index_is_refused_by_name_not_transcribed_from_the_trace() {
        let answers = [(
            BUS_INFO_INDEX_PCIE_GEN_INFO,
            PcieGenInfo::fully_trained(PcieGen::Gen4).encode(),
        )];
        for index in [0x0f, 0x10, 0x2c, 0x03, 0x06] {
            assert_eq!(
                answer_bus_get_info_v2(&build_request(&[(index, 0)]), &answers),
                Err(BusInfoError::UnmeasuredIndex { index }),
                "index {index:#x} must be refused, not filled from the ioctl trace"
            );
        }
        // …and the whole call fails on the first such entry even when a servable one
        // precedes it, which is `NV_CHECK_OK_OR_RETURN`'s shape.
        assert_eq!(
            answer_bus_get_info_v2(
                &build_request(&[(BUS_INFO_INDEX_PCIE_GEN_INFO, 0), (0x0f, 0)]),
                &answers
            ),
            Err(BusInfoError::UnmeasuredIndex { index: 0x0f })
        );
    }

    /// The guest's count is a loop bound over a buffer and is never taken on trust.
    #[test]
    fn a_hostile_or_absurd_list_size_is_refused_before_it_indexes_anything() {
        let answers = [(BUS_INFO_INDEX_PCIE_GEN_INFO, 0)];
        for bad in [0u32, 0x35, 0xffff, u32::MAX] {
            let mut p = vec![0u8; BUS_GET_INFO_V2_PARAMS_SIZE];
            p[0..4].copy_from_slice(&bad.to_le_bytes());
            assert_eq!(
                answer_bus_get_info_v2(&p, &answers),
                Err(BusInfoError::ListSize {
                    asked: bad,
                    max: BUS_INFO_MAX_LIST_SIZE
                })
            );
            assert!(decode_bus_info_pairs(&p).is_err());
        }
        // …and the largest LEGAL count is accepted, so the bound is not off by one in the
        // direction that refuses a legitimate request.
        let all: Vec<(u32, u32)> = (0..BUS_INFO_MAX_LIST_SIZE as u32).map(|i| (i, 0)).collect();
        let table: Vec<(u32, u32)> = all.clone();
        let rep = answer_bus_get_info_v2(&build_request(&all), &table).expect("52 entries legal");
        assert_eq!(
            decode_bus_info_pairs(&rep).expect("well formed").len(),
            0x34
        );
    }

    /// An index past the array is named before it is looked up.
    #[test]
    fn an_out_of_range_index_is_refused_as_out_of_range_and_not_as_unmeasured() {
        assert_eq!(
            answer_bus_get_info_v2(&build_request(&[(0x34, 0)]), &[(0x34, 7)]),
            Err(BusInfoError::IndexOutOfRange {
                index: 0x34,
                max: BUS_INFO_MAX_LIST_SIZE
            }),
            "the bound comes first — a table row for an illegal index must not rescue it"
        );
    }

    /// `[measured 2026-08-08, real GA106]` `PCIE_GPU_LINK_CAPS` (index `0x03`) on the bench
    /// part, from `traces/real_ga106/rmladder_r22_businfo_loaded_real_ga106.txt`.
    ///
    /// ⊘ Present as an ORACLE FOR THE FIELD LAYOUT, never as the value to serve — its
    /// `MAX_SPEED` is `Gen3`, the *slot's* ceiling, on a `Gen4` die.
    const MEASURED_GPU_LINK_CAPS: u32 = 0x0045_4d03;

    /// ★★★ The layout, checked against real silicon: the one measured word must decode to
    /// the two things `nvidia-smi` and the part's own datasheet say — 8 GT/s over 16 lanes.
    ///
    /// This is what makes the shifts a fact rather than a transcription. A one-bit error in
    /// `MAX_WIDTH_SHIFT` still decodes *plausibly* (`two_encodings_agreeing_on_the_first_values`
    /// is exactly how the `0x03003020` mis-transcription survived review), so the assertion
    /// is on the decoded values and not on the word.
    #[test]
    fn the_measured_link_caps_word_decodes_to_the_parts_real_link() {
        let caps = PcieLinkCaps::decode(MEASURED_GPU_LINK_CAPS).expect("a legal MAX_SPEED");
        assert_eq!(
            caps.max_gen,
            PcieGen::Gen3,
            "MAX_SPEED 3:0 = 3 is _8000MBPS, and the bench root port is a Gen3 slot"
        );
        assert_eq!(caps.max_width, 16, "MAX_WIDTH 9:4 = 16 lanes");
        // ⊘ And the DIE is Gen4 — so this word is NOT the die's generation, which is the
        // whole reason `fully_trained` does not copy it.
        assert_ne!(caps.max_gen, PcieGen::Gen4);
    }

    /// ★★★ THE BITE. `MAX_SPEED` is ONE-based while `GEN`/`CURR_LEVEL`/`GPU_GEN` in the same
    /// word are ZERO-based, and the two live one method call apart.
    ///
    /// `[measured 2026-08-09, boot lc1446]` a `MAX_SPEED` of 0 is what an unserved register
    /// produced, and it cost `cuInit`: `calculatePCIELinkRateMBps`'s `default` arm answers
    /// `NV_ERR_INVALID_STATE` for anything outside `1..=6`. So this asserts the served field
    /// against the SIX legal encodings by name, not against a remembered number.
    #[test]
    fn every_generation_encodes_to_a_speed_calculate_pcie_link_rate_accepts() {
        for g in [
            PcieGen::Gen1,
            PcieGen::Gen2,
            PcieGen::Gen3,
            PcieGen::Gen4,
            PcieGen::Gen5,
            PcieGen::Gen6,
        ] {
            let word = PcieLinkCaps::fully_trained(g).encode();
            let speed = word & MAX_SPEED_MASK;
            assert!(
                (1..=6).contains(&speed),
                "{g:?} encoded MAX_SPEED={speed}, which calculatePCIELinkRateMBps refuses \
                 (ogkm-580: nv_gpu_ops.c:2077-2079)"
            );
            // ⊘ The off-by-one, stated as an assertion: using `field()` here would put Gen1
            // out as 0 — the exact value that stopped `cuInit`.
            assert_eq!(speed, g.number());
            assert_ne!(speed, g.field(), "the two encodings never coincide");
            // Round-trips, and the width survives.
            let back = PcieLinkCaps::decode(word).expect("we just built it legal");
            assert_eq!(back.max_gen, g);
            assert_eq!(back.max_width, PRESENTED_LINK_WIDTH);
        }
    }

    /// The condition RM refuses, refused here by the same predicate.
    #[test]
    fn a_zero_link_caps_word_is_refused_rather_than_read_as_gen1() {
        assert_eq!(
            PcieLinkCaps::decode(0),
            None,
            "MAX_SPEED=0 names no PCIe generation; RM answers NV_ERR_INVALID_STATE for it, \
             and a decoder that folded it to Gen1 would hide the very defect this exists for"
        );
        // ⊘ ...unlike PCIE_GEN_INFO's fields, where zero IS Gen1. The two encodings sharing
        // one word is the hazard; this pair of assertions is the record of it.
        assert!(PcieGenInfo::decode(0).is_some());
        for v in 7..=0xf {
            assert_eq!(PcieLinkCaps::decode(v), None, "MAX_SPEED={v} is undefined");
        }
    }

    /// A truncated params buffer is refused rather than read short.
    #[test]
    fn a_short_params_buffer_never_decodes() {
        let full = build_request(&[(BUS_INFO_INDEX_PCIE_GEN_INFO, 0)]);
        for n in [0usize, 4, 11, BUS_GET_INFO_V2_PARAMS_SIZE - 1] {
            assert_eq!(
                answer_bus_get_info_v2(&full[..n], &[]),
                Err(BusInfoError::ShortParams {
                    len: n,
                    need: BUS_GET_INFO_V2_PARAMS_SIZE
                })
            );
        }
    }
}
