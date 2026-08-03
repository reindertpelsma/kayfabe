//! A whole faked-GSP world, with **an independent guest** on the other side of it.
//!
//! `mode2_gsp_port_plan.md` S5 wants a replay oracle. The recorded C traces do not exist
//! (§6.2's capture patch is a separate task against a file this work does not own), so the
//! oracle here is the next best thing and in one respect a better one: an independent
//! re-implementation of **the driver's own side** of the protocol, written from
//! `msgq.c` and `.../gpu/gsp/message_queue_cpu.c` rather than from `kayfabe-gsp`.
//!
//! ★ The `msgq` library **moved between the two trees**, so every citation below names
//! its tag and its own path: `ogkm-610: src/nvidia/src/libraries/msgq/msgq.c` (headers in
//! `src/nvidia/inc/libraries/msgq/`) is `ogkm-580: src/common/shared/msgq/msgq.c`
//! (headers in `src/common/shared/msgq/inc/msgq/`). The two are byte-identical and 580
//! runs a uniform **+1 line** from one extra include. `message_queue_cpu.c` keeps its
//! path (`src/nvidia/src/kernel/gpu/gsp/`) at both, but **not** its line numbers.
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

/// A deliberately tiny queue — 8 pages, a **7-slot** ring — so wrap and fullness are
/// reached rather than described. The default for [`Guest::new`].
pub const SMALL_QUEUE_SIZE: u32 = 0x8000;

