//! ★★★★★ 2026-10-24 — **the Windows GSP second life on Blackwell, replayed from the logged ordinals.**
//!
//! MEASURED (B1n, lease 57472433, GB205 RTX 5070, Windows 11 LTSC 2024, guest 580.88 keyed as 580.65.06, binary
//! `cb80cbd29`; private capture `on_diag_pnp/vm_logs/on_diag_pnp_b1_qemu.log`):
//! - the FIRST life boots: `w#13`..`w#26` are `kgspResetHw_GH100` (`0x1103c0` = `0x201`, `0`), the FMC-args
//!   mailbox pair (`0x110040` = `0x798d0000`, `0x110044` = `2`), `NV_PGSP_MAILBOX(0)` (`0x110804`), the six
//!   BCR DMA words, `BCR_DMACFG`, `BCR_CTRL` = `0x111` and `RISCV_CPUCTL` = `1` — `_kgspBootstrapGspFmc_GH100`
//!   (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/hopper/kernel_gsp_gh100.c:697-757`) — then ONE
//!   `GSP-PUBLISH`, `Cold -> Booted`, `FALCON_OS` (`0x110080`, w#27), fn 1, `Booted -> Running` (lines 263-275);
//! - after fn 47 (`Running -> Halted`, E9 + E13, line 10919) the SECOND life writes 70 applied words and
//!   publishes FOUR times (lines 10939-10947: one fresh, three `same_instance=true`), stays `Booted`, and sends
//!   no RPC again. 70 = 2 + 9 (FSP CAPS_QUERY) + 2 (doorbells, `QueueNotBound`) + 14 (attempt 1, no
//!   `FALCON_OS`) + 3 × 15 (attempts 2-4, each with `FALCON_OS`): `kgspInitRm_IMPL`'s retry loop at its
//!   default of 4 attempts (`ogkm-580: kernel_gsp.c:3903-3946`; `RmGspBootRetryAttempts` default 4,
//!   `ogkm-595.84: src/nvidia/interface/nvrm_registry.h:2808`). Identical in `on_urt` and `on_tdr60`.
//!
//! ⇒ Attempt 1 fails between its `RISCV_CPUCTL` start and `FALCON_OS`. In `kgspBootstrap_GH100`
//! (`kernel_gsp_gh100.c:833-948`) only two things lie there: `kfspWaitForGspTargetMaskReleased` (HWCFG2
//! non-zero — passes at once here) and `_kgspLockdownReleasedOrFmcError` followed by the `MAILBOX0 != 0`
//! FMC-error check. The test below replays both lives through the REAL Blackwell register model and the
//! REAL boot FSM, stops the second life exactly where the guest's poll runs (every write before the start
//! applied by the drainer, the start itself not yet — BAR0 reads take no exit, so the guest reads the
//! shadow the drainer last published), and asks the RM's own predicate what it sees.
//!
//! ⊘ The second life's own words were not logged (`LIFE_WRITES_LOGGED` = 512 was spent by life 1); the
//! replay gives it the first life's shape at fresh addresses, which is what the guest's `gpa_same=false`
//! and `seq_last=Some(1)` publish says it did.
use kf_arch::gsp::{BootPhase, GspModel};
use kf_gsp::{
    ElementLayout, FunctionCodes, GspAbi, GspFsm, GuestRam, InitArgsLayout, MsgqAbi, OutgoingRpc,
    RamRefused, RpcAbi, Transition, TransportHdr, TxHeader, encode_message,
};
use std::collections::BTreeMap;

// ── the logged register offsets (ogkm-580 headers; the model's own constants are private) ──
const ENGINE: u64 = 0x0011_03c0;
const MAILBOX0: u64 = 0x0011_0040;
const MAILBOX1: u64 = 0x0011_0044;
const PGSP_MAILBOX0: u64 = 0x0011_0804;
const HWCFG2: u64 = 0x0011_00f4;
const FALCON_OS: u64 = 0x0011_0080;
const QUEUE_HEAD0: u64 = 0x0011_0c00;
const RISCV_CPUCTL: u64 = 0x0011_1388;
/// `NV_PFALCON_FALCON_HWCFG2_RISCV_BR_PRIV_LOCKDOWN` (13:13), `_LOCK` = 1 (`ogkm-580: hopper/gh100/dev_falcon_v4.h:79-80`).
const HWCFG2_BR_PRIV_LOCKDOWN: u64 = 1 << 13;
/// `GSP_FMC_BOOT_PARAMS.gspRmParams.bootArgsOffset` (`kf_chip::fsp_gsp::FMC_BOOT_PARAMS_BOOT_ARGS_OFFSET`).
const FMC_BOOT_ARGS_OFF: u64 = kf_chip::fsp_gsp::FMC_BOOT_PARAMS_BOOT_ARGS_OFFSET;
/// `"RMARGS"` as the LibOS region id the model searches for.
const RMARGS_ID: u64 = 0x0000_524d_4152_4753;

