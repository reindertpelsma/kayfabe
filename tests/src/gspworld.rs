//! A whole faked-GSP world, with **an independent guest** on the other side of it.
//!
//! `mode2_gsp_port_plan.md` S5 wants a replay oracle. The recorded C traces do not exist
//! (§6.2's capture patch is a separate task against a file this work does not own), so the
//! oracle here is the next best thing and in one respect a better one: an independent
//! re-implementation of **the driver's own side** of the protocol, written from
//! `ogkm: src/nvidia/src/libraries/msgq/msgq.c` and
//! `ogkm: .../gpu/gsp/message_queue_cpu.c` rather than from `kayfabe-gsp`.
//!
//! ★ The independence is the point, so it is maintained deliberately: [`Guest`] does not
//! call `kayfabe_gsp::checksum32`, `rx_link_check` or `bytes_to_elements`. It folds its
//! own checksum, runs its own acceptance predicate and derives its own element counts. A
//! test that passed because both sides shared a bug would prove nothing.
//!
//! Everything here is an **instrument** — `mutants.toml` excludes `tests/src/**` for that
//! reason.

use std::collections::BTreeMap;

use kayfabe_arch::ids::{ClassId, ControlCmd, VChid};
use kayfabe_arch::{
    Arch, DoorbellTarget, GmmuFmt, GspModel, GspObservation, GspReg, LibosRegionLayout, ObjectKind,
    PushbufferAbi, UserdModel,
};
use kayfabe_gsp::{
    EchoOk, ElementLayout, FunctionCodes, GspAbi, GspFault, GspFsm, GuestRam, InitArgsLayout,
    MsgqAbi, RamRefused, RpcAbi, ServiceReport, Transition, TransportHdr,
};
use kayfabe_mocks::MockArch;

// ─────────────────────────────── guest physical memory ───────────────────────────────

/// Sparse guest RAM: a page exists only once something allocated it, so an access to
/// memory the guest never published is a **refusal**, not a silent zero.
#[derive(Debug, Default)]
pub struct FakeRam {
    pages: BTreeMap<u64, Vec<u8>>,
    /// Every read this RAM served, as `(gpa, len)`. The instrument behind
    /// "E8 reads **zero** guest RAM" — a count, never an absence.
    pub reads: Vec<(u64, usize)>,
    /// Every write, likewise.
    pub writes: Vec<(u64, usize)>,
}

/// The page granularity of [`FakeRam`], and of the guest's own page table.
pub const PAGE: u64 = 4096;

impl FakeRam {
    /// Allocate one page, zeroed. Idempotent.
    pub fn alloc(&mut self, gpa: u64) {
        self.pages
            .entry(gpa & !(PAGE - 1))
            .or_insert_with(|| vec![0u8; PAGE as usize]);
    }

    /// Allocate `n` pages starting at `gpa`.
    pub fn alloc_range(&mut self, gpa: u64, n: u64) {
        for i in 0..n {
            self.alloc(gpa + i * PAGE);
        }
    }

    fn visit(
        &mut self,
        gpa: u64,
        len: usize,
        mut f: impl FnMut(&mut [u8], usize),
    ) -> Result<(), RamRefused> {
        let mut done = 0usize;
        while done < len {
            let at = gpa + done as u64;
            let base = at & !(PAGE - 1);
            let within = (at - base) as usize;
            let take = ((PAGE as usize) - within).min(len - done);
            let page = self.pages.get_mut(&base).ok_or(RamRefused { gpa, len })?;
            f(&mut page[within..within + take], done);
            done += take;
        }
        Ok(())
    }
}

impl GuestRam for FakeRam {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused> {
        self.reads.push((gpa, buf.len()));
        let mut out = vec![0u8; buf.len()];
        self.visit(gpa, buf.len(), |src, at| {
            out[at..at + src.len()].copy_from_slice(src);
        })?;
        buf.copy_from_slice(&out);
        Ok(())
    }

    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused> {
        self.writes.push((gpa, bytes.len()));
        let src = bytes.to_vec();
        self.visit(gpa, bytes.len(), |dst, at| {
            dst.copy_from_slice(&src[at..at + dst.len()]);
        })
    }
}

// ──────────────────────────────── the two ABI profiles ────────────────────────────────

/// One driver era's element shape. **Two of these exist, and they are incompatible.**
#[derive(Debug, Clone, Copy)]
pub struct Profile {
    /// For failure messages.
    pub name: &'static str,
    /// `queueElementHdrSize`.
    pub hdr: usize,
    /// `checkSum` offset.
    pub checksum: usize,
    /// `seqNum` offset.
    pub seqnum: usize,
    /// `elemCount` offset, where the version has one.
    pub elem_count: Option<usize>,
    /// `(mctp_off, mctp_word, nvdm_off, nvdm_word)`.
    pub mctp: Option<(usize, u32, usize, u32)>,
    /// Does the init-args struct declare `queueElementHdrSize`?
    pub declares_hdr_size: bool,
}

/// The **580 / r535 / r570** element: 48 bytes, `elemCount@40`, no transport headers.
///
/// [src] `nv: r535/nvrm/gsp.h:808-816` (`authTagBuffer[16]@0, aadBuffer[16]@16,
/// checkSum@32, seqNum@36, elemCount@40, rpc@48`), independently `nv: r535/rpc.c:94-102`
/// (which spells the 44→48 pad explicitly), and the C artifact implements exactly this
/// (`C: src/qemu/nvkvm_gpu_emul.c:1583-1602`).
pub const P580: Profile = Profile {
    name: "580 (r535/r570-shaped)",
    hdr: 48,
    checksum: 32,
    seqnum: 36,
    elem_count: Some(40),
    mctp: None,
    declares_hdr_size: false,
};