/// The queue size three independent trees agree the driver actually uses: `0x40000`, i.e.
/// a **63-slot** ring
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:71-72`,
/// `ogkm-610: message_queue_cpu.c:71-72` — `defaultCommandQueueSize` /
/// `defaultStatusQueueSize`, the same two lines at both tags; `nv: r535/gsp.c:1164-1172`
/// verbatim). ★ Not `:88-91`, which this used to cite: at 580 those lines are the
/// status-queue *registry override*, not the default.
///
/// ★ Needed for the staging-buffer bound: only at this size can a message declare an
/// element count above `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX / RM_PAGE_SIZE` **and have that
/// many elements actually available**, which is the precondition for the overrun.
pub const REAL_QUEUE_SIZE: u32 = 0x0004_0000;

/// `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX` = `RM_PAGE_SIZE * 16`
/// (`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:91-92`) — and, at 580,
/// also the exact size of the receive **staging buffer** (see [`Guest::staging_bytes`]).
pub const STAGING_BYTES: u32 = 16 * PAGE as u32;

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
            let page = self.pages.get_mut(&base).ok_or(RamRefused {
                gpa,
                len,
                why: "this fake guest has no page at that address",
            })?;
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
///
/// ★★ B5: this is now a **consumer** of `kayfabe-abi`'s version table, not a second
/// definition of the same facts. A profile carries only the version key and its own name;
/// every offset, transport word and init-args geometry is looked up. Before this the
/// fixtures *were* the only definition of the element layout — `ElementLayout::new` had no
/// caller outside test code — so no production path selected a layout at all and the
/// version key was untested.
#[derive(Debug, Clone, Copy)]
pub struct Profile {
    /// For failure messages.
    pub name: &'static str,
    /// The driver version this profile speaks — the **key**, and the only thing that
    /// distinguishes the two.
    pub version: kayfabe_abi::DriverVersion,
}

/// The **580** element, keyed on the driver the bench actually runs.
///
/// [src] `ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:43-51`
/// (`authTagBuffer[16]@0, aadBuffer[16]@16, checkSum@32, seqNum@36, elemCount@40, rpc@48`),
/// independently `nv: r535/nvrm/gsp.h:808-816` and `nv: r535/rpc.c:94-102`, and the C
/// artifact implements exactly this (`C: src/qemu/nvkvm_gpu_emul.c:1583-1602`).
///
/// ★ Its **absent** transport headers are correct and must not be "fixed": `ogkm-580` has
/// no `mctp_format.h`, no `NVDM_TYPE_RM_RPC`, and its only MCTP is FSP/SEC2/NVSwitch.
/// Bytes @0..@31 are the Confidential-Compute buffers, which a CC-off guest never reads.
pub const P580: Profile = Profile {
    name: "580 (r535/r570-shaped)",
    version: kayfabe_abi::versions::BENCH_DRIVER,
};

/// The **610** element: 16 bytes, MCTP/NVDM transport headers at 0 and 4, no `elemCount`.
///
/// [src] `ogkm-610: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:52-67`, validated on
/// receive at `ogkm-610: message_queue_cpu.c:737-759` (the whole block; `:762` is already
/// the seqNum test and belongs to the next check).
///
/// ★★ **A seam, not a shape change: `ogkm-580` validates nothing here.** There is no
/// MCTP/NVDM block anywhere in 580's `GspMsgQueueReceiveStatus` — checksum
/// (`ogkm-580: message_queue_cpu.c:685-690`) is followed straight by the sequence check
/// (`:692-719`). The transport words exist only at 610.
pub const P610: Profile = Profile {
    name: "610 (MCTP/NVDM)",
    version: kayfabe_abi::DriverVersion {
        major: 610,
        minor: 43,
        patch: 2,
    },
};

impl Profile {
    /// The version table row this profile keys.
    #[must_use]
    pub fn table(&self) -> &'static kayfabe_abi::versions::DriverAbiTable {
        kayfabe_abi::versions::table_for(self.version).expect("a supported driver version")
    }

    /// `queueElementHdrSize`.
    #[must_use]
    pub fn hdr(&self) -> usize {
        self.table().gsp_element_wire().hdr_size()
    }

    /// `checkSum` offset.
    #[must_use]
    pub fn checksum(&self) -> usize {
        self.table().gsp_element_wire().checksum_off()
    }

    /// `seqNum` offset.
    #[must_use]
    pub fn seqnum(&self) -> usize {
        self.table().gsp_element_wire().seqnum_off()
    }

    /// `elemCount` offset, where the version has one.
    #[must_use]
    pub fn elem_count(&self) -> Option<usize> {
        self.table().gsp_element_wire().elem_count_off()
    }

    /// The transport words and the bits of them the guest validates.
    #[must_use]
    pub fn mctp(&self) -> Option<kayfabe_abi::versions::GspTransportWords> {
        self.table().gsp_element_wire().transport()
    }

    /// Does the init-args struct declare `queueElementHdrSize`?
    #[must_use]
    pub fn declares_hdr_size(&self) -> bool {
        self.table()
            .gsp_init_args_wire()
            .element_hdr_size_off()
            .is_some()
    }

    /// `queueElementSizeMax` — and, at 580, the receive staging buffer's size.
    #[must_use]
    pub fn element_size_max(&self) -> u32 {
        self.table().gsp_element_size_max()
    }
}

impl Profile {
    /// The crate-side element layout this profile describes — built from the production
    /// table, so a wrong entry there is a failing test here.
    #[must_use]
    pub fn layout(&self) -> ElementLayout {
        let transport = match self.mctp() {
            None => TransportHdr::None,
            Some(t) => TransportHdr::Mctp {
                header_off: t.header_off,
                header_word: t.header_word,
                nvdm_off: t.nvdm_off,
                nvdm_word: t.nvdm_word,
            },
        };
        ElementLayout::new(
            self.hdr(),
            self.checksum(),
            self.seqnum(),
            self.elem_count(),
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
        let wire = self.table().gsp_init_args_wire();
        InitArgsLayout {
            shared_mem_pa_off: 0,
            pte_count_off: 8,
            cmd_queue_off_off: 16,
            stat_queue_off_off: 24,
            min_size: wire.min_size(),
            element_hdr_size_off: wire.element_hdr_size_off(),
        }
    }

    /// The whole Axis-A bundle.
    ///
    /// ★★ **Delegated, as of 2026-07-31 — this used to be the ONLY assembly of a
    /// [`GspAbi`] anywhere in the tree, and it lived in a test harness.** Stage Q4 needs a
    /// production one, so the composition moved to
    /// [`kayfabe_device::abi::gsp_abi_for`] and this calls it. The consequence is the
    /// reason it is worth doing rather than copying: every GSP conformance test in this
    /// repository now drives the assembly a shipped device uses, so a mistake in it is a
    /// red test here instead of a bench cycle.
    #[must_use]
    pub fn abi(&self) -> GspAbi {
        kayfabe_device::abi::gsp_abi_for(self.version)
            .expect("a supported driver version with a describable element layout")
    }

    /// The assembly this harness used to perform itself, kept as the **differential**.
    ///
    /// ★ Not dead code and not a duplicate to be deleted: `abi_assembly_agrees` asserts
    /// this and [`Profile::abi`] produce the same value, which is what makes "the harness
    /// delegates" a checked statement rather than a claim in a comment. If the production
    /// assembly drifts, this is what says so.
    #[must_use]
    pub fn abi_local(&self) -> GspAbi {
        GspAbi {
            msgq: MsgqAbi {
                // MSGQ_VERSION = 0, MSGQ_MSG_SIZE_MIN = 16, MSGQ_FLAGS_SWAP_RX = 1
                // — byte-identical AND on the same lines at both tags, only the path
                // moves: `ogkm-610: src/nvidia/inc/libraries/msgq/msgq_priv.h:37-38` +
                // `msgq.h:30-39`,
                // `ogkm-580: src/common/shared/msgq/inc/msgq/msgq_priv.h:37-38` +
                // `msgq.h:30-39`. RM_PAGE_SIZE = 4096
                // (`ogkm-610: src/nvidia/inc/kernel/gpu/mem_mgr/rm_page_size.h:38`,
                // `ogkm-580: rm_page_size.h:38`) — a DRIVER page size, not the host's.
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
            // `queueElementSizeMax`, from the version table rather than from a literal
            // (`ogkm-580: message_queue_priv.h:92`, `ogkm-610: message_queue_cpu.c:88-89`).
            element_size_max: self.element_size_max(),
            init_args: self.init_args(),
            driver: *self.table(),
        }
    }
}

/// The function ids, all explicit in the driver's X-macro table
/// (`ogkm-610: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:11, 20, 31, 57, 75, 81, 82,
/// 83, 86, 113, 254, 256`). The ten `X(…)` rows sit on the same lines at `ogkm-580`; the
/// two `E(…)` event rows do not — `GSP_INIT_DONE` is `ogkm-580: rpc_global_enums.h:253`
/// and `POST_EVENT` is `:255`, because 610 carries an `#endif` at `:252` that 580 does
/// not. Every id is identical at both tags.
pub const FUNCTIONS: FunctionCodes = FunctionCodes {
    set_guest_system_info: 1,
    set_guest_system_info_ext: 0x40,
    init_gsp_trace_crash_buffer: 0xE4,
    free: 10,
    dup_object: 21,
    unloading_guest_driver: 47,
    get_gsp_static_info: 65,
    continuation_record: 71,
    gsp_set_system_info: 72,
    set_registry: 73,
    ecc_notifier_write_ack: 202,
    // `X(RM, UPDATE_BAR_PDE, 70)` — `ogkm-610:`/`ogkm-580: rpc_global_enums.h:80`.
    update_bar_pde: 70,
    gsp_rm_control: 76,
    gsp_rm_alloc: 103,
    gsp_init_done: 0x1001,
    post_event: 0x1003,
    // `E(RC_TRIGGERED, 0x1004)` — `ogkm-610: rpc_global_enums.h:257`, `ogkm-580: :256`.
    rc_triggered: 0x1004,
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
    /// ★ Deliberately wrong when set: OR the sentinel onto the mailbox shadow instead of
    /// replacing it. See [`FakeGspModel::with_or_ed_suspend_sentinel`].
    suspend_ors: bool,
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
    suspend_ors: false,
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
    suspend_ors: false,
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

    /// ★★ The same model with **one deliberate defect**: it OR-s the suspend sentinel
    /// onto the mailbox shadow instead of replacing it.
    ///
    /// That is 610's reading of the register — `#define
    /// INTERRUPT_PROCESSOR_SUSPENDED_VALUE 0x80000000` and
    /// `return (mailbox & INTERRUPT_PROCESSOR_SUSPENDED_VALUE) != 0`
    /// (`ogkm-610: kernel_gsp_tu102.c:333, 348`) — applied to a register **580 tests with
    /// exact equality**: `return (mailbox == 0x80000000)`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:1226-1238`,
    /// the constant inlined, no such symbol anywhere in that tree).
    ///
    /// So a shadow still holding a boot-args half with bit 31 set reads as *suspended* at
    /// 610 and hangs the teardown poll **forever** at 580 — and 580 polls it from two
    /// places: after fn-47 (`ogkm-580: kernel_gsp.c:4310`) and as a bootstrap liveness
    /// fallback (`ogkm-580: kernel_gsp_tu102.c:551`). This model exists so the pin that
    /// forbids OR-ing has something to catch; nothing else may use it.
    #[must_use]
    pub fn with_or_ed_suspend_sentinel(mut self) -> FakeGspModel {
        self.suspend_ors = true;
        self
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
                    if self.suspend_ors {
                        // The defect, on purpose — see `with_or_ed_suspend_sentinel`.
                        u64::from(obs.boot_args_lo) | self.suspend_sentinel
                    } else {
                        // Replace the whole value. Never OR: at 580 the guest's poll is
                        // `mailbox == 0x80000000` exactly.
                        self.suspend_sentinel
                    }
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

    /// This fixture models the falcon/secure-booter regime, so it selects that sequence.
    /// One shared stateless value: the *selection* is still per model instance.
    fn boot_sequence(&self) -> &dyn kayfabe_arch::gsp::BootSequence {
        static BOOT: kayfabe_gsp::FalconSecureBooterBoot = kayfabe_gsp::FalconSecureBooterBoot;
        &BOOT
    }

    fn libos_region_layout(&self) -> LibosRegionLayout {
        LibosRegionLayout {
            // `{ LibosAddress id8; LibosAddress pa; LibosAddress size; NvU8 kind; NvU8 loc; }`
            // = 32 bytes with alignment
            // (`ogkm-610: src/common/uproc/os/common/include/libos_init_args.h:49-56`,
            // `ogkm-580: libos_init_args.h:49-56` — identical at both, same lines),
            // matching the C's `LIBOS_REGION_STRIDE 32`.
            entry_stride: 32,
            id_offset: 0,
            pa_offset: 8,
            size_offset: 16,
            // LIBOS_MEMORY_REGION_INIT_ARGUMENTS_MAX = 4096
            // (`ogkm-610: libos_init_args.h:31`, `ogkm-580: libos_init_args.h:31`).
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
    fn vchid_from_userd_flags(&self, flags: u32) -> Option<VChid> {
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
    fn vchid_from_userd_flags(&self, flags: u32) -> Option<VChid> {
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
///
/// ★ Every citation here is `message_queue_cpu.c`, whose path is the same at both tags
/// and whose line numbers are not. Where only one tag is named, that is the claim: the
/// behaviour exists **only** there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestRefusal {
    /// `"Bad checksum."` (`ogkm-610: message_queue_cpu.c:730-735`,
    /// `ogkm-580: message_queue_cpu.c:685-690`).
    BadChecksum,
    /// `"MCTP protocol violation"` — **610 only**
    /// (`ogkm-610: message_queue_cpu.c:737-759`).
    ///
    /// ★★ A version seam, not a moved line: `ogkm-580` has no MCTP/NVDM validation at
    /// all, so under [`P580`] this variant is unreachable by construction (the profile
    /// carries no transport words).
    MctpViolation,
    /// `"Bad sequence number."` — carrying both numbers
    /// (`ogkm-610: message_queue_cpu.c:761-766`, `ogkm-580: message_queue_cpu.c:692-697`;
    /// the same `!=` test and the same `<`-only recovery branch at both).
    BadSequence {
        /// What the guest expected.
        expected: u32,
        /// What arrived.
        got: u32,
    },
    /// `"Incorrect message length"` — the send-path check
    /// (`ogkm-610: message_queue_cpu.c:491-497`, `ogkm-580: message_queue_cpu.c:468-475`)
    /// and its receive mirror (`ogkm-610: :826-833`, `ogkm-580: :762-770`).
    ///
    /// ★★ **The lower bound is version-split, and it is the seam this model sits on.**
    /// 610 refuses `msgLen < queueElementHdrSize`, which admits `rpc.length == 0`. 580
    /// refuses `msgLen < sizeof(GSP_MSG_QUEUE_ELEMENT)` — and at 580 that `sizeof`
    /// **includes the embedded `rpc_message_header_v`**
    /// (`ogkm-580: message_queue_priv.h:43-51`,
    /// `NV_DECLARE_ALIGNED(rpc_message_header_v rpc, 8)` at +48), so the bound is
    /// `48 + rpc.length >= 80`, i.e. **`rpc.length >= 32`**: 580 requires the RPC
    /// envelope to be present and 610 does not.
    BadLength(u32),
    /// The producer has not finished: fewer elements are available than declared
    /// (`NV_ERR_NOT_READY`, `ogkm-610: message_queue_cpu.c:664-677`,
    /// `ogkm-580: message_queue_cpu.c:632-645`).
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
    /// ★ `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX` — the size of the **staging buffer** the
    /// receive path copies elements into, and the bound this instrument exists to model.
    ///
    /// `_gspMsgQueueInit` allocates `(1 << GSP_MSG_QUEUE_ELEMENT_ALIGN) +
    /// GSP_MSG_QUEUE_ELEMENT_SIZE_MAX + msgqGetMetaSize()` from `portMemAllocNonPaged`,
    /// carves `pCmdQueueElement = ALIGN_UP(pWorkArea, 4096)` and puts the **live `msgq`
    /// metadata** immediately after it at `pCmdQueueElement + GSP_MSG_QUEUE_ELEMENT_SIZE_MAX`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:132-134, 143-145`).
    /// The receive loop then copies `elemCount` elements of 4096 bytes into it with **no
    /// bound but the ring's availability** (`:628, 648-650`), so an element declaring more
    /// than `element_size_max / msg_size` elements writes past a kernel allocation.
    pub staging_bytes: u32,
    rmargs_id: u64,
}

impl Guest {
    /// Allocate a guest's queues at `pages` — a **scrambled**, non-contiguous list, which
    /// is what `NV_MEMORY_NONCONTIGUOUS` produces
    /// (`ogkm-610: message_queue_cpu.c:250-256`, `ogkm-580: message_queue_cpu.c:228-234`
    /// — the same `memdescCreate` call at both).
    ///
    /// The geometry is `msgqTxCreate`'s own derivation
    /// (`ogkm-610: src/nvidia/src/libraries/msgq/msgq.c:236-251`,
    /// `ogkm-580: src/common/shared/msgq/msgq.c:237-252` — identical code, one line of
    /// drift from an extra include): `rxHdrOff = ALIGN_UP(32, 1 << hdrAlign)`,
    /// `entryOff = ALIGN_UP(rxHdrOff + 4, 1 << entryAlign)`,
    /// `msgCount = (size - entryOff) / msgSize`.
    ///
    /// ★ `hdrAlign = 4` and `entryAlign = RM_PAGE_SHIFT` are the **same numbers reached
    /// two different ways** — the axis-A seam B7 names. At 610 they are *runtime fields*
    /// the driver fills in (`ogkm-610: message_queue_cpu.c:88-91`:
    /// `queueHeaderAlign = 4`, `queueElementAlign = RM_PAGE_SHIFT`). At 580 they are
    /// compile-time macros with no field behind them
    /// (`ogkm-580: message_queue_priv.h:101-104`: `GSP_MSG_QUEUE_HEADER_ALIGN 4`,
    /// `GSP_MSG_QUEUE_ELEMENT_ALIGN RM_PAGE_SHIFT`). 580's `message_queue_cpu.c:88-91`
    /// is the status-queue registry override and says nothing about alignment.
    #[must_use]
    pub fn new(p: Profile, pages: Vec<u64>, boot_args_gpa: u64, rmargs_id: u64) -> Guest {
        // 8 pages per queue: a 7-slot ring, small on purpose.
        Guest::new_sized(p, pages, boot_args_gpa, rmargs_id, SMALL_QUEUE_SIZE)
    }

    /// The same guest with a caller-chosen queue size.
    ///
    /// ★ Needed because the **real** geometry is what makes the staging-buffer overrun
    /// reachable: at 63 elements a message may declare up to 62, and 62 × 4096 is
    /// 253 952 bytes into a 65 536-byte staging buffer. A 7-slot ring can never make a
    /// count above 16 *available*, so a test written against it would be modelling a
    /// bound it cannot reach. See [`REAL_QUEUE_SIZE`].
    #[must_use]
    pub fn new_sized(
        p: Profile,
        pages: Vec<u64>,
        boot_args_gpa: u64,
        rmargs_id: u64,
        size: u32,
    ) -> Guest {
        let msg_size = PAGE as u32;
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
            staging_bytes: STAGING_BYTES,
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
        // the page the table itself starts on (`ogkm-610: message_queue_cpu.c:295-310`,
        // `ogkm-580: message_queue_cpu.c:273-288` — same `memdescGetPhysAddrs` fill).
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
        if self.p.declares_hdr_size() {
            args[32..40].copy_from_slice(&(self.p.hdr() as u64).to_le_bytes());
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
    /// the driver's order (`ogkm-610: src/nvidia/src/libraries/msgq/msgq.c:329-405`,
    /// `ogkm-580: src/common/shared/msgq/msgq.c:330-406` — same checks, same order, same
    /// codes), returning the driver's own code.
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
        // store, because SWAP_RX is agreed (`ogkm-610: msgq.c:416-419, 435-437`,
        // `ogkm-580: msgq.c:417-420, 436-438`).
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
    /// (`ogkm-610: msgq.c:490-496`, `ogkm-580: msgq.c:491-497`), reading the peer's ack
    /// from the status queue's rx header.
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
    /// (`ogkm-610: msgq.c:544-547`, `ogkm-580: msgq.c:545-548`).
    pub fn send(
        &mut self,
        ram: &mut FakeRam,
        function: u32,
        sequence: u32,
        payload: &[u8],
    ) -> Result<u32, &'static str> {
        let rpc_length = 32 + payload.len() as u32;
        let msg_len = self.p.hdr() as u32 + rpc_length;
        let n = msg_len.div_ceil(self.msg_size);
        if n > self.free_space(ram) {
            return Err("no free space");
        }
        let mut buf = vec![0u8; (n * self.msg_size) as usize];
        put32(&mut buf, self.p.seqnum(), self.tx_seq);
        if let Some(o) = self.p.elem_count() {
            put32(&mut buf, o, n);
        }
        if let Some(t) = self.p.mctp() {
            put32(&mut buf, t.header_off, t.header_word);
            put32(&mut buf, t.nvdm_off, t.nvdm_word);
        }
        let h = self.p.hdr();
        put32(&mut buf, h, 0x0300_0000);
        put32(&mut buf, h + 4, 0x4350_5256);
        put32(&mut buf, h + 8, rpc_length);
        put32(&mut buf, h + 12, function);
        put32(&mut buf, h + 24, sequence);
        buf[h + 32..h + 32 + payload.len()].copy_from_slice(payload);
        let sum = fold(&buf, msg_len as usize);
        put32(&mut buf, self.p.checksum(), sum);

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
    /// ## ★★ The element count is **read**, not derived — and only on the versions that
    /// carry the field
    ///
    /// This is the axis on which the two protocols genuinely differ, and modelling it is
    /// what makes any test of the 580 invariants able to fail before the fix.
    ///
    /// | | how many elements the run occupies |
    /// |---|---|
    /// | 580 | `nElements = pMQI->pCmdQueueElement->elemCount` — **the field**, read out of element 0 after it has already been copied to staging (`ogkm-580: message_queue_cpu.c:652-658`), and the value `msgqRxMarkConsumed` advances the ring by (`:774`) |
    /// | 610 | derived from the declared length: `ceil((hdrSize + rpc.length) / elementSizeMin)` (`ogkm-610: message_queue_cpu.c:698-705`, consumed at `:838`) |
    ///
    /// 580 never derives the count from `rpc.length`. Its `msgLen` sanity check
    /// (`ogkm-580: :760-770`) runs *after* the element has been consumed and gates nothing
    /// — it sets a status the `exit:` path then overwrites the ring position regardless.
    ///
    /// The derived value is kept as a separate local so a caller can compare the two;
    /// this model deliberately does **not** cross-check them, because the driver does not.
    ///
    /// # Errors
    ///
    /// The driver's own refusal for the first message that fails.
    ///
    /// # Panics
    ///
    /// If the run the element declares overruns the receive staging buffer — see
    /// [`Guest::staging_bytes`]. A panic and not a refusal: the driver has **no check
    /// here**, and modelling one would be modelling a behaviour it does not have.
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
            let rpc_length = get32(&first, self.p.hdr() + 8);
            let msg_len = self.p.hdr() as u32 + rpc_length;
            // ★★ The lower bound is VERSION-SPLIT, and this `rpc_length < 32` is 580's,
            // not an addition of ours:
            //   580 — `msgLen >= sizeof(GSP_MSG_QUEUE_ELEMENT)`
            //         (`ogkm-580: message_queue_cpu.c:762-770`, mirroring the send check
            //         at `:468-475`). That `sizeof` embeds `rpc_message_header_v` at +48
            //         (`ogkm-580: message_queue_priv.h:43-51`), so 48 + rpc.length >= 80
            //         ⇒ **rpc.length >= 32**. Exactly the test below.
            //   610 — `msgLen >= queueElementHdrSize`
            //         (`ogkm-610: message_queue_cpu.c:826-833`), which **admits
            //         `rpc.length == 0`**, because 610's element ends at a flexible
            //         `payload[]` with no envelope inside it
            //         (`ogkm-610: message_queue_priv.h:52-67`).
            // ⚠ So under `P610` this predicate is STRICTER THAN THE DRIVER. It is left
            // as-is deliberately — the model slices where the driver casts a flexible
            // array, and relaxing it would need a shape change, not a comment — but no
            // test may assert that a 610 guest *rejects* `rpc.length == 0`: it does not.
            //
            // ⚠⚠ AND SO IS ITS POSITION, which is a second divergence and a different one.
            // At BOTH tags the length test is the LAST thing the receive path does and it
            // gates NOTHING: 580 runs it after the checksum and the seqNum, past the retry
            // loop, and the `exit:` label then calls `msgqRxMarkConsumed(nElements)` and
            // `pMQI->rxSeqNum++` regardless of the status it set
            // (`ogkm-580: message_queue_cpu.c:760-786`); 610 is the same shape
            // (`ogkm-610: :826-833`). So a real guest handed a bad-length element
            // CONSUMES it, advances the ring and the sequence, and carries on — while
            // this model returns and consumes nothing. Two consequences, neither of them
            // load-bearing today and both of them assertions nobody may make:
            //   (a) for an element that is bad in more than one way, the driver reports
            //       BadChecksum or BadSequence where this reports BadLength;
            //   (b) a bad-length element is not a wedge on a real guest, and it is here.
            // The two live call sites (`tests/tests/gsp_boot.rs:197, 217`) accept
            // `BadLength` only inside an `Err(A | B)` disjunction, so nothing currently
            // pins the wrong refusal. Straightening it means making the model
            // consume-and-continue, which is the same shape change the `rpc.length` bound
            // above needs — so both are recorded together and neither is guessed at.
            if rpc_length < 32
                || msg_len < self.p.hdr() as u32
                || msg_len > self.p.element_size_max()
            {
                return Err(GuestRefusal::BadLength(rpc_length));
            }
            let derived = msg_len.div_ceil(self.msg_size);
            let n = match self.p.elem_count() {
                // 580: the field, straight out of element 0's header.
                Some(off) => get32(&first, off),
                // 610: there is no such field; the count comes from the declared length.
                None => derived,
            };
            // A declared count of zero is not a refusal in the driver either: element 0
            // has already been copied, the loop then exits, and `msgqRxMarkConsumed(0)`
            // advances the ring by nothing — so the same element is re-read forever
            // (`ogkm-580: message_queue_cpu.c:628, 774`). Loud here, because an
            // instrument that reproduces an infinite loop is a hung CI job.
            assert!(
                n > 0,
                "GUEST RECEIVE WEDGE: an element declaring elemCount = 0 consumes nothing \
                 and is re-read forever (ogkm-580: message_queue_cpu.c:628, 774)",
            );
            // ★ The copy loop stops at the first unavailable element
            // (`msgqRxGetReadBuffer` returns NULL once `n >= available`,
            // `ogkm-580: src/common/shared/msgq/msgq.c:673-693`), so the number of
            // elements that actually reach the staging buffer is `min(n, avail)` — and
            // every one of them advances `pTgt` by `GSP_MSG_QUEUE_ELEMENT_SIZE_MIN` with
            // nothing checking the destination (`ogkm-580: message_queue_cpu.c:648-650`).
            let copied = n.min(avail);
            assert!(
                u64::from(copied) * u64::from(self.msg_size) <= u64::from(self.staging_bytes),
                "GUEST KERNEL HEAP CORRUPTION: an element declaring {n} elements copied \
                 {copied} of them ({} bytes) into a {}-byte staging buffer, overrunning \
                 portMemAllocNonPaged and hitting the live msgq metadata that sits \
                 immediately after it (ogkm-580: message_queue_cpu.c:132-145, 648-650)",
                u64::from(copied) * u64::from(self.msg_size),
                self.staging_bytes,
            );
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
            // ★★ B6 — validate **only what the driver validates**: the MCTP header
            // version nibble and the NVDM vendor id
            // (`ogkm-610: message_queue_cpu.c:737-759` — the whole block; `:762` is
            // already the seqNum test and belongs to the next check). SOM, EOM, the
            // packet sequence
            // and the NVDM *type* byte are written by the sender and never read, so an
            // instrument that enforced the whole words would be stricter than the driver
            // — and a test written against it would assert a behaviour the guest does not
            // have. Do not tighten this without a citation that says the guest reads more.
            if let Some(t) = self.p.mctp()
                && ((get32(&run, t.header_off) & t.header_validated_mask)
                    != (t.header_word & t.header_validated_mask)
                    || (get32(&run, t.nvdm_off) & t.nvdm_validated_mask)
                        != (t.nvdm_word & t.nvdm_validated_mask))
            {
                return Err(GuestRefusal::MctpViolation);
            }
            let seq_num = get32(&run, self.p.seqnum());
            if seq_num != self.rx_seq {
                return Err(GuestRefusal::BadSequence {
                    expected: self.rx_seq,
                    got: seq_num,
                });
            }
            let h = self.p.hdr();
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

/// The XOR fold, re-implemented from `_checkSum32`
/// (`ogkm-610: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:191-209`,
/// `ogkm-580: message_queue_priv.h:106-124` — character for character the same routine;
/// it moved because 610 inserted the CC helpers above it, and 580's copy of that header
/// is only 126 lines long, so `:191-209` does not exist there at all): 64-bit steps to
/// the next 8-byte boundary past `len`, reduced by `hi ^ lo`.
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

/// Which of the two rings an element lives in. Named rather than boolean: a test that
/// corrupts the wrong queue is a test that passes for the wrong reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingId {
    /// The status queue — the ring the faked GSP produces into.
    Status,
    /// The command queue — the ring the guest produces into.
    Command,
}

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
    queue_size: u32,
    next_base: u64,
}

impl GspWorld {
    /// A cold device with a freshly allocated, fragmented guest region and a 7-slot ring.
    #[must_use]
    pub fn new(profile: Profile, model: FakeGspModel) -> GspWorld {
        GspWorld::new_sized(profile, model, SMALL_QUEUE_SIZE)
    }

    /// The same, with the guest's queue size chosen — [`REAL_QUEUE_SIZE`] for the 63-slot
    /// ring the driver actually builds.
    #[must_use]
    pub fn new_sized(profile: Profile, model: FakeGspModel, queue_size: u32) -> GspWorld {
        let mut w = GspWorld {
            ram: FakeRam::default(),
            guest: Guest::new_sized(profile, vec![0], 0, model.rmargs_id, queue_size),
            fsm: GspFsm::new(profile.abi()),
            arch: GspArch::new(model),
            profile,
            queue_size,
            next_base: 0x4000_0000,
        };
        w.allocate_guest_memory();
        w
    }

    /// Give the guest a **fresh, fragmented** region — a new driver life allocates new
    /// memory, and `NV_MEMORY_NONCONTIGUOUS` means the pages are wherever the allocator
    /// had them (`ogkm-610: message_queue_cpu.c:250-256`,
    /// `ogkm-580: message_queue_cpu.c:228-234`).
    /// Give the guest a fresh, fragmented region — what a new driver life allocates.
    pub fn allocate_guest_memory(&mut self) {
        let rmargs_id = self.arch.model().rmargs_id;
        let n =
            Guest::new_sized(self.profile, vec![0], 0, rmargs_id, self.queue_size).region_pages();
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
        self.guest = Guest::new_sized(self.profile, pages, boot_args, rmargs_id, self.queue_size);
        self.guest.publish(&mut self.ram);
    }

    /// Rewrite one word of an element **in the guest's backing store** and repair its
    /// checksum, so what a test exercises is what the receive path does with the *field*
    /// rather than whether it noticed the corruption.
    ///
    /// The repair is the sender's own order (`ogkm-580: message_queue_cpu.c:483, 519`):
    /// zero `checkSum`, fold over `hdrSize + rpc.length`, store.
    ///
    /// `hdr` and `checksum_off` are passed in rather than read from the profile: a test
    /// that took them from the same table as the code under test could not catch a wrong
    /// table entry.
    ///
    /// # Panics
    ///
    /// If the element is not mapped.
    pub fn poke_element(
        &mut self,
        ring: RingId,
        slot: u32,
        hdr: usize,
        checksum_off: usize,
        off: usize,
        val: u32,
    ) {
        let gpa = match ring {
            RingId::Status => self.status_element_gpa(slot),
            RingId::Command => self.command_element_gpa(slot),
        };
        let mut el = vec![0u8; PAGE as usize];
        GuestRam::read(&mut self.ram, gpa, &mut el).expect("the element is mapped");
        el[off..off + 4].copy_from_slice(&val.to_le_bytes());
        el[checksum_off..checksum_off + 4].copy_from_slice(&0u32.to_le_bytes());
        let rpc_length = get32(&el, hdr + 8);
        let sum = fold(&el, hdr + rpc_length as usize);
        el[checksum_off..checksum_off + 4].copy_from_slice(&sum.to_le_bytes());
        GuestRam::write(&mut self.ram, gpa, &el).expect("the element is mapped");
    }

    /// The slot the guest's most recent `send` landed in.
    #[must_use]
    pub fn last_command_slot(&self) -> u32 {
        (self.guest.tx_write_ptr + self.guest.msg_count - 1) % self.guest.msg_count
    }
    /// The guest-physical address of one element of the **status** queue — the ring we
    /// produce into. A test that has to corrupt an element in flight needs it, and
    /// computing it here keeps the ring's geometry in one place.
    #[must_use]
    pub fn status_element_gpa(&self, slot: u32) -> u64 {
        self.guest.gpa_of(
            self.guest.stat_off
                + u64::from(self.guest.entry_off)
                + u64::from(slot) * u64::from(self.guest.msg_size),
        )
    }

    /// Likewise for the **command** queue, which the guest produces into.
    #[must_use]
    pub fn command_element_gpa(&self, slot: u32) -> u64 {
        self.guest.gpa_of(
            self.guest.cmd_off
                + u64::from(self.guest.entry_off)
                + u64::from(slot) * u64::from(self.guest.msg_size),
        )
    }

    /// Write a GSP register, by its abstract name — the test never knows an offset.
    ///
    /// Answers with [`EchoOk`], the C artifact's own policy. See [`GspWorld::wr_with`]
    /// for the form that drives a real one.
    ///
    /// # Errors
    ///
    /// Whatever the transition it triggers refuses with.
    pub fn wr(&mut self, reg: GspReg, val: u64) -> Result<ServiceReport, GspFault> {
        self.wr_with(&mut EchoOk, reg, val)
    }

    /// ★★ **B2** — the same write, answered by a **caller-supplied policy**.
    ///
    /// This is the seam that lets a scripted boot drive the RM object model: pass a
    /// `kayfabe_rmrpc::GraphPolicy` and every command the guest posts is translated and
    /// applied on its way past. The policy is an argument rather than a field because it
    /// borrows the device mutably — `GraphPolicy<'a>` holds `&'a mut Gpu` — and a world
    /// that owned one would own the `Gpu` too, which no other test here wants.
    ///
    /// # Errors
    ///
    /// Whatever the transition it triggers refuses with. ★ A **policy** refusal is not
    /// one of them: the policy answers it with a non-zero `rpc_result` on the wire, and
    /// the transport keeps servicing — which is `GspFault`'s per-message refusal rule
    /// (§7-G8) seen from one level up.
    pub fn wr_with(
        &mut self,
        policy: &mut dyn kayfabe_gsp::CommandPolicy,
        reg: GspReg,
        val: u64,
    ) -> Result<ServiceReport, GspFault> {
        let (bar, off) = self.arch.model().at(reg);
        self.fsm
            .mmio_write(&mut self.ram, &self.arch, policy, bar, off, val)
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
    /// (`ogkm-610: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:522-618`,
    /// `ogkm-580: kernel_gsp_tu102.c:493-578`): FWSEC/STARTCPU, boot-args mailboxes,
    /// Booter Load. Returns every transition that fired.
    ///
    /// ★★ The two bootstraps are **not** the same function with a moved line. 610 sends
    /// the init RPCs from *inside* it, after Booter Load
    /// (`ogkm-610: kernel_gsp_tu102.c:578` calls `kgspSendInitRpcs`); at 580 no such call
    /// exists in this function at all — the same two RPCs are queued *before* bootstrap
    /// by `kgspQueueAsyncInitRpcs_IMPL`. That is the B4 seam, and it is why
    /// [`GspWorld::boot_with`] drains a pre-bind backlog.
    /// Drive the guest's boot and return every transition that fired.
    ///
    /// # Panics
    ///
    /// If any step is refused.
    pub fn boot(&mut self) -> Vec<Transition> {
        self.boot_with(&mut EchoOk)
    }

    /// The same boot, answered by a caller-supplied policy — see [`GspWorld::wr_with`].
    ///
    /// ★ The policy is reached during the boot itself, not only afterwards: `publish`
    /// drains whatever the guest queued **before** the bind (580's two async init RPCs),
    /// so a `GraphPolicy` passed here sees the pre-bootstrap backlog too.
    ///
    /// # Panics
    ///
    /// If any step is refused.
    pub fn boot_with(&mut self, policy: &mut dyn kayfabe_gsp::CommandPolicy) -> Vec<Transition> {
        let m = self.arch.model();
        let mut t = Vec::new();
        t.extend(
            self.wr_with(policy, GspReg::GspFalconCpuctl, m.startcpu())
                .unwrap()
                .transitions,
        );
        let gpa = self.guest.boot_args_gpa;
        t.extend(
            self.wr_with(policy, GspReg::GspFalconMailbox0, gpa & 0xFFFF_FFFF)
                .unwrap()
                .transitions,
        );
        t.extend(
            self.wr_with(policy, GspReg::GspFalconMailbox1, gpa >> 32)
                .unwrap()
                .transitions,
        );
        // Booter Load: a SEC2 STARTCPU whose argument is not the Unload sentinel.
        t.extend(
            self.wr_with(policy, GspReg::Sec2FalconMailbox0, 0)
                .unwrap()
                .transitions,
        );
        t.extend(
            self.wr_with(policy, GspReg::Sec2FalconCpuctl, m.startcpu())
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

    /// The same doorbell, answered by a caller-supplied policy — see
    /// [`GspWorld::wr_with`].
    ///
    /// # Errors
    ///
    /// As [`GspWorld::wr_with`].
    pub fn doorbell_with(
        &mut self,
        policy: &mut dyn kayfabe_gsp::CommandPolicy,
    ) -> Result<ServiceReport, GspFault> {
        self.wr_with(policy, GspReg::GspQueueHead(0), 1)
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