// ── the guest's memory, laid out as `kgspSetupGspFmcArgs` / `GspMsgQueueInit` lay it out ──
const PAGE: u64 = 4096;
const ELEM: u64 = PAGE;
const COUNT: u64 = 64;
const CMD_OFF: u64 = 0x1000;
const QSIZE: u64 = ELEM * (1 + COUNT);
const STAT_OFF: u64 = CMD_OFF + QSIZE;
const REGION_PAGES: u64 = (STAT_OFF + QSIZE) / PAGE;
const RX_HDR_OFF: u64 = 32;

/// One driver life's allocations. The first life's FMC-args address is the logged one (`MAILBOX1:MAILBOX0` =
/// `0x2:0x798d0000`); the second life's are fresh, as `gpa_same=false` measured.
#[derive(Clone, Copy)]
struct Life {
    fmc: u64,
    array: u64,
    args: u64,
    region: u64,
}

const LIFE1: Life = Life {
    fmc: 0x2_798d_0000,
    array: 0x2_798e_0000,
    args: 0x2_798f_0000,
    region: 0x2_4500_0000,
};
const LIFE2: Life = Life {
    fmc: 0x2_1f00_0000,
    array: 0x2_1f01_0000,
    args: 0x2_1f02_0000,
    region: 0x2_1f20_0000,
};

#[derive(Default)]
struct Ram(BTreeMap<u64, Box<[u8; PAGE as usize]>>);

impl Ram {
    fn put(&mut self, gpa: u64, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            let at = gpa + i as u64;
            self.0
                .entry(at / PAGE)
                .or_insert_with(|| Box::new([0; PAGE as usize]))[(at % PAGE) as usize] = *b;
        }
    }
    fn put32(&mut self, gpa: u64, v: u32) {
        self.put(gpa, &v.to_le_bytes());
    }
    fn put64(&mut self, gpa: u64, v: u64) {
        self.put(gpa, &v.to_le_bytes());
    }
    fn get32(&mut self, gpa: u64) -> u32 {
        let mut b = [0u8; 4];
        self.read(gpa, &mut b).expect("mapped");
        u32::from_le_bytes(b)
    }
}

impl GuestRam for Ram {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), RamRefused> {
        let len = buf.len();
        for (i, o) in buf.iter_mut().enumerate() {
            let at = gpa + i as u64;
            *o = self
                .0
                .get(&(at / PAGE))
                .map(|p| p[(at % PAGE) as usize])
                .ok_or(RamRefused {
                    gpa,
                    len,
                    why: "not registered",
                })?;
        }
        Ok(())
    }
    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), RamRefused> {
        self.put(gpa, bytes);
        Ok(())
    }
}

/// The Axis-A bundle for the driver this guest is keyed as (580.88 = 580.65.06, one changelist).
fn abi() -> GspAbi {
    let table = kf_abi::versions::table_for(kf_abi::DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    })
    .expect("580.65.06 is a served driver");
    let wire = table.gsp_element_wire();
    let transport = match wire.transport() {
        None => TransportHdr::None,
        Some(t) => TransportHdr::Mctp {
            header_off: t.header_off,
            header_word: t.header_word,
            nvdm_off: t.nvdm_off,
            nvdm_word: t.nvdm_word,
        },
    };
    let element = ElementLayout::new(
        wire.hdr_size(),
        wire.checksum_off(),
        wire.seqnum_off(),
        wire.elem_count_off(),
        transport,
    )
    .expect("element layout");
    let init = table.gsp_init_args_wire();
    GspAbi {
        msgq: MsgqAbi {
            version: 0,
            msg_size_min: 4096,
            swap_rx_flag: 1,
            region_page_size: 4096,
        },
        element,
        rpc: RpcAbi {
            header_version: 0x0300_0000,
            codes: FunctionCodes {
                set_guest_system_info: 1,
                set_guest_system_info_ext: 64,
                free: 10,
                dup_object: 21,
                unloading_guest_driver: 47,
                get_gsp_static_info: 65,
                init_gsp_trace_crash_buffer: 228,
                continuation_record: 71,
                gsp_set_system_info: 72,
                set_registry: 73,
                ecc_notifier_write_ack: 202,
                update_bar_pde: 70,
                gsp_rm_control: 76,
                gsp_rm_alloc: 103,
                gsp_init_done: 0x1001,
                post_event: 0x1002,
                rc_triggered: 0x1003,
                set_page_directory:
                    kf_abi::generated::matrix::RPC_FUNCTIONS_NV_VGPU_MSG_FUNCTION_SET_PAGE_DIRECTORY
                        .everywhere_u32(),
                unset_page_directory:
                    kf_abi::generated::matrix::RPC_FUNCTIONS_NV_VGPU_MSG_FUNCTION_UNSET_PAGE_DIRECTORY
                        .everywhere_u32(),
            },
        },
        element_size_max: table.gsp_element_size_max(),
        init_args: InitArgsLayout {
            shared_mem_pa_off: 0,
            pte_count_off: 8,
            cmd_queue_off_off: 16,
            stat_queue_off_off: 24,
            min_size: init.min_size(),
            element_hdr_size_off: init.element_hdr_size_off(),
        },
        driver: *table,
    }
}