/// The **610** element: 16 bytes, MCTP/NVDM transport headers at 0 and 4, no `elemCount`.
///
/// [src] `ogkm: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:52-67`, validated on
/// receive at `ogkm: message_queue_cpu.c:737-759`.
///
/// ★ The two transport **words** here are placeholders. `mctpCreateTransportHeader(SOM=1,
/// EOM=1, 0,0,0)` and `mctpCreateNvdmHeader(NVDM_TYPE_RM_RPC)` assemble them from bit
/// fields in `mctp_format.h`/`nvdm_format.h` that this work did not transcribe, and
/// inventing them would be exactly the "cite, never invent" failure. What is under test is
/// the *shape*: that a transport header exists, is emitted, and is validated. The real
/// words belong in `kayfabe-abi` beside the rest of the version's table.
pub const P610: Profile = Profile {
    name: "610 (MCTP/NVDM)",
    hdr: 16,
    checksum: 8,
    seqnum: 12,
    elem_count: None,
    mctp: Some((0, 0x0000_0001, 4, 0x0000_10de)),
    declares_hdr_size: true,
};

impl Profile {
    /// The crate-side element layout this profile describes.
    #[must_use]
    pub fn layout(&self) -> ElementLayout {
        let transport = match self.mctp {
            None => TransportHdr::None,
            Some((header_off, header_word, nvdm_off, nvdm_word)) => TransportHdr::Mctp {
                header_off,
                header_word,
                nvdm_off,
                nvdm_word,
            },
        };
        ElementLayout::new(
            self.hdr,
            self.checksum,
            self.seqnum,
            self.elem_count,
            transport,
        )
        .expect("profile describes a real element")
    }

    /// The init-args layout that goes with it.
    ///
    /// `NvLength` is `size_t`, so there are 4 pad bytes after the `u32` at +8 — the C's
    /// `+0/+8/+16/+24` are right, and right because they were hard-coded rather than
    /// derived (`C: src/qemu/nvkvm_gpu_emul.c:3411-3425`).
    #[must_use]
    pub fn init_args(&self) -> InitArgsLayout {
        InitArgsLayout {
            shared_mem_pa_off: 0,
            pte_count_off: 8,
            cmd_queue_off_off: 16,
            stat_queue_off_off: 24,
            min_size: if self.declares_hdr_size { 40 } else { 32 },
            element_hdr_size_off: if self.declares_hdr_size {
                Some(32)
            } else {
                None
            },
        }
    }

    /// The whole Axis-A bundle.
    #[must_use]
    pub fn abi(&self) -> GspAbi {
        GspAbi {
            msgq: MsgqAbi {
                // MSGQ_VERSION = 0, MSGQ_MSG_SIZE_MIN = 16, MSGQ_FLAGS_SWAP_RX = 1
                // (`ogkm: msgq_priv.h:37-38`, `ogkm: msgq.h:30-39`); RM_PAGE_SIZE = 4096
                // (`ogkm: rm_page_size.h:38`) — a DRIVER page size, not the host's.
                version: 0,
                msg_size_min: 16,
                swap_rx_flag: 1,
                region_page_size: 4096,
            },
            element: self.layout(),
            rpc: RpcAbi {
                header_version: 0x0300_0000,
                codes: FUNCTIONS,
            },
            // queueElementSizeMax = RM_PAGE_SIZE * 16 (`ogkm: message_queue_cpu.c:88-89`).
            element_size_max: 4096 * 16,
            init_args: self.init_args(),
            driver: *kayfabe_abi::versions::table_for(kayfabe_abi::versions::BENCH_DRIVER)
                .expect("the bench driver has a table"),
        }
    }
}

/// The function ids, all explicit in the driver's X-macro table
/// (`ogkm: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:11, 57, 75, 81, 82, 83, 86, 113,
/// 254, 256`).
pub const FUNCTIONS: FunctionCodes = FunctionCodes {
    set_guest_system_info: 1,
    unloading_guest_driver: 47,
    get_gsp_static_info: 65,
    continuation_record: 71,
    gsp_set_system_info: 72,
    set_registry: 73,
    gsp_rm_control: 76,
    gsp_rm_alloc: 103,
    gsp_init_done: 0x1001,
    post_event: 0x1003,
};

// ───────────────────────────────── two GSP models ─────────────────────────────────

/// A register model with **deliberately fake** offsets and bit positions.
///
/// Two of these exist with nothing in common, which is the whole test of the seam: if any
/// offset, bit or sentinel had leaked into `kayfabe-gsp`, the same FSM could not drive
/// both. `MockArch`'s own doc states the principle — *"deliberately fake encodings so any
/// core code that secretly assumes a real NVIDIA encoding fails the mock-driven tests"*.
#[derive(Debug, Clone, Copy)]
pub struct FakeGspModel {
    base: u64,
    stride: u64,
    bar: u8,
    startcpu_bit: u64,
    unload_sentinel: u32,
    swgen0_bit: u64,
    wpr2_hi_up: u64,
    riscv_active_bit: u64,
    suspend_sentinel: u64,
    /// The id8 of the region carrying the init args ("RMARGS", little-endian ASCII —
    /// `C: src/qemu/nvkvm_gpu_emul.c:3408`).
    pub rmargs_id: u64,
}

/// Model **A** — one plausible register map.
pub const MODEL_A: FakeGspModel = FakeGspModel {
    base: 0x0011_0000,
    stride: 4,
    bar: 0,
    startcpu_bit: 0x2,
    unload_sentinel: 0xff,
    swgen0_bit: 1 << 6,
    wpr2_hi_up: 0x0000_1fff,
    riscv_active_bit: 1 << 7,
    suspend_sentinel: 0x8000_0000,
    rmargs_id: 0x0000_524d_4152_4753,
};

/// Model **B** — a different BAR, a different base, a different stride, a different
/// STARTCPU bit, a different unload sentinel, a different WPR2 encoding, a different
/// suspend sentinel. Nothing is shared with [`MODEL_A`] except the abstract vocabulary.
pub const MODEL_B: FakeGspModel = FakeGspModel {
    base: 0x0002_0000,
    stride: 8,
    bar: 2,
    startcpu_bit: 0x40,
    unload_sentinel: 0xdead_beef,
    swgen0_bit: 1 << 17,
    wpr2_hi_up: 0x00ab_cdef,
    riscv_active_bit: 1 << 3,
    suspend_sentinel: 0x0000_0bad,
    rmargs_id: 0x0000_524d_4152_4753,
};

impl FakeGspModel {
    /// The (bar, offset) this model puts a register at — the test's way to poke it
    /// without knowing the encoding.
    #[must_use]
    pub fn at(&self, reg: GspReg) -> (u8, u64) {
        (self.bar, self.base + self.stride * self.index(reg))
    }

    /// The value that means STARTCPU on this model.
    #[must_use]
    pub fn startcpu(&self) -> u64 {
        self.startcpu_bit
    }

    /// The SEC2 mailbox argument that means Booter **Unload** on this model.
    #[must_use]
    pub fn unload_arg(&self) -> u64 {
        u64::from(self.unload_sentinel)
    }

    /// A value that clears the status-queue interrupt.
    #[must_use]
    pub fn irq_clear(&self) -> u64 {
        self.swgen0_bit
    }

    /// This model's suspend sentinel, for asserting what `MAILBOX0` serves.
    #[must_use]
    pub fn suspend_sentinel(&self) -> u64 {
        self.suspend_sentinel
    }

    fn index(&self, reg: GspReg) -> u64 {
        match reg {
            GspReg::GfwBootProgress => 0,
            GspReg::GfwBootPlm => 1,
            GspReg::GspFalconCpuctl => 2,
            GspReg::GspFalconHwcfg2 => 3,
            GspReg::GspFalconDmatrfcmd => 4,
            GspReg::GspFalconMailbox0 => 5,
            GspReg::GspFalconMailbox1 => 6,
            GspReg::GspFalconIrqstat => 7,
            GspReg::GspFalconIrqmask => 8,
            GspReg::GspFalconIrqdest => 9,
            GspReg::GspFalconIrqsclr => 10,
            GspReg::GspRiscvCpuctl => 11,
            GspReg::Sec2FalconCpuctl => 12,
            GspReg::Sec2FalconMailbox0 => 13,
            GspReg::Sec2FalconDmatrfcmd => 14,
            GspReg::Wpr2AddrLo => 15,
            GspReg::Wpr2AddrHi => 16,
            GspReg::GspQueueHead(i) => 17 + u64::from(i),
        }
    }
}

impl GspModel for FakeGspModel {
    fn decode_reg(&self, bar: u8, off: u64) -> Option<GspReg> {
        if bar != self.bar || off < self.base || !(off - self.base).is_multiple_of(self.stride) {
            return None;
        }
        let i = (off - self.base) / self.stride;
        Some(match i {
            0 => GspReg::GfwBootProgress,
            1 => GspReg::GfwBootPlm,
            2 => GspReg::GspFalconCpuctl,
            3 => GspReg::GspFalconHwcfg2,
            4 => GspReg::GspFalconDmatrfcmd,
            5 => GspReg::GspFalconMailbox0,
            6 => GspReg::GspFalconMailbox1,
            7 => GspReg::GspFalconIrqstat,
            8 => GspReg::GspFalconIrqmask,
            9 => GspReg::GspFalconIrqdest,
            10 => GspReg::GspFalconIrqsclr,
            11 => GspReg::GspRiscvCpuctl,
            12 => GspReg::Sec2FalconCpuctl,
            13 => GspReg::Sec2FalconMailbox0,
            14 => GspReg::Sec2FalconDmatrfcmd,
            15 => GspReg::Wpr2AddrLo,
            16 => GspReg::Wpr2AddrHi,
            17..=24 => GspReg::GspQueueHead((i - 17) as u8),
            _ => return None,
        })
    }

    fn is_startcpu(&self, value: u64) -> bool {
        value & self.startcpu_bit != 0
    }

    fn is_booter_unload(&self, sec2_mailbox0: u32) -> bool {
        sec2_mailbox0 == self.unload_sentinel
    }

    fn is_swgen0_clear(&self, value: u64) -> bool {
        value & self.swgen0_bit != 0
    }