/// The device the way kf3 runs it: the FSM, the Blackwell model, the guest's RAM, and the guest's own
/// command-queue producer state. `write` is the register drainer applying one trapped word in order
/// (`kf_qemu::device::Device::apply_register`), deferred doorbells serviced after it.
struct Dev {
    fsm: GspFsm,
    model: Box<dyn GspModel>,
    ram: Ram,
    policy: kf_gsp::EchoOk,
    life: Life,
    guest_seq: u32,
    guest_wp: u32,
    transitions: Vec<Transition>,
}

impl Dev {
    fn new() -> Dev {
        Dev {
            fsm: GspFsm::new(abi()),
            model: kf_chip::Family::Blackwell
                .gsp_model(0x5, 8192)
                .expect("the Blackwell row"),
            ram: Ram::default(),
            policy: kf_gsp::EchoOk,
            life: LIFE1,
            guest_seq: 0,
            guest_wp: 0,
            transitions: Vec::new(),
        }
    }

    /// `kgspSetupGspFmcArgs` + `GspMsgQueueInit` for a new driver life: a fresh `MESSAGE_QUEUE_INFO`
    /// (`txSeqNum` and the producer's `writePtr` back at 0).
    fn new_life(&mut self, l: Life) {
        self.life = l;
        self.guest_seq = 0;
        self.guest_wp = 0;
        let fsm_abi = abi();
        let il = fsm_abi.init_args;
        self.ram.put64(l.fmc + FMC_BOOT_ARGS_OFF, l.array);
        self.ram.put64(l.array, RMARGS_ID);
        self.ram.put64(l.array + 8, l.args);
        let mut args = vec![0u8; il.min_size.max(64)];
        args[il.shared_mem_pa_off..il.shared_mem_pa_off + 8]
            .copy_from_slice(&l.region.to_le_bytes());
        args[il.cmd_queue_off_off..il.cmd_queue_off_off + 8]
            .copy_from_slice(&CMD_OFF.to_le_bytes());
        args[il.stat_queue_off_off..il.stat_queue_off_off + 8]
            .copy_from_slice(&STAT_OFF.to_le_bytes());
        args[il.pte_count_off..il.pte_count_off + 4]
            .copy_from_slice(&(REGION_PAGES as u32).to_le_bytes());
        self.ram.put(l.args, &args);
        for i in 0..REGION_PAGES {
            self.ram.put64(l.region + i * 8, l.region + i * PAGE);
        }
        // the status ring's consumer header starts at 0 in fresh memory
        self.ram.put(l.region + CMD_OFF, &[0u8; PAGE as usize]);
        self.ram.put(l.region + STAT_OFF, &[0u8; PAGE as usize]);
        let hdr = TxHeader {
            version: 0,
            size: QSIZE as u32,
            msg_size: ELEM as u32,
            msg_count: COUNT as u32,
            write_ptr: 0,
            flags: fsm_abi.msgq.swap_rx_flag,
            rx_hdr_off: RX_HDR_OFF as u32,
            entry_off: ELEM as u32,
        };
        self.ram.put(l.region + CMD_OFF, &hdr.encode());
    }

    /// One trapped BAR0 word, applied by the drainer.
    fn write(&mut self, off: u64, val: u64) {
        if let Ok(r) =
            self.fsm
                .mmio_write_with(&mut self.ram, &*self.model, &mut self.policy, 0, off, val)
        {
            self.transitions.extend(r.transitions);
        }
        while self.fsm.pending_command_doorbells() > 0 {
            match self
                .fsm
                .service_one_deferred_command(&mut self.ram, &mut self.policy)
            {
                Ok(r) => self.transitions.extend(r.transitions),
                Err(_) => break, // `QueueNotBound` while Halted: E8, logged and dropped
            }
        }
    }