    fn encode(&self, reg: GspReg, obs: &GspObservation) -> Option<u64> {
        Some(match reg {
            // Constant: always "boot complete", mask fully lowered.
            GspReg::GfwBootProgress => 0xff,
            GspReg::GfwBootPlm => 1,
            GspReg::GspFalconCpuctl => 0x10, // HALTED
            GspReg::GspFalconHwcfg2 => 1,    // RISCV_ENABLE
            GspReg::GspFalconDmatrfcmd | GspReg::Sec2FalconDmatrfcmd => 0, // IDLE
            GspReg::GspFalconMailbox0 => {
                if obs.suspended {
                    self.suspend_sentinel
                } else {
                    u64::from(obs.boot_args_lo)
                }
            }
            GspReg::GspFalconMailbox1 => u64::from(obs.boot_args_hi),
            GspReg::GspFalconIrqstat => {
                if obs.swgen0_pending {
                    self.swgen0_bit
                } else {
                    0
                }
            }
            GspReg::GspFalconIrqmask | GspReg::GspFalconIrqdest => self.swgen0_bit,
            GspReg::GspFalconIrqsclr => 0,
            GspReg::GspRiscvCpuctl => {
                if obs.riscv_active {
                    self.riscv_active_bit
                } else {
                    0
                }
            }
            GspReg::Wpr2AddrLo => 0,
            GspReg::Wpr2AddrHi => {
                if obs.wpr2_up {
                    self.wpr2_hi_up
                } else {
                    0
                }
            }
            GspReg::Sec2FalconCpuctl | GspReg::Sec2FalconMailbox0 => 0,
            GspReg::GspQueueHead(_) => 0,
        })
    }

    fn libos_region_layout(&self) -> LibosRegionLayout {
        LibosRegionLayout {
            // `{ LibosAddress id8; LibosAddress pa; LibosAddress size; NvU8 kind; NvU8 loc; }`
            // = 32 bytes with alignment (`ogkm: libos_init_args.h:49-56`), matching the C's
            // `LIBOS_REGION_STRIDE 32`.
            entry_stride: 32,
            id_offset: 0,
            pa_offset: 8,
            size_offset: 16,
            // LIBOS_MEMORY_REGION_INIT_ARGUMENTS_MAX = 4096 (`ogkm: libos_init_args.h:31`).
            // The C caps its scan at 16 (`C:3388-3407`) — a parameter it never named.
            // Kept small here only so a test's array is small; the point is that it is a
            // parameter at all.
            max_entries: 8,
            rmargs_id: self.rmargs_id,
        }
    }
}

/// An [`Arch`] that is `MockArch` in every respect **except** that it has a GSP.
///
/// Composition, not modification: the GSP seam is bolted onto an existing architecture
/// implementation with zero edits to it, which is the property CLAUDE.md rule 2 asks of
/// every new seam.
#[derive(Debug)]
pub struct GspArch {
    inner: MockArch,
    gsp: FakeGspModel,
}

impl GspArch {
    /// Wrap a register model.
    #[must_use]
    pub fn new(gsp: FakeGspModel) -> GspArch {
        GspArch {
            inner: MockArch::new(),
            gsp,
        }
    }

    /// The model, for a test that needs to know where a register lives.
    #[must_use]
    pub fn model(&self) -> FakeGspModel {
        self.gsp
    }
}

impl Arch for GspArch {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn classify(&self, class: ClassId) -> ObjectKind {
        self.inner.classify(class)
    }
    fn vchid_from_userd_flags(&self, flags: u32) -> VChid {
        self.inner.vchid_from_userd_flags(flags)
    }
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        self.inner.decode_doorbell(token)
    }
    fn mmu(&self) -> &dyn GmmuFmt {
        self.inner.mmu()
    }
    fn userd(&self) -> &dyn UserdModel {
        self.inner.userd()
    }
    fn is_case2_control(&self, cmd: ControlCmd) -> bool {
        self.inner.is_case2_control(cmd)
    }
    fn pushbuffer(&self) -> &dyn PushbufferAbi {
        self.inner.pushbuffer()
    }
    fn gsp(&self) -> Option<&dyn GspModel> {
        Some(&self.gsp)
    }
}

/// An [`Arch`] with **no** GSP model — the MISS = FAULT arm.
#[derive(Debug, Default)]
pub struct NoGspArch(MockArch);

impl Arch for NoGspArch {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn classify(&self, class: ClassId) -> ObjectKind {
        self.0.classify(class)
    }
    fn vchid_from_userd_flags(&self, flags: u32) -> VChid {
        self.0.vchid_from_userd_flags(flags)
    }
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        self.0.decode_doorbell(token)
    }
    fn mmu(&self) -> &dyn GmmuFmt {
        self.0.mmu()
    }
    fn userd(&self) -> &dyn UserdModel {
        self.0.userd()
    }
    fn is_case2_control(&self, cmd: ControlCmd) -> bool {
        self.0.is_case2_control(cmd)
    }
    fn pushbuffer(&self) -> &dyn PushbufferAbi {
        self.0.pushbuffer()
    }
}

// ─────────────────────────────────── the guest ───────────────────────────────────

/// What the guest's receive path refused with — the driver's own error vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestRefusal {
    /// `"Bad checksum."` (`ogkm: message_queue_cpu.c:730-734`).
    BadChecksum,
    /// `"MCTP protocol violation"` (`ogkm: message_queue_cpu.c:737-759`).
    MctpViolation,
    /// `"Bad sequence number."` — carrying both numbers (`:761-766`).
    BadSequence {
        /// What the guest expected.
        expected: u32,
        /// What arrived.
        got: u32,
    },
    /// `"Incorrect message length"` (`:487-497`, and the receive mirror at `:824-833`).
    BadLength(u32),
    /// The producer has not finished: fewer elements are available than declared
    /// (`NV_ERR_NOT_READY`, `:670-680`).
    Incomplete,
}

/// One message the guest accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMsg {
    /// `rpc.function`.
    pub function: u32,
    /// `rpc.sequence`.
    pub sequence: u32,
    /// `rpc.rpc_result`.
    pub rpc_result: u32,
    /// The body after the 32-byte envelope.
    pub payload: Vec<u8>,
    /// The element sequence number it arrived with.
    pub seq_num: u32,
}

/// The guest driver's half of the protocol, re-implemented from ogkm.
#[derive(Debug, Clone)]
pub struct Guest {
    p: Profile,
    /// The region's page table, in order. Deliberately **not** contiguous.
    pub pages: Vec<u64>,
    /// Byte offset of the command queue within the region.
    pub cmd_off: u64,
    /// Byte offset of the status queue within the region.
    pub stat_off: u64,
    /// Guest-physical address of the LibOS region array.
    pub boot_args_gpa: u64,
    /// Guest-physical address of `GSP_ARGUMENTS_CACHED`.
    pub rmargs_gpa: u64,
    /// `size`.
    pub size: u32,
    /// `msgSize`.
    pub msg_size: u32,
    /// `msgCount`, derived exactly as `msgqTxCreate` derives it.
    pub msg_count: u32,
    /// `rxHdrOff`.
    pub rx_hdr_off: u32,
    /// `entryOff`.
    pub entry_off: u32,
    /// `flags`.
    pub flags: u32,
    /// The command queue's producer position.
    pub tx_write_ptr: u32,
    /// The command queue's per-message sequence.
    pub tx_seq: u32,
    /// The status queue's consumer position.
    pub rx_read_ptr: u32,
    /// The sequence the next status message must carry.
    pub rx_seq: u32,
    /// Whether `msgqRxLink` has succeeded.
    pub linked: bool,
    rmargs_id: u64,
}

impl Guest {
    /// Allocate a guest's queues at `pages` — a **scrambled**, non-contiguous list, which
    /// is what `NV_MEMORY_NONCONTIGUOUS` produces (`ogkm: message_queue_cpu.c:250-256`).
    ///
    /// The geometry is `msgqTxCreate`'s own derivation
    /// (`ogkm: msgq.c:236-251`): `rxHdrOff = ALIGN_UP(32, 1 << hdrAlign)`,
    /// `entryOff = ALIGN_UP(rxHdrOff + 4, 1 << entryAlign)`,
    /// `msgCount = (size - entryOff) / msgSize`, with `hdrAlign = 4` and
    /// `entryAlign = RM_PAGE_SHIFT` (`ogkm: message_queue_cpu.c:88-91`).
    #[must_use]
    pub fn new(p: Profile, pages: Vec<u64>, boot_args_gpa: u64, rmargs_id: u64) -> Guest {
        let msg_size = PAGE as u32;
        let size = 0x8000; // 8 pages per queue: a 7-slot ring, small on purpose.
        let rx_hdr_off = align_up(32, 1 << 4);
        let entry_off = align_up(rx_hdr_off + 4, 1 << 12);
        let msg_count = (size - entry_off) / msg_size;
        Guest {
            p,
            pages,
            cmd_off: PAGE, // the page table occupies the first page
            stat_off: PAGE + u64::from(size),
            boot_args_gpa,
            rmargs_gpa: boot_args_gpa + PAGE,
            size,
            msg_size,
            msg_count,
            rx_hdr_off,
            entry_off,
            flags: 1, // MSGQ_FLAGS_SWAP_RX
            tx_write_ptr: 0,
            tx_seq: 0,
            rx_read_ptr: 0,
            rx_seq: 0,
            linked: false,
            rmargs_id,
        }
    }

    /// How many pages the region needs.
    #[must_use]
    pub fn region_pages(&self) -> u64 {
        1 + 2 * u64::from(self.size) / PAGE
    }

    /// Write everything the driver publishes before the GSP is expected to answer: the
    /// region's page table, the LibOS region array, `MESSAGE_QUEUE_INIT_ARGUMENTS`, and
    /// the command queue's own tx header.
    pub fn publish(&self, ram: &mut FakeRam) {
        // The self-describing page table: entry i is the region's page i, and entry 0 is
        // the page the table itself starts on (`ogkm: message_queue_cpu.c:295-329`).
        let mut table = Vec::new();
        for pa in &self.pages {
            table.extend_from_slice(&pa.to_le_bytes());
        }
        ram.write(self.pages[0], &table).expect("table page mapped");

        // The LibOS region array. Entry 0 is a decoy, entry 1 is all-zero (which is a
        // SKIP here, not a terminator — [inferred] I8), and RMARGS is last.
        let mut arr = vec![0u8; 32 * 3];
        arr[0..8].copy_from_slice(&0x0000_4d45_4d31u64.to_le_bytes());
        arr[8..16].copy_from_slice(&0xdead_0000u64.to_le_bytes());
        arr[64..72].copy_from_slice(&self.rmargs_id.to_le_bytes());
        arr[72..80].copy_from_slice(&self.rmargs_gpa.to_le_bytes());
        arr[80..88].copy_from_slice(&PAGE.to_le_bytes());
        ram.write(self.boot_args_gpa, &arr).expect("array mapped");

        // MESSAGE_QUEUE_INIT_ARGUMENTS.
        let mut args = vec![0u8; 40];
        args[0..8].copy_from_slice(&self.pages[0].to_le_bytes());
        args[8..12].copy_from_slice(&(self.pages.len() as u32).to_le_bytes());
        args[16..24].copy_from_slice(&self.cmd_off.to_le_bytes());
        args[24..32].copy_from_slice(&self.stat_off.to_le_bytes());
        if self.p.declares_hdr_size {
            args[32..40].copy_from_slice(&(self.p.hdr as u64).to_le_bytes());
        }
        ram.write(self.rmargs_gpa, &args).expect("rmargs mapped");

        self.write_cmd_header(ram);
    }