    /// What the guest reads from the shadow: the FSM's answer as the drainer last published it.
    fn read(&self, off: u64) -> u64 {
        self.fsm
            .mmio_read_with(&*self.model, 0, off)
            .expect("a GSP register of this model")
            .expect("served")
    }

    /// The guest's own command element (`GspMsgQueueSendCommand`), no doorbell.
    fn queue(&mut self, code: u32, payload: Vec<u8>) {
        let a = abi();
        let rpc = OutgoingRpc {
            function: code,
            sequence: self.guest_seq,
            rpc_result: 0,
            rpc_result_private: 0,
            payload,
        };
        let bytes = encode_message(
            &a.element,
            a.rpc.header_version,
            ELEM as u32,
            a.element_size_max,
            self.guest_seq,
            &rpc,
        )
        .expect("a command that fits");
        let n = bytes.len() as u64 / ELEM;
        for i in 0..n {
            let slot = (u64::from(self.guest_wp) + i) % COUNT;
            let from = (i * ELEM) as usize;
            self.ram.put(
                self.life.region + CMD_OFF + ELEM + slot * ELEM,
                &bytes[from..from + ELEM as usize],
            );
        }
        self.guest_wp = ((u64::from(self.guest_wp) + n) % COUNT) as u32;
        self.ram
            .put32(self.life.region + CMD_OFF + 16, self.guest_wp);
        self.guest_seq += 1;
    }

    /// `rpcSendMessage`: queue, then ring `QUEUE_HEAD(0)`.
    fn send(&mut self, code: u32, payload: Vec<u8>) {
        self.queue(code, payload);
        self.write(QUEUE_HEAD0, 0);
    }

    /// The guest consumed everything in the status ring (its `readPtr`, published by `MSGQ_FLAGS_SWAP_RX`).
    fn drain_status(&mut self) {
        let wp = self.ram.get32(self.life.region + STAT_OFF + 16);
        self.ram.put32(self.life.region + CMD_OFF + RX_HDR_OFF, wp);
    }

    /// `kgspQueueAsyncInitRpcs`: fn 72 and SET_REGISTRY are queued, and their doorbells rung, before the
    /// boot (log: `QueueNotBound` ×2 in the second life, `w#11`/`w#12` in the first).
    fn async_init_rpcs(&mut self) {
        self.send(72, vec![0u8; 936]);
        self.send(73, vec![0u8; 64]);
    }

    /// `kgspResetHw_GH100` + `_kgspBootstrapGspFmc_GH100` up to, NOT including, the `RISCV_CPUCTL` start —
    /// the logged `w#13`..`w#25` words.
    fn bootstrap_fmc_before_start(&mut self) {
        let fmc = self.life.fmc;
        self.write(ENGINE, 0x201); // RESET_ASSERT (as logged)
        self.write(ENGINE, 0x0); // RESET_DEASSERT
        self.write(MAILBOX0, fmc & 0xffff_ffff);
        self.write(MAILBOX1, fmc >> 32);
        self.write(PGSP_MAILBOX0, 0x0);
        for (off, v) in [
            (0x0011_1678, 0x0279_8a72),
            (0x0011_167c, 0x0),
            (0x0011_1680, 0x0279_898a),
            (0x0011_1684, 0x0),
            (0x0011_1670, 0x0279_8980),
            (0x0011_1674, 0x0),
            (0x0011_166c, 0x8000_0001),
            (0x0011_1668, 0x111),
        ] {
            self.write(off, v);
        }
    }

    /// What `kgspBootstrap_GH100` concludes from the registers right now (`kernel_gsp_gh100.c:926-948`):
    /// `Ok(true)` lockdown released and no FMC error, `Ok(false)` still waiting, `Err(mailbox0)` the
    /// "GSP-FMC reported an error while attempting to boot GSP" exit (`NV_ERR_NOT_READY`).
    fn rm_lockdown_poll(&self) -> Result<bool, u64> {
        let mailbox0 = self.read(MAILBOX0);
        // `_kgspLockdownReleasedOrFmcError` (`:491-535`)
        if mailbox0 != 0
            && mailbox0 == self.life.fmc & 0xffff_ffff
            && self.read(MAILBOX1) == self.life.fmc >> 32
        {
            return Ok(false); // the boot args are still in the mailboxes: keep waiting
        }
        let hwcfg2 = self.read(HWCFG2);
        let done = hwcfg2 & HWCFG2_BR_PRIV_LOCKDOWN == 0 || mailbox0 != 0;
        if !done {
            return Ok(false);
        }
        // `:944-948` — the second read of MAILBOX0
        match self.read(MAILBOX0) {
            0 => Ok(true),
            e => Err(e),
        }
    }