    /// `msgqTxCreate`'s publication: the whole 32-byte header in field order.
    pub fn write_cmd_header(&self, ram: &mut FakeRam) {
        let hdr = [
            0u32, // version = MSGQ_VERSION
            self.size,
            self.msg_size,
            self.msg_count,
            self.tx_write_ptr,
            self.flags,
            self.rx_hdr_off,
            self.entry_off,
        ];
        let mut b = Vec::new();
        for w in hdr {
            b.extend_from_slice(&w.to_le_bytes());
        }
        self.wr(ram, self.cmd_off, &b);
    }

    /// `msgqRxLink`, re-implemented: read the peer's header and run all nine checks in
    /// the driver's order (`ogkm: msgq.c:329-405`), returning the driver's own code.
    ///
    /// # Errors
    ///
    /// The negative code the driver would return. `-7` is `rx.size != size` — the one the
    /// bench observed 71 064 times.
    pub fn rx_link(&mut self, ram: &mut FakeRam) -> Result<(), i32> {
        if self.linked {
            return Err(-1);
        }
        let msg_size = self.msg_size;
        let size = self.size;
        if msg_size < 16 {
            return Err(-2);
        }
        if msg_size > size {
            return Err(-3);
        }
        let mut b = [0u8; 32];
        self.rd(ram, self.stat_off, &mut b);
        let w = |i: usize| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        let (rx_version, rx_size, rx_msg_size, rx_msg_count, rx_hdr_off, rx_entry_off) =
            (w(0), w(1), w(2), w(3), w(6), w(7));
        if size < rx_entry_off + msg_size {
            return Err(-6);
        }
        if rx_size != size {
            return Err(-7);
        }
        if rx_msg_size != msg_size {
            return Err(-8);
        }
        if rx_version != 0 {
            return Err(-9);
        }
        if rx_hdr_off < 32
            || rx_entry_off < self.rx_hdr_off + 4
            || rx_msg_count != (size - rx_entry_off) / msg_size
        {
            return Err(-10);
        }
        self.linked = true;
        // On success the read pointer is zeroed and published — into OUR OWN backing
        // store, because SWAP_RX is agreed (`ogkm: msgq.c:416-419, 435-437`).
        self.rx_read_ptr = 0;
        self.publish_read_ptr(ram);
        Ok(())
    }

    /// Publish our consumption of the status queue (the swapped location).
    pub fn publish_read_ptr(&self, ram: &mut FakeRam) {
        self.wr(
            ram,
            self.cmd_off + u64::from(self.rx_hdr_off),
            &self.rx_read_ptr.to_le_bytes(),
        );
    }

    /// Free elements in the command queue, as `msgqTxGetFreeSpace` computes it
    /// (`ogkm: msgq.c:490-496`), reading the peer's ack from the status queue's rx header.
    pub fn free_space(&self, ram: &mut FakeRam) -> u32 {
        let mut b = [0u8; 4];
        self.rd(ram, self.stat_off + u64::from(self.rx_hdr_off), &mut b);
        let read_ptr = u32::from_le_bytes(b);
        if read_ptr >= self.msg_count {
            return 0;
        }
        let mut free = read_ptr + self.msg_count - self.tx_write_ptr - 1;
        if free >= self.msg_count {
            free -= self.msg_count;
        }
        free
    }

    /// Send one command, exactly as `GspMsgQueueSendCommand` does.
    ///
    /// # Errors
    ///
    /// `"no free space"` when the ring is full — the driver's own refusal
    /// (`ogkm: msgq.c:544-547`).
    pub fn send(
        &mut self,
        ram: &mut FakeRam,
        function: u32,
        sequence: u32,
        payload: &[u8],
    ) -> Result<u32, &'static str> {
        let rpc_length = 32 + payload.len() as u32;
        let msg_len = self.p.hdr as u32 + rpc_length;
        let n = msg_len.div_ceil(self.msg_size);
        if n > self.free_space(ram) {
            return Err("no free space");
        }
        let mut buf = vec![0u8; (n * self.msg_size) as usize];
        put32(&mut buf, self.p.seqnum, self.tx_seq);
        if let Some(o) = self.p.elem_count {
            put32(&mut buf, o, n);
        }
        if let Some((ho, hw, no, nw)) = self.p.mctp {
            put32(&mut buf, ho, hw);
            put32(&mut buf, no, nw);
        }
        let h = self.p.hdr;
        put32(&mut buf, h, 0x0300_0000);
        put32(&mut buf, h + 4, 0x4350_5256);
        put32(&mut buf, h + 8, rpc_length);
        put32(&mut buf, h + 12, function);
        put32(&mut buf, h + 24, sequence);
        buf[h + 32..h + 32 + payload.len()].copy_from_slice(payload);
        let sum = fold(&buf, msg_len as usize);
        put32(&mut buf, self.p.checksum, sum);

        for i in 0..n {
            let slot = (self.tx_write_ptr + i) % self.msg_count;
            let at = (i * self.msg_size) as usize;
            self.wr(
                ram,
                self.element_off(self.cmd_off, slot),
                &buf[at..at + self.msg_size as usize],
            );
        }
        self.tx_write_ptr = (self.tx_write_ptr + n) % self.msg_count;
        self.wr(ram, self.cmd_off + 16, &self.tx_write_ptr.to_le_bytes());
        self.tx_seq = self.tx_seq.wrapping_add(1);
        Ok(n)
    }

    /// Drain the status queue, as `GspMsgQueueReceiveStatus` does.
    ///
    /// # Errors
    ///
    /// The driver's own refusal for the first message that fails.
    pub fn recv(&mut self, ram: &mut FakeRam) -> Result<Vec<GuestMsg>, GuestRefusal> {
        let mut out = Vec::new();
        loop {
            let mut b = [0u8; 4];
            self.rd(ram, self.stat_off + 16, &mut b);
            let write_ptr = u32::from_le_bytes(b);
            if write_ptr >= self.msg_count {
                return Err(GuestRefusal::Incomplete);
            }
            let mut avail = write_ptr + self.msg_count - self.rx_read_ptr;
            if avail >= self.msg_count {
                avail -= self.msg_count;
            }
            if avail == 0 {
                return Ok(out);
            }
            let mut first = vec![0u8; self.msg_size as usize];
            self.rd(
                ram,
                self.element_off(self.stat_off, self.rx_read_ptr),
                &mut first,
            );
            let rpc_length = get32(&first, self.p.hdr + 8);
            let msg_len = self.p.hdr as u32 + rpc_length;
            // The driver's own bound is `msgLen >= queueElementHdrSize`
            // (`ogkm: message_queue_cpu.c:824-833`), which admits `rpc.length == 0`; this
            // model additionally needs the envelope itself to be present, because unlike
            // the driver it slices rather than casting a flexible array.
            if rpc_length < 32 || msg_len < self.p.hdr as u32 || msg_len > 16 * PAGE as u32 {
                return Err(GuestRefusal::BadLength(rpc_length));
            }
            let n = msg_len.div_ceil(self.msg_size);
            if n > avail {
                return Err(GuestRefusal::Incomplete);
            }
            let mut run = vec![0u8; (n * self.msg_size) as usize];
            run[..self.msg_size as usize].copy_from_slice(&first);
            for i in 1..n {
                let slot = (self.rx_read_ptr + i) % self.msg_count;
                let at = (i * self.msg_size) as usize;
                let end = at + self.msg_size as usize;
                let mut tmp = vec![0u8; self.msg_size as usize];
                self.rd(ram, self.element_off(self.stat_off, slot), &mut tmp);
                run[at..end].copy_from_slice(&tmp);
            }
            if fold(&run, msg_len as usize) != 0 {
                return Err(GuestRefusal::BadChecksum);
            }
            if let Some((ho, hw, no, nw)) = self.p.mctp
                && (get32(&run, ho) != hw || get32(&run, no) != nw)
            {
                return Err(GuestRefusal::MctpViolation);
            }
            let seq_num = get32(&run, self.p.seqnum);
            if seq_num != self.rx_seq {
                return Err(GuestRefusal::BadSequence {
                    expected: self.rx_seq,
                    got: seq_num,
                });
            }
            let h = self.p.hdr;
            out.push(GuestMsg {
                function: get32(&run, h + 12),
                sequence: get32(&run, h + 24),
                rpc_result: get32(&run, h + 16),
                payload: run[h + 32..h + rpc_length as usize].to_vec(),
                seq_num,
            });
            self.rx_seq = self.rx_seq.wrapping_add(1);
            self.rx_read_ptr = (self.rx_read_ptr + n) % self.msg_count;
            self.publish_read_ptr(ram);
        }
    }

    fn element_off(&self, queue: u64, slot: u32) -> u64 {
        queue + u64::from(self.entry_off) + u64::from(slot) * u64::from(self.msg_size)
    }

    /// Resolve a region offset through the page table — the guest's own view, computed
    /// independently of `RegionMap`.
    #[must_use]
    pub fn gpa_of(&self, off: u64) -> u64 {
        let page = (off / PAGE) as usize;
        self.pages[page] + off % PAGE
    }

    fn rd(&self, ram: &mut FakeRam, off: u64, buf: &mut [u8]) {
        let mut done = 0usize;
        while done < buf.len() {
            let at = off + done as u64;
            let take = ((PAGE - at % PAGE) as usize).min(buf.len() - done);
            ram.read(self.gpa_of(at), &mut buf[done..done + take])
                .expect("guest reads its own memory");
            done += take;
        }
    }

    fn wr(&self, ram: &mut FakeRam, off: u64, bytes: &[u8]) {
        let mut done = 0usize;
        while done < bytes.len() {
            let at = off + done as u64;
            let take = ((PAGE - at % PAGE) as usize).min(bytes.len() - done);
            ram.write(self.gpa_of(at), &bytes[done..done + take])
                .expect("guest writes its own memory");
            done += take;
        }
    }
}

/// The XOR fold, re-implemented from `ogkm: message_queue_priv.h:191-209`: 64-bit steps
/// to the next 8-byte boundary past `len`, reduced by `hi ^ lo`.
#[must_use]
pub fn fold(bytes: &[u8], len: usize) -> u32 {
    let end = len.next_multiple_of(8).min(bytes.len());
    let mut acc = 0u64;
    let mut i = 0;
    while i + 8 <= end {
        acc ^= u64::from_le_bytes(bytes[i..i + 8].try_into().expect("8 bytes"));
        i += 8;
    }
    ((acc >> 32) as u32) ^ (acc as u32)
}