    /// The tail of a successful `kgspBootstrap_GH100` and `kgspInitRm`: `FALCON_OS`, the status-queue link
    /// and `GSP_INIT_DONE` drained, then fn 1 (`SET_GUEST_SYSTEM_INFO`).
    fn finish_boot(&mut self) {
        self.write(FALCON_OS, 0);
        self.drain_status();
        self.send(1, vec![0u8; 64]);
        self.drain_status();
    }

    fn publishes(&self) -> usize {
        self.transitions
            .iter()
            .filter(|t| **t == Transition::E6)
            .count()
    }
}

/// The first Windows life on the FMC-direct route, as logged: the poll waits until the start is applied,
/// then passes, and the life runs. (Green before and after the fix: it pins the replay itself.)
#[test]
fn the_first_windows_life_boots_on_the_fmc_direct_route() {
    let mut d = Dev::new();
    d.new_life(LIFE1);
    d.async_init_rpcs();
    d.bootstrap_fmc_before_start();
    assert_eq!(
        d.rm_lockdown_poll(),
        Ok(false),
        "before the start is applied the guest keeps polling (lockdown, no FMC error)"
    );
    d.write(RISCV_CPUCTL, 1);
    assert_eq!(d.publishes(), 1);
    assert_eq!(d.fsm.phase(), BootPhase::Booted);
    assert_eq!(d.rm_lockdown_poll(), Ok(true));
    d.finish_boot();
    assert_eq!(d.fsm.phase(), BootPhase::Running);
}

/// ★ The defect. After fn 47 ends the first life (E9 + E13) the suspend sentinel stays in `MAILBOX0` for
/// `kgspWaitForProcessorSuspend` — and, today, it is still what the FSM serves after the NEXT life has
/// reset the GSP and written its own FMC-args address into that very register. The guest's lockdown poll
/// runs in the window before the drainer has applied its start; it reads `0x80000000`, leaves the poll at
/// once and fails with "GSP-FMC reported an error". On silicon the register holds what the guest wrote.
#[test]
fn the_second_windows_life_never_reads_the_previous_lifes_suspend_sentinel() {
    let mut d = Dev::new();
    d.new_life(LIFE1);
    d.async_init_rpcs();
    d.bootstrap_fmc_before_start();
    d.write(RISCV_CPUCTL, 1);
    d.finish_boot();
    assert_eq!(d.fsm.phase(), BootPhase::Running);

    // fn 47 UNLOADING_GUEST_DRIVER (bInPMTransition = false): E9 then E13.
    d.send(47, vec![0u8; 8]);
    assert_eq!(d.fsm.phase(), BootPhase::Halted);
    assert!(d.transitions.contains(&Transition::E13));
    assert_eq!(
        d.read(MAILBOX0),
        0x8000_0000,
        "kgspWaitForProcessorSuspend still reads the sentinel (kernel_gsp_tu102.c:1215-1239)"
    );

    // The next life (pnputil enable #2, or the TDR's new RM life), in the logged order.
    d.transitions.clear();
    d.new_life(LIFE2);
    d.async_init_rpcs(); // QueueNotBound ×2 (E8), as logged
    d.bootstrap_fmc_before_start();
    let poll = d.rm_lockdown_poll();
    assert!(
        matches!(poll, Ok(false) | Ok(true)),
        "the second life's lockdown poll, before its start is applied, must keep waiting or pass — it read \
         MAILBOX0={poll:?}: the previous life's LIBOS_INTERRUPT_PROCESSOR_SUSPENDED, so \
         _kgspLockdownReleasedOrFmcError returns at once and kgspBootstrap_GH100 fails with NV_ERR_NOT_READY \
         (measured: 4 boot attempts, 4 publishes, no RPC, Code 43 / 0x116)"
    );

    // …and the start then boots it exactly once, fresh, to Running.
    d.write(RISCV_CPUCTL, 1);
    assert_eq!(d.publishes(), 1, "one attempt, one publish");
    assert_eq!(d.rm_lockdown_poll(), Ok(true));
    d.finish_boot();
    assert_eq!(d.fsm.phase(), BootPhase::Running);
}