fn align_up(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

fn put32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn get32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().expect("4 bytes"))
}

// ───────────────────────────────── the composed world ─────────────────────────────────

/// One device, one guest, one RAM. Everything a boot needs and nothing else — no VMM, no
/// fd, no clock, no thread.
pub struct GspWorld {
    /// Guest physical memory.
    pub ram: FakeRam,
    /// The independent guest driver.
    pub guest: Guest,
    /// The faked GSP under test.
    pub fsm: GspFsm,
    /// The architecture, carrying one of the two register models.
    pub arch: GspArch,
    profile: Profile,
    next_base: u64,
}

impl GspWorld {
    /// A cold device with a freshly allocated, fragmented guest region.
    #[must_use]
    pub fn new(profile: Profile, model: FakeGspModel) -> GspWorld {
        let mut w = GspWorld {
            ram: FakeRam::default(),
            guest: Guest::new(profile, vec![0], 0, model.rmargs_id),
            fsm: GspFsm::new(profile.abi()),
            arch: GspArch::new(model),
            profile,
            next_base: 0x4000_0000,
        };
        w.allocate_guest_memory();
        w
    }

    /// Give the guest a **fresh, fragmented** region — a new driver life allocates new
    /// memory, and `NV_MEMORY_NONCONTIGUOUS` means the pages are wherever the allocator
    /// had them (`ogkm: message_queue_cpu.c:250-256`).
    /// Give the guest a fresh, fragmented region — what a new driver life allocates.
    pub fn allocate_guest_memory(&mut self) {
        let n = Guest::new(self.profile, vec![0], 0, self.arch.model().rmargs_id).region_pages();
        let base = self.next_base;
        self.next_base += 0x1000_0000;
        // Descending, widely spaced: `base + offset` lands on the WRONG page for every
        // page but the first, which is exactly what GSP-D8 is about.
        let pages: Vec<u64> = (0..n).map(|i| base + (n - i) * 0x0001_0000).collect();
        for &p in &pages {
            self.ram.alloc(p);
        }
        let boot_args = base + 0x0F00_0000;
        self.ram.alloc_range(boot_args, 2);
        self.guest = Guest::new(self.profile, pages, boot_args, self.arch.model().rmargs_id);
        self.guest.publish(&mut self.ram);
    }

    /// Write a GSP register, by its abstract name — the test never knows an offset.
    ///
    /// # Errors
    ///
    /// Whatever the transition it triggers refuses with.
    pub fn wr(&mut self, reg: GspReg, val: u64) -> Result<ServiceReport, GspFault> {
        let (bar, off) = self.arch.model().at(reg);
        self.fsm
            .mmio_write(&mut self.ram, &self.arch, &mut EchoOk, bar, off, val)
    }

    /// Read a GSP register, by its abstract name.
    ///
    /// # Panics
    ///
    /// If the register is not this model's, or the FSM cannot serve it.
    #[must_use]
    pub fn rd(&self, reg: GspReg) -> u64 {
        let (bar, off) = self.arch.model().at(reg);
        self.fsm
            .mmio_read(&self.arch, bar, off)
            .expect("a GSP register")
            .expect("serviceable")
    }

    /// The guest's boot, in `kgspBootstrap_TU102`'s order
    /// (`ogkm: kernel_gsp_tu102.c:522-618`): FWSEC/STARTCPU, boot-args mailboxes, Booter
    /// Load. Returns every transition that fired.
    /// Drive the guest's boot and return every transition that fired.
    ///
    /// # Panics
    ///
    /// If any step is refused.
    pub fn boot(&mut self) -> Vec<Transition> {
        let m = self.arch.model();
        let mut t = Vec::new();
        t.extend(
            self.wr(GspReg::GspFalconCpuctl, m.startcpu())
                .unwrap()
                .transitions,
        );
        let gpa = self.guest.boot_args_gpa;
        t.extend(
            self.wr(GspReg::GspFalconMailbox0, gpa & 0xFFFF_FFFF)
                .unwrap()
                .transitions,
        );
        t.extend(
            self.wr(GspReg::GspFalconMailbox1, gpa >> 32)
                .unwrap()
                .transitions,
        );
        // Booter Load: a SEC2 STARTCPU whose argument is not the Unload sentinel.
        t.extend(self.wr(GspReg::Sec2FalconMailbox0, 0).unwrap().transitions);
        t.extend(
            self.wr(GspReg::Sec2FalconCpuctl, m.startcpu())
                .unwrap()
                .transitions,
        );
        t
    }

    /// Ring the command-queue doorbell.
    /// Ring the command-queue doorbell.
    ///
    /// # Errors
    ///
    /// As [`GspWorld::wr`].
    pub fn doorbell(&mut self) -> Result<ServiceReport, GspFault> {
        self.wr(GspReg::GspQueueHead(0), 1)
    }

    /// The guest links its status queue and drains whatever is waiting.
    /// The guest links its status queue and drains whatever is waiting.
    ///
    /// # Panics
    ///
    /// If the published header does not link, or the stream does not validate.
    pub fn link_and_drain(&mut self) -> Vec<GuestMsg> {
        self.guest
            .rx_link(&mut self.ram)
            .expect("the published header links");
        self.guest.recv(&mut self.ram).expect("a clean stream")
    }
}
