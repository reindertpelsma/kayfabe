//! ★★★ **The display plane** — the kf3 device's emulated NVDisplay, running (`docs/design/V3_DISPLAY.md`
//! §4.3–§4.5, steps (2)/(3)).
//!
//! Three threads touch it, and each does only what its constraints allow (`THE_CONSTRAINTS.md`):
//! - **a vCPU** (a BAR0 write into the display aperture): a PUT is posted to the shared
//!   [`Ports`] and the worker is woken — one store and one eventfd write; a cursor PIO method is
//!   posted likewise; an event register's write-1-to-clear is applied to its atomic word and the
//!   ISR's summary registers are republished — a pure bit operation. ⊘ No lock, no guest-memory
//!   access, no host call, and nothing reaches the privileged ring (the GSP state machine owns no
//!   display register).
//! - **the register drainer** (a physical-RM control or an alloc/free RPC): answered by the shared
//!   [`kf_disp::model::DisplayModel`] under its lock, which queues statements for the worker and
//!   wakes it (`kf_rm::display`). The drainer never waits on the worker.
//! - **the display worker** ([`crate::device::Device::display_loop`], its own thread): drains the
//!   statements, reads each channel's pushbuffer out of guest memory, runs the
//!   [`kf_disp::engine::Engine`], and performs its effects IN ORDER — the ARMED mirror first, then the
//!   notifiers and semaphores the guest waits on (written only after the engine applied the update
//!   they report), then GET, then the display interrupt. Guest system memory is read and written
//!   through the RAM QEMU registered; guest video memory (the context-DMA table in display
//!   instance memory, a video-memory notifier) only by the GPU, through the plane's own CUDA context
//!   ([`kf_cuda::display::DisplayGpu`], §38 — the CPU never reads guest vidmem).
//!
//! ⊘ Owner rule A.3: the display is an EMULATED device — no GPU work sits behind its channels
//! (§37) — so its completions are the end of its own processing, and none is written before the
//! state it reports is armed. Vblank is a host timer (the worker's poll deadline) at the armed
//! raster's refresh.
//!
//! ⊘ Hostile guest (only the guest KERNEL allocates display channels): every register offset is
//! decoded against the derived vocabulary, every pushbuffer and context-DMA access is bounded by
//! the guest RAM block or the store it falls in and by the context DMA's own limit, every PUT is a
//! channel number the display has; a malformed stream stops its channel by name.

use crate::device::Device;
use kf_broker::{CursorMode, CursorPoint, CursorWant, FrameCursors, HotTracker, ShownFrame};
use kf_cuda::display::{ComposeLayer, DisplayGpu, Frame};
use kf_disp::engine::{Acquire, Composition, Effect, Engine, PbLoc, ScanVocab, Vocab};
use kf_disp::inst::{CtxDma, Layout, Target};
use kf_disp::lockstep::CreditGate;
use kf_disp::model::{ChannelKind, Statement, Waker};
use kf_disp::pace::{HeadCounts, Mailbox, Meter, NonFlip, Pacer, Sent, flip_completion_need};
use kf_disp::ports::{EventReg, Ports};
use kf_disp::regs::Regs;
use kf_disp::scanout::{LayerPlan, ScanFormats};
use kf_linux_raw::{Notifier, PollTimeout, Poller, ReadyTokens};
use kf_rm::display::SharedDisplayModel;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Heads any family's register file indexes.
const MAX_HEADS: usize = kf_disp::ports::MAX_HEADS;

/// How often [`Device::scanline_loop`] republishes a running raster's position: 250 us, shorter than
/// any vertical blank of a mode this display drives (1080p60 0.67 ms, 1440p144 ~0.3 ms), so a guest
/// polling once per frame sees the blank.
const SCAN_LINE_STEP: Duration = Duration::from_micros(250);
/// …and how often it looks for a running raster when none runs.
const SCAN_LINE_IDLE: Duration = Duration::from_millis(20);

/// ★ 2026-10-08 EXPERIMENT (H-loadv; `KF3_DISPLAY_LOADV=1`, default off): bit 0 of
/// `NV_PDISP_FE_EVT_STAT_HEAD_TIMING(h)`. ⚠ NOT in ogkm's published `dev_disp.h` (v03_00 names only
/// LAST_DATA 1:1, VBLANK 2:2, RG_LINE_A 5:5, RG_LINE_B 6:6), so it is named here by hand, from:
/// nouveau `nvkm/engine/disp/gv100.c` `gv100_disp_intr_head_timing` ("`/* LAST_DATA, LOADV. */`",
/// `stat & 0x00000003`, LAST_DATA being bit 1); and the measurement `[measured, VFIO DVI reference
/// boot3, 2026-10-08, traces/vfio_dvi_reference_20261008]`: on real hardware Windows reads
/// `0x611800` = `0x7` at every head-timing ISR (1902 reads) and `0x5` after it write-1-clears `0x2`
/// (it never clears bit 0), while kf3 publishes `0x6`/`0x4`. The bit is not an enable bit Windows
/// sets (`0x611d80` <- `0x3f0062`), so it never reaches RM by itself; what it changes is what the
/// guest READS. Owner review needed before it may become default (a hand-named field).
pub const EVT_STAT_HEAD_TIMING_LOADV: u32 = 1 << 0;

/// `KF3_DISPLAY_LOADV=1` (read once): the H-loadv experiment ([`EVT_STAT_HEAD_TIMING_LOADV`]).
fn loadv_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_DISPLAY_LOADV").is_ok_and(|v| v == "1"))
}

/// ⚠ DIAGNOSTIC (2026-10-08, `KF3_DISPLAY_WRITE_TRACE=1`, default off): every guest write in the
/// display aperture (BAR0 writes trap by design — no read is trapped), the head-timing interrupts
/// raised and the window latches, each with the host-uptime clock the `maplog` lines use, so they
/// align with the channel/ETW timelines. Bounded: [`display_trace_cap`] lines per kind and run.
fn write_trace_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_DISPLAY_WRITE_TRACE").is_ok_and(|v| v == "1"))
}

/// ⚠ EXPERIMENT (default off, 2026-10-08, H-blankstate; `KF3_DISPLAY_BLANK_STATE=1`, read once).
/// The core channel's `SET_GET_BLANKING_CTRL(h)` (core user area, derived) READS back the head's
/// blanking state on the hardware: `[measured, VFIO DVI reference boot3, RTX 4070, 2026-10-08]`
/// Windows reads all four heads right after the core channel's birth (11.0769 s): `0x2` (UNBLANK)
/// for the head the firmware lit, `0x1` (BLANK) for the three unlit ones — kf3's plain shadow word
/// reads `0` (neither), a value the hardware never returned. Under the experiment a new core life
/// publishes BLANK for every head (kf3 presents no head lit at the core's birth:
/// `SYSTEM_GET_ACTIVE` answers 0); the guest's own writes then overwrite it as before.
fn blank_state_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_DISPLAY_BLANK_STATE").is_ok_and(|v| v == "1"))
}

/// ⚠ EXPERIMENT (default off, 2026-10-08, H-armeddefault; `KF3_DISPLAY_ARMED_DEFAULTS=1`, read
/// once). `[measured, VFIO DVI reference boot3, RTX 4070, 2026-10-08]` Windows reads the core's
/// ARMED `HEAD_SET_MIN_FRAME_IDLE(h)` (`0x68a218 + h·0x400`) 335 times around its first
/// IS_MODE_POSSIBLE and once inside the modeset (12.498437 s, right after BLANK): `0x10002` for all
/// four heads, a value its own pushbuffers never write (`[measured, run96]`: no `0x2218/0x2618/…`
/// method), i.e. the engine's default — the one NVKMS itself programs when IMP gives none
/// (`ogkm-595.84 nvkms-evo3.c:1433-1438`: LEADING_RASTER_LINES 2, TRAILING_RASTER_LINES 1). kf3's
/// ARMED mirror reads `0` until the guest writes the method. Under the experiment a new core life's
/// ARMED mirror starts with that default for every head (field positions derived).
fn armed_defaults_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_DISPLAY_ARMED_DEFAULTS").is_ok_and(|v| v == "1"))
}

/// ⚠ EXPERIMENTS (default off, 2026-10-09, H-mirror; `KF3_DISPLAY_LUT_MIRROR=1`,
/// `KF3_DISPLAY_ILUT_OFFSET_256=1`, read once; `traces/windows_reset_20261009/`). `[measured,
/// runs 98/99, RTX 4070]` Windows' first colour program (window 0 ILUT and head 0 OLUT, control
/// `0x4050a` = DIRECT10 + MIRROR; ILUT offset `0x21`) is refused by the SDR decoder, the refusal
/// halts every display channel, no later flip or core update is consumed, and ~17.5 s later the
/// guest resets the display and bugchecks 0x116. See `kf_disp::color::Experiments` for what each
/// flag changes and why it is exact.
fn color_experiments() -> kf_disp::color::Experiments {
    static X: std::sync::OnceLock<kf_disp::color::Experiments> = std::sync::OnceLock::new();
    *X.get_or_init(|| {
        let on = |n: &str| std::env::var(n).is_ok_and(|v| v == "1");
        let x = kf_disp::color::Experiments {
            lut_mirror: on("KF3_DISPLAY_LUT_MIRROR"),
            ilut_offset_256: on("KF3_DISPLAY_ILUT_OFFSET_256"),
        };
        if x.lut_mirror {
            eprintln!(
                "kf3: display: EXPERIMENT KF3_DISPLAY_LUT_MIRROR=1 — MIRROR accepted on DIRECT ILUT/OLUT where no negative input can reach it"
            );
        }
        if x.ilut_offset_256 {
            eprintln!(
                "kf3: display: EXPERIMENT KF3_DISPLAY_ILUT_OFFSET_256=1 — SET_OFFSET_ILUT read in 256-byte units (as OLUT/TMO)"
            );
        }
        x
    })
}

/// `nvkms-evo3.c:1437-1438`'s default `MIN_FRAME_IDLE` (H-armeddefault): leading, trailing lines.
const MIN_FRAME_IDLE_DEFAULT: (u32, u32) = (2, 1);

/// ⚠⚠ PROBE (default off, 2026-10-09, H-caps; `KF3_DISPLAY_CAPS_PROBE=1`, read once) — never a
/// shipped behaviour, and a CAPTURED table (the defect v3 exists to end), so it may only ever be a
/// probe. `[measured, VFIO DVI reference boot3, RTX 4070, 2026-10-08]` (record
/// `traces/display_reply_diff_20261008/` §0 row P16r) Windows reads the whole caps page
/// (`NV_PDISP_FE_SW`) at 10.9488 s and 0.12 s later initialises only windows 0/2/4/6 — the windows
/// whose `PRECOMP_WIN_PIPE_HDR_CAPA` has the scaler and TMO; the odd windows have CSC11 only — while
/// under kf3 (every window alike) it initialises all eight. 101 of the page's 1024 words differ.
/// Under the probe kf3 publishes the real GPU's page, for its caps class and page base only.
fn caps_probe_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_DISPLAY_CAPS_PROBE").is_ok_and(|v| v == "1"))
}

/// The caps class and page base the probe's page was measured with (RTX 4070, 2026-10-08).
const CAPS_PROBE_AT: (u32, u64) = (0xC773, 0x0064_0000);

/// The real GPU's caps page: every non-zero word, `(byte offset, value)` (VFIO DVI reference boot3,
/// 8-byte reads 10.948826-10.953350 s; RTX 4070, 2026-10-08).
const CAPS_PROBE_WORDS: [(u32, u32); 99] = [
    (0x000, 0x00000f0f),
    (0x004, 0x000000ff),
    (0x008, 0x00000432),
    (0x00c, 0x00000100),
    (0x010, 0x81f22b20),
    (0x018, 0x00001822),
    (0x048, 0x00000a00),
    (0x058, 0x00000a00),
    (0x068, 0x00000a00),
    (0x078, 0x00000a00),
    (0x0c0, 0x00000010),
    (0x0d0, 0x00000010),
    (0x0e0, 0x00000010),
    (0x0f0, 0x00000010),
    (0x144, 0x1b000300),
    (0x14c, 0x1b000300),
    (0x154, 0x1b000300),
    (0x15c, 0x1b000300),
    (0x5e4, 0x00000086),
    (0x5e8, 0x00000086),
    (0x5ec, 0x00000086),
    (0x5f0, 0x00000086),
    (0x5f4, 0x00000086),
    (0x5f8, 0x00000086),
    (0x5fc, 0x00000086),
    (0x600, 0x00000086),
    (0x604, 0x00000086),
    (0x608, 0x003c0051),
    (0x60c, 0x003c0051),
    (0x610, 0x003c0051),
    (0x614, 0x003c0051),
    (0x618, 0x003c0051),
    (0x61c, 0x003c0051),
    (0x620, 0x003c0051),
    (0x624, 0x003c0051),
    (0x680, 0x00ff01d0),
    (0x684, 0x0000d681),
    (0x688, 0x03580e0e),
    (0x68c, 0x0a001400),
    (0x690, 0x00000506),
    (0x694, 0x00001400),
    (0x6a0, 0x00ff01d0),
    (0x6a4, 0x0000d680),
    (0x6a8, 0x03580e0e),
    (0x6ac, 0x0a001400),
    (0x6b0, 0x00000506),
    (0x6b4, 0x00001400),
    (0x6c0, 0x00ff01d0),
    (0x6c4, 0x0000d680),
    (0x6c8, 0x03580e0e),
    (0x6cc, 0x0a001400),
    (0x6d0, 0x00000506),
    (0x6d4, 0x00001400),
    (0x6e0, 0x00ff01d0),
    (0x6e4, 0x0000d680),
    (0x6e8, 0x03580e0e),
    (0x6ec, 0x0a001400),
    (0x6f0, 0x00000506),
    (0x6f4, 0x00001400),
    (0x780, 0x01df21d0),
    (0x784, 0x0000da90),
    (0x788, 0x000e164e),
    (0x78c, 0x5075816a),
    (0x790, 0x000e1a4e),
    (0x794, 0x0a001400),
    (0x7a0, 0x010021d0),
    (0x7a4, 0x0000da90),
    (0x7b0, 0x000e0000),
    (0x7b4, 0x0a001400),
    (0x7c0, 0x01df21d0),
    (0x7c4, 0x0000da90),
    (0x7c8, 0x000e164e),
    (0x7cc, 0x5075816a),
    (0x7d0, 0x000e1a4e),
    (0x7d4, 0x0a001400),
    (0x7e0, 0x010021d0),
    (0x7e4, 0x0000da90),
    (0x7f0, 0x000e0000),
    (0x7f4, 0x0a001400),
    (0x800, 0x01df21d0),
    (0x804, 0x0000da90),
    (0x808, 0x000e164e),
    (0x80c, 0x5075816a),
    (0x810, 0x000e1a4e),
    (0x814, 0x0a001400),
    (0x820, 0x010021d0),
    (0x824, 0x0000da90),
    (0x830, 0x000e0000),
    (0x834, 0x0a001400),
    (0x840, 0x01df21d0),
    (0x844, 0x0000da90),
    (0x848, 0x000e164e),
    (0x84c, 0x5075816a),
    (0x850, 0x000e1a4e),
    (0x854, 0x0a001400),
    (0x860, 0x010021d0),
    (0x864, 0x0000da90),
    (0x870, 0x000e0000),
    (0x874, 0x0a001400),
];

/// H-caps: the measured page (VFIO DVI reference boot3, RTX 4070, 2026-10-08) for the caps class and
/// page base it was measured with, else `None`.
fn caps_probe_page(base: u64, caps_class: u32) -> Option<kf_disp::caps::CapsPage> {
    ((caps_class, base) == CAPS_PROBE_AT).then(|| kf_disp::caps::CapsPage {
        base,
        words: CAPS_PROBE_WORDS.to_vec(),
    })
}

/// Lines per kind the display write trace prints in a run.
const DISPLAY_TRACE_CAP_DEFAULT: u32 = 4096;

/// Lines per kind: [`DISPLAY_TRACE_CAP_DEFAULT`], or `KF3_DISPLAY_TRACE_CAP` (diagnostic only; read once).
fn display_trace_cap() -> u32 {
    static CAP: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("KF3_DISPLAY_TRACE_CAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DISPLAY_TRACE_CAP_DEFAULT)
    })
}

/// Take one of a bounded trace's lines (`false` once `cap` were taken). Lock-free.
fn trace_slot(n: &AtomicU32, cap: u32) -> bool {
    n.fetch_add(1, Ordering::Relaxed) < cap
}

/// ★ One head's FRAME EDGE (the worker's tick for that head), as the hardware latches it: the
/// head-timing events every frame sets — LAST_DATA and VBLANK, plus LOADV under the H-loadv
/// experiment — become PENDING whether or not the guest enabled them (`[measured, VFIO DVI
/// reference boot3, RTX 4070, 2026-10-08]` `0x611800` reads `0x7` while LAST_DATA is disabled), and the display interrupt
/// is due at this edge exactly when an ENABLED bit is pending (`RM_INTR_STAT_HEAD_TIMING`, derived).
/// So: an interrupt at every frame edge while the guest keeps LAST_DATA enabled, none after it
/// disabled it, and none at the enable itself when the guest cleared the bit first (Windows'
/// order on hardware, 113/113: `0x611800` <- `0x2`, then `0x611d80` <- enable; its first interrupt
/// came 0.58-16.87 ms later, at the next frame edge). Returns the RM-visible bits (non-zero =
/// raise the display vector after this tick's effects).
fn frame_edge(ports: &Ports, map: &RegMap, h: usize, loadv: bool) -> u32 {
    let mut bits = map.head_last_data | map.head_vblank;
    if loadv {
        bits |= EVT_STAT_HEAD_TIMING_LOADV;
    }
    ports.raise(EventReg::HeadTiming(h), bits);
    ports.rm_head_timing(h)
}

/// A head's whole frame edge: its frame counters (`RG_DPCA`, LOADV), then [`frame_edge`]. Returns the RM-visible bits.
fn frame_edge_out(
    dp: &DisplayPlane,
    store: &dyn Fn(u64, u32),
    h: usize,
    f: u32,
    loadv: bool,
    wtrace: bool,
    traced: &AtomicU32,
) -> u32 {
    let (b, st, fld) = dp.map.rg_dpca;
    store(b + h as u64 * st, kf_disp::class::put(0, fld, f));
    if let Some((lb, ls)) = dp.map.loadv {
        store(lb + h as u64 * ls, f);
    }
    // ★ 2026-10-14: the raster is at the start of its vertical blank now — the scan-line words read
    // so (VBLANK, line BLANK_START.Y) before the interrupt the guest reads them in.
    let now = dp.ports.now_ns();
    dp.ports.note_frame_edge_at(h, now);
    for (o, v) in dp.map.scan_words(&dp.ports, now, Some(h)) {
        store(o, v);
    }
    let irq = frame_edge(&dp.ports, &dp.map, h, loadv);
    if wtrace && irq != 0 && trace_slot(traced, display_trace_cap()) {
        eprintln!(
            "kf3: display: WTRACE t={:.6} VSYNC h{h} frame={f} evt={:#x} en={:#x} rm={irq:#x}",
            kf_mem::maplog::t(),
            dp.ports.event(EventReg::HeadTiming(h)),
            dp.ports.event(EventReg::HeadTimingEn(h)),
        );
    }
    irq
}

/// ★ 2026-10-14 — **THE VBLANK COUNTER A GSP CLIENT READS.** On a GSP system kernel RM answers
/// `NV0073_CTRL_CMD_SYSTEM_GET_VBLANK_COUNTER` (normal latency) from `kHeadVblankCount[head]` in the
/// display shared-data page it handed physical RM (`MAP_SHARED_DATA`; ogkm
/// `disp_common_kern_ctrl_minimal.c:401-410`, `kern_disp.c:557-627`), and physical RM keeps it. kf3 is
/// physical RM here and never wrote it: the count stayed 0. `[measured run 2990, 2026-10-14]` an OpenGL
/// application with swap interval 1 (`kf_glvsync --swap 1`, Minecraft) never completed its second
/// SwapBuffers; the ICD gave up after 300 s ("Unable to communicate with the display driver", error 2).
/// Writes head `h`'s count (u32, little-endian, at `4*h`, `g_kern_disp_nvoc.h:72-75`) into the page,
/// bounded by the size the guest stated; FB pages through the store, sysmem through guest RAM.
fn write_vblank_count(
    io: &Io<'_>,
    page: kf_disp::model::InstMem,
    h: usize,
    n: u32,
) -> Result<(), String> {
    let off = (h as u64).checked_mul(4).ok_or("head")?;
    if off.checked_add(4).is_none_or(|e| e > page.size) {
        return Err(format!(
            "head {h}'s count is past the {:#x}-byte page",
            page.size
        ));
    }
    let at = page.phys.checked_add(off).ok_or("address")?;
    let bytes = n.to_le_bytes();
    match page.addr_space {
        2 => io
            .gpu
            .as_ref()
            .ok_or("no display GPU context")?
            .write_store(at, &bytes)
            .map_err(|e| e.to_string()),
        1 => {
            let b = io
                .dev
                .ram
                .block_for(at, 4)
                .ok_or_else(|| format!("{at:#x} is not guest RAM"))?;
            b.mem
                .write_from((at - b.gpa) as usize, &bytes)
                .then_some(())
                .ok_or_else(|| "guest RAM write refused".to_string())
        }
        s => Err(format!("address space {s}")),
    }
}

/// `NV_PDISP_FE_CORE_HEAD_STATE(i)`: base, stride, the `OPERATING_MODE` field `(hi, lo)`, and its
/// `AWAKE` and `SLEEP` values.
type CoreHeadState = (u64, u64, (u8, u8), u32, u32);

/// ★ Every BAR0 offset the plane decodes, RESOLVED from the derived register and class tables at
/// realize — so the vCPU path compares integers and a missing name refuses the device up front.
#[derive(Debug, Clone)]
pub struct RegMap {
    lo: u64,
    hi: u64,
    heads: u32,
    windows: u32,
    core_assy: u64,
    core_armed: u64,
    core_len: u64,
    win: (u64, u64),
    winim: (u64, u64),
    curs: (u64, u64),
    user_len: u64,
    put: u64,
    get: u64,
    cursor_free: u64,
    cursor_update: u32,
    evt_awaken_win: u64,
    evt_awaken_other: u64,
    awaken_core_bit: u32,
    evt_sem_win: Option<u64>,
    evt_head_timing: (u64, u64),
    head_last_data: u32,
    head_vblank: u32,
    rm_intr_en_head_timing: Option<(u64, u64)>,
    rm_intr_stat_head_timing: (u64, u64),
    rm_intr_dispatch: u64,
    rm_ctrl_disp: u64,
    rm_ctrl_awaken_bit: u8,
    rm_ctrl_win_sem_bit: Option<u8>,
    evt_dispatch: Option<(u64, u32)>,
    chnctl: [(u64, u64); 4],
    chnctl_alloc: u32,
    chnstatus: [(u64, u64); 4],
    chnstatus_state: [(u8, u8); 4],
    chnstatus_idle: [u32; 4],
    chnstatus_busy: [u32; 4],
    core_head_state: Option<CoreHeadState>,
    rg_dpca: (u64, u64, (u8, u8)),
    loadv: Option<(u64, u64)>,
    /// ★ The core's `GET_RG_SCAN_LINE(h)` (offset in the core user area, stride, the LINE and VBLANK
    /// fields) — a LIVE raster position on the hardware ([`kf_disp::ports::ScanGeom`]).
    rg_scan_line: Option<(u64, u64, (u8, u8), (u8, u8))>,
    /// ★ The window channel's `GET_LINE` (offset in a window's user area, the LINE field): the line
    /// of the raster of the head the window belongs to.
    win_get_line: Option<(u64, (u8, u8))>,
    /// H-blankstate: the core's `SET_GET_BLANKING_CTRL(h)` (offset in the core user area, stride)
    /// and its BLANK read-back value; `None` when the class table lacks the names.
    blanking: Option<(u64, u64, u32)>,
    /// H-armeddefault: the core's `HEAD_SET_MIN_FRAME_IDLE(h)` (method offset, stride) and the
    /// default word; `None` when the class table lacks the names.
    min_frame_idle: Option<(u64, u64, u32)>,
}

fn kind_index(k: ChannelKind) -> usize {
    match k {
        ChannelKind::Core => 0,
        ChannelKind::Window => 1,
        ChannelKind::WindowImm => 2,
        ChannelKind::Cursor => 3,
    }
}

/// What a vCPU write in the display aperture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispWrite {
    /// PUT of DMA channel number `chn`.
    Put(u32),
    /// A write the channel's user area does not accept from the guest (GET, the ARMED half, a
    /// cursor's `Free`): dropped, the register keeps the engine's value.
    ReadOnly,
    /// A cursor PIO method: head, offset inside the cursor's user area.
    Cursor(u32, u32),
    /// An event / interrupt-enable register.
    Event(EventReg),
    /// Any other display register: a plain shadow store, as every B page.
    Plain,
}

impl RegMap {
    /// Resolve the map for a family.
    ///
    /// # Errors
    /// The first name the derived tables lack.
    pub fn resolve(
        r: &Regs,
        t: &kf_disp::class::ClassTable,
        row: &kf_chip::display::DisplayRow,
    ) -> Result<RegMap, String> {
        let miss =
            |n: &str| format!("display register vocabulary: {n} is not derived for this family");
        let v = |n: &str| r.v(n).ok_or_else(|| miss(n));
        let a2 = |n: &str| -> Result<(u64, u64), String> {
            let b = r.a(n, 0).ok_or_else(|| miss(n))?;
            Ok((b, r.a(n, 1).ok_or_else(|| miss(n))? - b))
        };
        let f = |n: &str| r.f32(n).ok_or_else(|| miss(n));
        let bit = |n: &str| -> Result<u32, String> { f(n).map(|(_, lo)| 1u32 << lo) };
        let rt = r.table();
        let (hi, lo) = rt
            .f_in("v04_01", "NV_PDISP")
            .ok_or_else(|| miss("NV_PDISP"))?;
        let core_assy = v("NV_UDISP_FE_CHN_ASSY_BASEADR_CORE")?;
        let core_armed = v("NV_UDISP_FE_CHN_ARMED_BASEADR_CORE")?;
        let win = a2("NV_UDISP_FE_CHN_ASSY_BASEADR_WIN")?;
        let cls = |n: &str| {
            t.v(row.classes.core, n)
                .map(u64::from)
                .ok_or_else(|| miss(&format!("NV{:04X}_{n}", row.classes.core)))
        };
        let chn_state = |k: &str| -> Result<((u8, u8), u32, u32), String> {
            Ok((
                f(&format!("NV_PDISP_FE_CHNSTATUS_{k}_STATE"))?,
                r.v32(&format!("NV_PDISP_FE_CHNSTATUS_{k}_STATE_IDLE"))
                    .ok_or_else(|| miss(k))?,
                r.v32(&format!("NV_PDISP_FE_CHNSTATUS_{k}_STATE_BUSY"))
                    .ok_or_else(|| miss(k))?,
            ))
        };
        let (cs, ci, cb) = chn_state("CORE")?;
        let (ws, wi, wb) = chn_state("WIN")?;
        let (is, ii, ib) = chn_state("WINIM")?;
        let (ks, ki, kb) = chn_state("CURS")?;
        let core_head_state = match r.a("NV_PDISP_FE_CORE_HEAD_STATE", 0) {
            Some(b) => {
                let s = r
                    .a("NV_PDISP_FE_CORE_HEAD_STATE", 1)
                    .ok_or_else(|| miss("CORE_HEAD_STATE"))?
                    - b;
                Some((
                    b,
                    s,
                    f("NV_PDISP_FE_CORE_HEAD_STATE_OPERATING_MODE")?,
                    r.v32("NV_PDISP_FE_CORE_HEAD_STATE_OPERATING_MODE_AWAKE")
                        .ok_or_else(|| miss("OPERATING_MODE_AWAKE"))?,
                    r.v32("NV_PDISP_FE_CORE_HEAD_STATE_OPERATING_MODE_SLEEP")
                        .ok_or_else(|| miss("OPERATING_MODE_SLEEP"))?,
                ))
            }
            None => None,
        };
        let dpca = a2("NV_PDISP_RG_DPCA")?;
        // ⚠ the two experiments' class names (optional: a table without them leaves them off)
        let core = row.classes.core;
        let arr = |n: &str| -> Option<(u64, u64)> {
            let b = u64::from(t.a(core, n, 0)?);
            Some((b, u64::from(t.a(core, n, 1)?).checked_sub(b)?))
        };
        let blanking = arr("SET_GET_BLANKING_CTRL").and_then(|(b, s)| {
            let blank = kf_disp::class::put(
                0,
                t.f(core, "SET_GET_BLANKING_CTRL_BLANK")?,
                t.v(core, "SET_GET_BLANKING_CTRL_BLANK_ENABLE")?,
            );
            Some((b, s, blank))
        });
        let min_frame_idle = arr("HEAD_SET_MIN_FRAME_IDLE").and_then(|(b, s)| {
            let (lead, trail) = MIN_FRAME_IDLE_DEFAULT;
            let w = kf_disp::class::put(
                0,
                t.f(core, "HEAD_SET_MIN_FRAME_IDLE_LEADING_RASTER_LINES")?,
                lead,
            );
            let w = kf_disp::class::put(
                w,
                t.f(core, "HEAD_SET_MIN_FRAME_IDLE_TRAILING_RASTER_LINES")?,
                trail,
            );
            Some((b, s, w))
        });
        let rg_scan_line = arr("GET_RG_SCAN_LINE").and_then(|(b, s)| {
            Some((
                b,
                s,
                t.f(core, "GET_RG_SCAN_LINE_LINE")?,
                t.f(core, "GET_RG_SCAN_LINE_VBLANK")?,
            ))
        });
        let win_get_line = t
            .v(row.classes.window, "GET_LINE")
            .zip(t.f(row.classes.window, "GET_LINE_LINE"))
            .map(|(o, f)| (u64::from(o), f));
        Ok(RegMap {
            lo,
            hi,
            heads: row.heads.min(MAX_HEADS as u32),
            windows: row.windows.min(32),
            core_assy,
            core_armed,
            core_len: 2 * (core_armed - core_assy),
            win,
            winim: a2("NV_UDISP_FE_CHN_ASSY_BASEADR_WINIM")?,
            curs: a2("NV_UDISP_FE_CHN_ASSY_BASEADR_CURS")?,
            user_len: win.1,
            put: cls("PUT")?,
            get: cls("GET")?,
            cursor_free: t
                .v(row.classes.cursor, "FREE")
                .map(u64::from)
                .ok_or_else(|| miss("cursor FREE"))?,
            cursor_update: t
                .v(row.classes.cursor, "UPDATE")
                .ok_or_else(|| miss("cursor UPDATE"))?,
            evt_awaken_win: v("NV_PDISP_FE_EVT_STAT_AWAKEN_WIN")?,
            evt_awaken_other: v("NV_PDISP_FE_EVT_STAT_AWAKEN_OTHER")?,
            awaken_core_bit: bit("NV_PDISP_FE_EVT_STAT_AWAKEN_OTHER_CORE")?,
            evt_sem_win: r.v("NV_PDISP_FE_EVT_STAT_SEM_WIN"),
            evt_head_timing: a2("NV_PDISP_FE_EVT_STAT_HEAD_TIMING")?,
            head_last_data: bit("NV_PDISP_FE_EVT_STAT_HEAD_TIMING_LAST_DATA")?,
            head_vblank: bit("NV_PDISP_FE_EVT_STAT_HEAD_TIMING_VBLANK")?,
            rm_intr_en_head_timing: a2("NV_PDISP_FE_RM_INTR_EN_HEAD_TIMING").ok(),
            rm_intr_stat_head_timing: a2("NV_PDISP_FE_RM_INTR_STAT_HEAD_TIMING")?,
            rm_intr_dispatch: v("NV_PDISP_FE_RM_INTR_DISPATCH")?,
            rm_ctrl_disp: v("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP")?,
            rm_ctrl_awaken_bit: f("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP_AWAKEN")?.1,
            rm_ctrl_win_sem_bit: r
                .f32("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP_WIN_SEM")
                .map(|x| x.1),
            evt_dispatch: r.v("NV_PDISP_FE_EVT_DISPATCH").zip(
                r.f32("NV_PDISP_FE_EVT_DISPATCH_SEM_WIN")
                    .map(|x| 1u32 << x.1),
            ),
            chnctl: [
                (v("NV_PDISP_FE_CHNCTL_CORE")?, 0),
                a2("NV_PDISP_FE_CHNCTL_WIN")?,
                a2("NV_PDISP_FE_CHNCTL_WINIM")?,
                a2("NV_PDISP_FE_CHNCTL_CURS")?,
            ],
            chnctl_alloc: bit("NV_PDISP_FE_CHNCTL_CORE_ALLOCATION")?,
            chnstatus: [
                (v("NV_PDISP_FE_CHNSTATUS_CORE")?, 0),
                a2("NV_PDISP_FE_CHNSTATUS_WIN")?,
                a2("NV_PDISP_FE_CHNSTATUS_WINIM")?,
                a2("NV_PDISP_FE_CHNSTATUS_CURS")?,
            ],
            chnstatus_state: [cs, ws, is, ks],
            chnstatus_idle: [ci, wi, ii, ki],
            chnstatus_busy: [cb, wb, ib, kb],
            core_head_state,
            rg_dpca: (dpca.0, dpca.1, f("NV_PDISP_RG_DPCA_FRM_CNT")?),
            loadv: a2("NV_PDISP_POSTCOMP_HEAD_LOADV_COUNTER").ok(),
            blanking,
            min_frame_idle,
            rg_scan_line,
            win_get_line,
        })
    }

    /// ★ The live scan-line words at `now_ns` as `(BAR0 offset, value)`: every running head's core
    /// `GET_RG_SCAN_LINE(h)` (LINE, VBLANK) and every owned window's `GET_LINE` (its head's LINE);
    /// `only` limits both to one head. Empty for a head with no running raster or no frame edge yet
    /// (its words keep their last value), and for a class table without the words.
    #[must_use]
    pub fn scan_words(&self, ports: &Ports, now_ns: u64, only: Option<usize>) -> Vec<(u64, u32)> {
        let mut pos = [None; MAX_HEADS];
        for (h, p) in pos.iter_mut().enumerate().take(self.heads as usize) {
            if only.is_none_or(|o| o == h) {
                *p = ports.scan_position_at(h, now_ns);
            }
        }
        let mut out = Vec::new();
        if let Some((b, s, lf, vf)) = self.rg_scan_line {
            for (h, p) in pos.iter().enumerate() {
                if let Some((line, vblank)) = *p {
                    let w = kf_disp::class::put(
                        kf_disp::class::put(0, lf, line),
                        vf,
                        u32::from(vblank),
                    );
                    out.push((self.core_assy + b + h as u64 * s, w));
                }
            }
        }
        if let Some((o, lf)) = self.win_get_line {
            for w in 0..self.windows {
                let Some(h) = ports.window_head(w as usize) else {
                    continue;
                };
                if let Some(Some((line, _))) = pos.get(h as usize) {
                    out.push((
                        self.user_base(ChannelKind::Window, w) + o,
                        kf_disp::class::put(0, lf, *line),
                    ));
                }
            }
        }
        out
    }

    /// ★ The words a new core life publishes under the H-blankstate / H-armeddefault experiments,
    /// as `(BAR0 offset, value)`: BLANK in every head's `SET_GET_BLANKING_CTRL`, and the default
    /// `MIN_FRAME_IDLE` in every head's ARMED copy. Empty with both off (the default).
    #[must_use]
    pub fn core_birth_words(&self, blank_state: bool, armed_defaults: bool) -> Vec<(u64, u32)> {
        let mut out = Vec::new();
        for h in 0..u64::from(self.heads) {
            if blank_state && let Some((b, s, v)) = self.blanking {
                out.push((self.core_assy + b + h * s, v));
            }
            if armed_defaults && let Some((b, s, v)) = self.min_frame_idle {
                out.push((self.core_armed + b + h * s, v));
            }
        }
        out
    }

    /// Is BAR0 offset `off` in the display aperture (`NV_PDISP`)?
    #[must_use]
    pub fn owns(&self, off: u64) -> bool {
        off >= self.lo && off <= self.hi
    }

    /// The user area of channel `(kind, instance)`.
    #[must_use]
    pub fn user_base(&self, kind: ChannelKind, i: u32) -> u64 {
        match kind {
            ChannelKind::Core => self.core_assy,
            ChannelKind::Window => self.win.0 + u64::from(i) * self.win.1,
            ChannelKind::WindowImm => self.winim.0 + u64::from(i) * self.winim.1,
            ChannelKind::Cursor => self.curs.0 + u64::from(i) * self.curs.1,
        }
    }

    fn in_array(&self, off: u64, (base, stride): (u64, u64), n: u32) -> Option<(u32, u64)> {
        let rel = off.checked_sub(base)?;
        let i = rel / stride;
        (i < u64::from(n)).then(|| (i as u32, rel % stride))
    }

    /// ★ **vCPU**: what a write at `off` is (pure arithmetic on resolved offsets).
    #[must_use]
    pub fn classify(&self, off: u64) -> DispWrite {
        if (self.core_assy..self.core_assy + self.core_len).contains(&off) {
            let rel = off - self.core_assy;
            return if rel == self.put {
                DispWrite::Put(0)
            } else if rel == self.get || rel >= self.core_armed - self.core_assy {
                DispWrite::ReadOnly
            } else {
                DispWrite::Plain
            };
        }
        for (arr, first) in [(self.win, 1u32), (self.winim, 33u32)] {
            if let Some((i, rel)) = self.in_array(off, arr, self.windows) {
                return if rel == self.put {
                    DispWrite::Put(first + i)
                } else if rel == self.get || rel >= self.user_len / 2 {
                    DispWrite::ReadOnly
                } else {
                    DispWrite::Plain
                };
            }
        }
        if let Some((h, rel)) = self.in_array(off, self.curs, self.heads) {
            return if rel == self.cursor_free || rel >= self.user_len / 2 {
                DispWrite::ReadOnly
            } else {
                DispWrite::Cursor(h, rel as u32)
            };
        }
        if off == self.evt_awaken_win {
            return DispWrite::Event(EventReg::AwakenWin);
        }
        if off == self.evt_awaken_other {
            return DispWrite::Event(EventReg::AwakenOther);
        }
        if Some(off) == self.evt_sem_win {
            return DispWrite::Event(EventReg::SemWin);
        }
        if let Some((h, 0)) = self.in_array(off, self.evt_head_timing, self.heads) {
            return DispWrite::Event(EventReg::HeadTiming(h as usize));
        }
        if let Some((h, 0)) = self
            .rm_intr_en_head_timing
            .and_then(|a| self.in_array(off, a, self.heads))
        {
            return DispWrite::Event(EventReg::HeadTimingEn(h as usize));
        }
        // the ISR's summary registers are read-only (derived)
        if off == self.rm_ctrl_disp
            || off == self.rm_intr_dispatch
            || self
                .in_array(off, self.rm_intr_stat_head_timing, self.heads)
                .is_some_and(|(_, r)| r == 0)
        {
            return DispWrite::ReadOnly;
        }
        DispWrite::Plain
    }
}

/// One head's cursor PIO registers, posted by vCPUs (the latest value per register, and an
/// `Update` count the worker follows).
#[derive(Debug, Default)]
pub struct CursorPorts {
    regs: [AtomicU32; 8],
    offs: [AtomicU32; 8],
    updates: AtomicU32,
}

impl CursorPorts {
    /// ★ **vCPU**: post a cursor method write. `update` = this was the `Update` method.
    fn post(&self, rel: u32, v: u32, update: bool) {
        if update {
            self.updates.fetch_add(1, Ordering::AcqRel);
            return;
        }
        // a small fixed register file: the slot of `rel`, or the first free one
        for i in 0..self.offs.len() {
            let o = self.offs[i].load(Ordering::Acquire);
            if o == rel + 1
                || (o == 0
                    && self.offs[i]
                        .compare_exchange(0, rel + 1, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok())
            {
                self.regs[i].store(v, Ordering::Release);
                return;
            }
        }
    }
}

/// ★ MEASUREMENT ONLY (hitch investigation, 2026-10-17; never a decision input, no behaviour): a bounded latency
/// meter — count, sum and max of one step's wall time, three relaxed atomics, no allocation, no lock.
#[derive(Debug)]
pub struct LatStat {
    enabled: bool,
    n: AtomicU64,
    sum_ns: AtomicU64,
    max_ns: AtomicU64,
}

impl Default for LatStat {
    fn default() -> Self {
        Self::with_diagnostics(false)
    }
}

impl LatStat {
    fn with_diagnostics(enabled: bool) -> Self {
        Self {
            enabled,
            n: AtomicU64::new(0),
            sum_ns: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
        }
    }

    fn stamp(&self) -> Option<Instant> {
        self.start_with(Instant::now)
    }

    fn start_with(&self, clock: impl FnOnce() -> Instant) -> Option<Instant> {
        self.enabled.then(clock)
    }

    fn finish(&self, started: Option<Instant>) {
        self.finish_with(started, Instant::now);
    }

    fn finish_with(&self, started: Option<Instant>, clock: impl FnOnce() -> Instant) {
        if self.enabled
            && let Some(started) = started
        {
            self.add(clock().saturating_duration_since(started));
        }
    }

    /// One sample.
    pub fn add(&self, d: Duration) {
        if !self.enabled {
            return;
        }
        let ns = u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
        self.n.fetch_add(1, Ordering::Relaxed);
        self.sum_ns.fetch_add(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    /// `name=n/sum_us/max_us` — cumulative; a reader takes window deltas of `n` and `sum_us`.
    #[must_use]
    pub fn fragment(&self, name: &str) -> String {
        let o = Ordering::Relaxed;
        format!(
            "{name}={}/{}/{}",
            self.n.load(o),
            self.sum_ns.load(o) / 1000,
            self.max_ns.load(o) / 1000
        )
    }
}

/// Boot-log counters.
#[derive(Debug, Default)]
pub struct DispCounters {
    /// ★ MEASUREMENT: display-worker pass start → a vblank edge (event + interrupt) raised.
    pub lat_edge: LatStat,
    /// ★ MEASUREMENT: `ScanState::resolve_now` (the context-DMA resolve at a latch).
    pub lat_resolve: LatStat,
    /// ★ MEASUREMENT: `ScanState::start` (launching a scanout copy).
    pub lat_start: LatStat,
    /// ★ MEASUREMENT: display-worker pass start → end of the irq step (the whole pass up to the interrupt).
    pub lat_pass: LatStat,
    /// Display-aperture writes from vCPUs.
    pub writes: AtomicU64,
    /// Updates the engine completed.
    pub updates: AtomicU64,
    /// Methods executed.
    pub methods: AtomicU64,
    /// Notifiers written.
    pub notifies: AtomicU64,
    /// Semaphores released.
    pub releases: AtomicU64,
    /// Channel exceptions.
    pub exceptions: AtomicU64,
    /// Effects refused (a context DMA that did not resolve, an access outside its bounds).
    pub refused: AtomicU64,
    /// Display interrupts sent.
    pub irqs: AtomicU64,
    /// Vblanks ticked.
    pub vblanks: AtomicU64,
    /// Scanout copies completed (M2): frames the console can show.
    pub scanouts: AtomicU64,
    /// Scanouts refused (a surface the console cannot copy, by name in the log).
    pub scanout_refused: AtomicU64,
    /// ★ Windows left out of a console copy because the console cannot compose them (a format with
    /// no console pixel format, a context DMA that does not resolve, ...). Console-only: the
    /// guest's flip of that window still completes.
    pub console_windows_left_out: AtomicU64,
    /// ★ Console copies given up for a console-only reason (a colour program the console cannot
    /// build). The flips behind them completed; the console kept its last frame.
    pub console_copies_skipped: AtomicU64,
    /// Microseconds from queueing a scanout copy to observing its completion: the sum and the max.
    pub scanout_us_total: AtomicU64,
    /// The longest one.
    pub scanout_us_max: AtomicU64,
    /// ★ Copies with no free slot — an invariant violation of the frame ring's cap argument
    /// (`kf_broker::slots`), counted rather than papered over with a slot someone reads.
    pub scanout_no_slot: AtomicU64,
    /// ★ GPU-copy rung (§8.11): frames copied to host memory (D2H) …
    pub scanout_d2h: AtomicU64,
    /// … frames packed into a VRAM slot (no byte to the CPU) …
    pub scanout_pack: AtomicU64,
    /// … frames the broker wanted in VRAM but no VRAM slot could take (none provisioned or large
    /// enough yet, or every free one still fenced) …
    pub scanout_pack_skipped: AtomicU64,
    /// … and the VRAM provisioned for slots, in bytes.
    pub vram_bytes: AtomicU64,
    /// ★ §O, the host cursor: guest cursor images copied for the broker (a GPU copy into a buffer
    /// kf owns, one per frame while a cursor-capable broker is attached) …
    pub host_cursor_reads: AtomicU64,
    /// … and cursors the host could not show (XOR, an additive blend, an unreadable surface),
    /// composed into the frame in every mode instead.
    pub host_cursor_refused: AtomicU64,
    /// ★ The boot display: copies started from the boot layer (`gop=on`).
    pub boot_frames: AtomicU64,
    /// ★ Milliseconds from the worker's start to the first armed head, when the boot layer retired
    /// (at least 1; 0 while it is still shown, or without one).
    pub boot_done_ms: AtomicU64,
    /// ★ `display-max-fps` D2 (§8.16): non-flip checks started — a composition and its checksum at
    /// the console head's tick while watched …
    pub checks: AtomicU64,
    /// … of those, checks whose frame was what the last published one was (nothing sent) …
    pub same: AtomicU64,
    /// … and screendump/console refresh requests served.
    pub ondemand: AtomicU64,
}

impl DispCounters {
    /// Construct the four diagnostic meters with one explicit selection; ordinary counters stay on.
    #[must_use]
    pub fn with_step_meters(enabled: bool) -> Self {
        Self {
            lat_edge: LatStat::with_diagnostics(enabled),
            lat_resolve: LatStat::with_diagnostics(enabled),
            lat_start: LatStat::with_diagnostics(enabled),
            lat_pass: LatStat::with_diagnostics(enabled),
            ..Self::default()
        }
    }

    /// The legacy latency fragment only when the diagnostic was selected at construction.
    #[must_use]
    pub fn step_fragment(&self) -> String {
        if !self.lat_pass.enabled {
            return String::new();
        }
        format!(
            " lat[{} {} {} {}]",
            self.lat_edge.fragment("edge"),
            self.lat_resolve.fragment("resolve"),
            self.lat_start.fragment("start"),
            self.lat_pass.fragment("pass")
        )
    }
}

/// ★ §8.16: the console or the broker WATCHES while it asked for a frame within this many
/// milliseconds — non-flip copies are made only then (D2.3), and its return after longer is a new
/// watcher.
const WATCHED_MS: u64 = 2000;

/// The most console frame slots ([`kf_broker::slots::MAX_SLOTS`]): three with the broker off
/// (one shown, one ready, one the GPU fills — today's triple buffer), five with it on (two more
/// the broker may hold, `docs/design/V3_DISPLAY.md` §8.3).
const SLOTS: usize = kf_broker::slots::MAX_SLOTS;

/// ★ What the console reads: a frame's host address, geometry, format and serial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameView {
    /// Host address of the first pixel (page-locked memory the worker owns for the process).
    pub addr: usize,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
    /// [`kf_disp::scanout::PixelFormat`] code.
    pub format: u32,
    /// Increases with every frame the worker publishes.
    pub serial: u64,
    /// ★ §8.13: the guest's cursor is composed into this frame.
    pub cursor: bool,
}

/// ★★ M2 — the frames the display worker hands QEMU's console (and, with `display-broker`, the
/// broker relay), lock-free (`V3_DISPLAY.md` §4.6, §8.3).
///
/// The occupancy is ONE atomic word, [`kf_broker::FrameRing`]: the console takes the ready frame
/// as its new front ([`ConsoleShare::take`]); the worker fills a slot named nowhere in the word
/// ([`ConsoleShare::free_slot`]), then publishes it as ready — for the console and, when the
/// broker is on, for the relay too — dropping an untaken older one. Nobody can name the slot
/// the GPU is writing, so it is never the one on screen nor one the broker reads.
/// ⊘ Frame memory is never freed while the device lives (a screendump may still hold a pixman image
/// of an old front after the console moved on — a stale read is harmless, a freed page is not).
#[derive(Debug)]
pub struct ConsoleShare {
    ring: Arc<kf_broker::FrameRing>,
    /// Per slot: the host address of its current frame (the console's view of the pixels).
    addrs: [AtomicUsize; SLOTS],
    /// Per slot: the [`kf_disp::scanout::PixelFormat`] code of its frame.
    formats: [AtomicU32; SLOTS],
    /// Milliseconds (since the plane's start) of the console's last request — the refresh rate
    /// follows demand, and so does the D2H copy.
    demand_ms: AtomicU64,
    /// ★ The same for the broker's activity (§8.11): it keeps the refresh rate, but feeds the D2H
    /// copy only while the broker is shown through host memory (`kf_broker::gpucopy::plan`).
    broker_ms: AtomicU64,
    /// ★ §8.13: the shown head's cursor image top-left on the console's frame — the worker writes
    /// it every pass, the console's cursor (`kf3_display_cursor`) adds the image's hot spot to it.
    cursor_point: CursorPoint,
    /// ★ §8.13 (the review, 2026-10-04): which frames have the guest's cursor composed in, and
    /// whether the one the console took last does — the console's cursor follows the frame it
    /// SHOWS, never the relay's mode alone.
    cursors: FrameCursors,
    epoch: Instant,
    /// ★ `display-max-fps` D2.3 (§8.16): refresh requests from the console's `gfx_update` (a
    /// `screendump` among them; main loop) …
    refresh_req: AtomicU64,
    /// … the last one served (worker) …
    refresh_served: AtomicU64,
    /// … and the descriptor the worker signals when that advances (the C device's fd handler then
    /// shows the newest frame and ends the screendump's wait). `None`: none could be made — every
    /// request is answered at once by the C device.
    refreshed: Option<Notifier>,
    /// ★ Bumped when a watcher STARTS (the console's or the broker's first request after 2 s
    /// without, a broker session that became active): its view may be stale, so the next check
    /// sends whatever it finds.
    watch_epoch: AtomicU64,
}

impl Default for ConsoleShare {
    fn default() -> ConsoleShare {
        ConsoleShare::over(Arc::new(kf_broker::FrameRing::new(
            kf_broker::slots::CONSOLE_SLOTS,
            false,
        )))
    }
}

impl ConsoleShare {
    /// The console over `ring` (5 slots that feed the broker, or 3 that do not).
    #[must_use]
    pub fn over(ring: Arc<kf_broker::FrameRing>) -> ConsoleShare {
        ConsoleShare {
            ring,
            addrs: core::array::from_fn(|_| AtomicUsize::new(0)),
            formats: core::array::from_fn(|_| AtomicU32::new(0)),
            demand_ms: AtomicU64::new(0),
            broker_ms: AtomicU64::new(0),
            cursor_point: CursorPoint::default(),
            cursors: FrameCursors::default(),
            epoch: Instant::now(),
            refresh_req: AtomicU64::new(0),
            refresh_served: AtomicU64::new(0),
            refreshed: Notifier::create().ok(),
            watch_epoch: AtomicU64::new(0),
        }
    }

    /// The occupancy ring (the broker relay shares it).
    #[must_use]
    pub fn ring(&self) -> &Arc<kf_broker::FrameRing> {
        &self.ring
    }

    /// Milliseconds since the plane's start (the console cursor's pacing clock, too).
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// ★ **Worker**, every pass: the shown head's cursor image top-left on the frame, or `None`.
    fn note_cursor_point(&self, p: Option<(i32, i32)>) {
        self.cursor_point.set(p);
    }

    /// ★ **Console (QEMU's main thread)**: the point [`ConsoleShare::note_cursor_point`] last stored.
    #[must_use]
    pub fn cursor_point(&self) -> Option<(i32, i32)> {
        self.cursor_point.get()
    }

    /// ★ **Console (QEMU's main thread)**: what the frame [`ConsoleShare::take`] last handed the
    /// console carries — whether the guest's cursor is composed into it. (`kf3_gfx_update` shows
    /// every frame it takes; the frame it would refuse — a bad format or geometry — the worker
    /// never makes.)
    #[must_use]
    pub fn shown_frame(&self) -> ShownFrame {
        self.cursors.shown()
    }

    /// Record the console's request for frames now. ★ §8.16: a console that had not asked for 2 s
    /// is a NEW watcher (a VNC client connected, a screendump).
    pub fn note_demand(&self) {
        if !self.within(&self.demand_ms, WATCHED_MS) {
            self.watch_epoch.fetch_add(1, Ordering::Relaxed);
        }
        self.demand_ms
            .store(self.now_ms().max(1), Ordering::Relaxed);
    }

    /// ★ Record an active broker now (`kf3_broker_ready`) — frames are wanted (the refresh rate),
    /// the host copy only if the broker is fed through host memory. ★ §8.16: a broker that had
    /// not been active for 2 s is a new watcher.
    pub fn note_broker_demand(&self) {
        if !self.within(&self.broker_ms, WATCHED_MS) {
            self.watch_epoch.fetch_add(1, Ordering::Relaxed);
        }
        self.broker_ms
            .store(self.now_ms().max(1), Ordering::Relaxed);
    }

    /// ★ §8.16 (main loop): a broker SESSION became active (a reconnect inside 2 s is still a new
    /// viewer with an empty window) — the next check sends.
    pub fn note_new_watcher(&self) {
        self.watch_epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// The new-watcher counter (worker).
    #[must_use]
    pub fn watch_epoch(&self) -> u64 {
        self.watch_epoch.load(Ordering::Relaxed)
    }

    /// ★ D2.3 (main loop, the console's `gfx_update`): ask for a frame no older than now. `false`
    /// when no answer can come (no descriptor): the caller answers its waiter itself.
    #[must_use]
    pub fn request_refresh(&self) -> bool {
        if self.refreshed.is_none() {
            return false;
        }
        self.refresh_req.fetch_add(1, Ordering::AcqRel);
        true
    }

    /// The request counter (worker).
    #[must_use]
    pub fn refresh_requested(&self) -> u64 {
        self.refresh_req.load(Ordering::Acquire)
    }

    /// ★ Worker: every request up to `upto` is served — the frame published (or found unchanged)
    /// by a copy that started after it is the ready one. One non-blocking descriptor write.
    fn serve_refresh(&self, upto: u64) {
        self.refresh_served.fetch_max(upto, Ordering::AcqRel);
        if let Some(n) = &self.refreshed {
            let _ = n.signal();
        }
    }

    /// The last request served.
    #[must_use]
    pub fn refresh_served(&self) -> u64 {
        self.refresh_served.load(Ordering::Acquire)
    }

    /// The descriptor the C device watches for served requests, or -1.
    #[must_use]
    pub fn refresh_fd(&self) -> i32 {
        use std::os::fd::AsRawFd as _;
        self.refreshed
            .as_ref()
            .map_or(-1, |n| n.as_source_fd().as_raw_fd())
    }

    /// Main loop: consume the descriptor's readiness.
    pub fn refresh_drain(&self) {
        if let Some(n) = &self.refreshed {
            let _ = n.drain();
        }
    }

    /// ★ **Console (QEMU's main thread)**: the newest frame — the ready one becomes the front — or
    /// `None` before the first. The returned memory stays valid and unwritten until the next call.
    pub fn take(&self) -> Option<FrameView> {
        self.note_demand();
        let slot = self.ring.take_console()?;
        let g = self.ring.geometry(slot);
        let cursor = self.cursors.took(slot);
        Some(FrameView {
            addr: self.addrs[slot].load(Ordering::Acquire),
            width: g.width,
            height: g.height,
            stride: g.stride,
            format: self.formats[slot].load(Ordering::Acquire),
            serial: g.serial,
            cursor,
        })
    }

    /// **Worker**: a slot named nowhere in the occupancy word — the next copy's target. `None`
    /// breaks the ring's cap argument and is the caller's counted fault, never a fallback.
    fn free_slot(&self) -> Option<usize> {
        self.ring.fill_target(None)
    }

    /// **Worker**: describe slot `i`'s finished frame — in its host frame when `host` (the D2H
    /// copy ran), in its VRAM slot when `vram` (the pack ran) — then make it the ready one.
    fn publish(&self, i: usize, f: FrameView, host: bool, vram: Option<kf_broker::VramGeom>) {
        if i >= SLOTS {
            return;
        }
        if host {
            self.addrs[i].store(f.addr, Ordering::Release);
            self.formats[i].store(f.format, Ordering::Release);
        }
        // ★ the bit belongs to the frame whichever backing carries it: the console reads it with the
        // host frame, the relay (the broker's host cursor) with the committed slot, VRAM-only too
        self.cursors.publish(i, f.cursor);
        self.ring.describe(
            i,
            kf_broker::FrameGeom {
                width: f.width,
                height: f.height,
                stride: f.stride,
                // the console's x8r8g8b8 byte order is DRM's XRGB8888 (`kf3.c`'s pixman map)
                fourcc: kf_broker::wire::FOURCC_XR24,
                serial: f.serial,
                cursor: f.cursor,
            },
        );
        self.ring.describe_backings(i, host, vram);
        self.ring.publish(i);
    }

    fn within(&self, at: &AtomicU64, ms: u64) -> bool {
        let last = at.load(Ordering::Relaxed);
        last != 0 && self.now_ms().saturating_sub(last) <= ms
    }

    /// Did the console OR an active broker ask for a frame within the last `ms` milliseconds?
    /// (Whether anybody watches, §8.16.)
    fn wanted_within(&self, ms: u64) -> bool {
        self.within(&self.demand_ms, ms) || self.within(&self.broker_ms, ms)
    }

    /// Did the console ask within the last `ms` milliseconds?
    fn console_wanted_within(&self, ms: u64) -> bool {
        self.within(&self.demand_ms, ms)
    }

    /// Was the broker active within the last `ms` milliseconds?
    fn broker_wanted_within(&self, ms: u64) -> bool {
        self.within(&self.broker_ms, ms)
    }
}

/// ★ The boot display's picture (`gop=on`, `docs/design/V3_DISPLAY.md` §4.11.2): the boot layer
/// (`kf_disp::scanout::boot_layer`, store `[0, G)`) and the frame size it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootScan {
    /// The one layer: an opaque pitch copy of store `[0, G)`.
    pub layer: LayerPlan,
    /// The frame's width and height (the mode's).
    pub size: (u32, u32),
}

impl BootScan {
    /// The boot layer for a `gop=on` plan.
    ///
    /// # Errors
    /// `kf_disp::scanout::boot_layer`'s refusal, by name.
    pub fn of(plan: &crate::gop::BootPlan) -> Result<BootScan, String> {
        let s = plan.surface();
        let layer = kf_disp::scanout::boot_layer(&s).map_err(|r| format!("gop=on: {}", r.0))?;
        Ok(BootScan {
            layer,
            size: (s.width, s.height),
        })
    }
}

/// ★ What the console shows on one scanout copy.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shown {
    /// The lowest running head's composition (every enabled window it owns) — the guest's own.
    Armed(Composition),
    /// The boot layer, until the guest arms its first head (`gop=on`).
    Boot(LayerPlan, (u32, u32)),
    /// ★ 2026-10-03 (B5, `V3_DISPLAY.md` §4.11.13): the scanout the guest freed its channels from
    /// with `PRESERVE_HW` (NVKMS, after restoring the console, `nvkms-rm.c:2990-3017`) — planned
    /// layers of the last armed composition, kept: the hardware keeps scanning them.
    Preserved(Vec<LayerPlan>, (u32, u32)),
    /// ★ 2026-10-03 (B5): nothing is scanned (no head lit, or a lit head with no window): the
    /// monitor shows black, never the last frame.
    Blank((u32, u32)),
}

/// ★ What the display shows when no armed head scans a window (`V3_DISPLAY.md` §4.11.13).
#[derive(Debug, Default)]
struct Held {
    /// The scanout the guest freed with `PRESERVE_HW`, until a head is armed again.
    preserved: Option<(Vec<LayerPlan>, (u32, u32))>,
    /// ★ The size of the last ARMED composition chosen — `None` until a guest head's scanout was
    /// shown at all. Only a scanout that was shown can be lost: the boot layer → first armed head
    /// handoff never sets it, and neither does a family without a window vocabulary (GB20x, until
    /// M5), so neither ever goes black.
    scanned: Option<(u32, u32)>,
    /// What the guest's heads say now.
    dark: Dark,
}

impl Held {
    /// ★ B5: a channel freed with `PRESERVE_HW` keeps `last` — the last armed composition a copy
    /// composed whole ([`ScanState::last_plan`]) — on the monitor until a head is armed again. The
    /// first preserving free that finds one decides; the others change nothing.
    fn channel_freed(&mut self, preserve: bool, last: Option<&(Vec<LayerPlan>, (u32, u32))>) {
        if preserve
            && self.preserved.is_none()
            && let Some(plan) = last
        {
            self.preserved = Some(plan.clone());
        }
    }
}

/// Window channels any family has (`chan_of`: channel numbers 1..=32).
const MAX_WINDOWS: usize = 32;

/// ★★ 2026-10-04 (B5 on box vmb, `V3_DISPLAY.md` §4.11.13): the ISO context DMA each window's ARMED
/// state was resolved to — kept until that window latches again, because the display engine scans
/// the surface its ARMED state latched, not whatever the instance-memory hash says on a later frame.
///
/// NVKMS depends on that: tearing down after X it restores the console, then frees the console
/// surface — `FreeSurfaceEvoRm` → `nvCtxDmaFree`, and RM clears the context DMA's hash entry and
/// object from display instance memory (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/context_dma.c:356-357`,
/// `src/nvidia/src/kernel/gpu/disp/inst_mem/disp_inst_mem.c:861-930`) — with the window still armed
/// on it (`skipUpdate`: no UPDATE), and only THEN frees the window and core channels with
/// `PRESERVE_HW` *"to avoid shutting down the heads we just enabled"* (`nvFreeDevEvo`,
/// `src/nvidia-modeset/src/nvkms-evo.c:9101-9112`; `nvkms-rm.c:2990-3017`). Resolved per copy, a
/// refresh copy landing between that unbind and the frees (`[measured vmb, 8a682f1b and d4c3767b]`
/// 346 ms to 2.2 s, a copy every 250 ms) refused the window — *"scanout REFUSED context DMA 0x10088
/// on channel 7: NotBound"* — the copy was no longer whole, the preserving free found no plan, and
/// the console went black for good.
///
/// ⊘ What is still refused: a window that LATCHES (any UPDATE, even one that keeps the handle)
/// with a context DMA that does not resolve, and a handle its armed state never resolved. The first
/// copy after a latch resolves afresh, and the flip's completions and GET wait for that copy
/// (`ScanState::barrier`), so a guest that waits for its channel to idle before freeing the surface
/// — NVKMS does (`nvEvoClearSurfaceUsage` → `nvRMSyncEvoChannel`, `nvkms-flip.c:1229-1263`) — is
/// resolved before it unbinds, whatever the timing. Only the store is ever read through a kept
/// resolution (`kf_disp::scanout::plan` refuses system memory), bounded again by the compose kernel.
#[derive(Debug, Default)]
struct LatchedDmas {
    /// Per window: the armed state's `(client, handle, chn)` and what it resolved to.
    by_window: [Option<(DmaKey, CtxDma)>; MAX_WINDOWS],
}

/// A window's context-DMA hash key as its ARMED state names it: `(client, handle, chn)`.
type DmaKey = (u32, u32, u32);

impl LatchedDmas {
    /// Window `w`'s ARMED state changed (an UPDATE latched it) or its channel was allocated or
    /// freed: its next copy resolves the context DMA again.
    fn forget(&mut self, w: u32) {
        if let Some(e) = self.by_window.get_mut(w as usize) {
            *e = None;
        }
    }

    /// The context DMA window `so` scans: the one its armed state resolved to since it last
    /// latched, else `fresh()` — kept when it resolves. A refusal is never kept.
    fn resolve(
        &mut self,
        so: &kf_disp::engine::Scanout,
        fresh: impl FnOnce() -> Result<CtxDma, String>,
    ) -> Result<CtxDma, String> {
        let key = (so.client, so.handle, so.chn);
        let slot = self.by_window.get_mut(so.window as usize);
        if let Some(Some((k, dma))) = slot.as_deref()
            && *k == key
        {
            return Ok(*dma);
        }
        let dma = fresh()?;
        if let Some(s) = slot {
            *s = Some((key, dma));
        }
        Ok(dma)
    }
}

/// What [`ReasonLog::admit`] decided about a refusal reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admit {
    /// Logged before (or beyond the bound): count it, say nothing.
    Seen,
    /// The first time: log it.
    New,
    /// The bound was just reached: log that new reasons are now only counted.
    Full,
}

/// A refusal is a visible state, not a silent skip: each distinct reason is logged once, so a
/// frozen console always has its cause in the log however many frames were refused. Bounded
/// ([`ReasonLog::MAX`] reasons of at most [`ReasonLog::LEN`] bytes), because a refusal text can
/// carry a guest-chosen number: a hostile guest cannot grow the log or this set without bound.
#[derive(Debug, Default)]
struct ReasonLog {
    seen: Vec<String>,
    full: bool,
}

impl ReasonLog {
    const MAX: usize = 64;
    const LEN: usize = 160;

    fn admit(&mut self, why: &str) -> Admit {
        let key: String = why.chars().take(Self::LEN).collect();
        if self.seen.contains(&key) {
            Admit::Seen
        } else if self.seen.len() < Self::MAX {
            self.seen.push(key);
            Admit::New
        } else if !self.full {
            self.full = true;
            Admit::Full
        } else {
            Admit::Seen
        }
    }
}

/// A resolved colour binding belongs to one armed channel incarnation. Unlike the
/// framebuffer cache, its token also names the immutable GPU snapshot of the LUT.
struct ColorDmas {
    slots: [Option<(ColorKey, kf_cuda::display::ColorLut)>; 65],
    next: u64,
}

impl Default for ColorDmas {
    fn default() -> Self {
        Self {
            slots: [None; 65],
            next: 0,
        }
    }
}

type ColorKey = (u32, u32, u32, kf_disp::color::Lut);

impl ColorDmas {
    /// Drop per-life bindings while keeping GPU snapshot tokens unique across lives.
    fn reset_life(&mut self) {
        self.slots.fill(None);
    }

    fn forget(&mut self, slot: usize) {
        if let Some(s) = self.slots.get_mut(slot) {
            *s = None;
        }
    }

    fn forget_window(&mut self, window: usize) {
        self.forget(window);
        self.forget(32 + window);
    }

    fn resolve(
        &mut self,
        slot: usize,
        client: u32,
        chn: u32,
        life: u32,
        lut: Option<kf_disp::color::Lut>,
        fresh: impl FnOnce(u32) -> Result<CtxDma, String>,
    ) -> Result<Option<kf_cuda::display::ColorLut>, String> {
        let s = self
            .slots
            .get_mut(slot)
            .ok_or("colour slot outside fixed allocation")?;
        let Some(lut) = lut else {
            *s = None;
            return Ok(None);
        };
        let key = (client, chn, life, lut);
        if let Some((old, resolved)) = *s
            && old == key
        {
            return Ok(Some(resolved));
        }
        let dma = match lut.binding {
            kf_disp::color::Binding::Dma { handle, .. } => Some(fresh(handle)?),
            kf_disp::color::Binding::Vidmem(_) => None,
        };
        let src = lut
            .binding
            .span_bytes(dma.as_ref(), (u64::from(lut.entries) + 4) * 8)
            .map_err(str::to_owned)?;
        self.next = self
            .next
            .checked_add(1)
            .ok_or("colour binding token exhausted")?;
        let resolved = kf_cuda::display::ColorLut {
            entries: lut.entries,
            src,
            interpolate: lut.interpolate,
            mirror: lut.mirror,
            token: self.next,
        };
        *s = Some((key, resolved));
        Ok(Some(resolved))
    }
}

/// ★ review hitch-20261017 (3): resolve a latched window's LUT and TMO context-DMA bindings AT THE LATCH — the same calls the
/// copy makes (`ScanState::start`'s colour program), so the copy a tick later finds them cached. Without it the bindings were
/// resolved at copy time, which with the one flip rule can be a tick AFTER the flip completed: a guest that idles and unbinds the
/// LUT context DMA (NVKMS teardown) before the tick would make the copy fail (`Fault::Console`: a stale console frame and a
/// refusal line). The LUT *contents* are still read by the copy (GPU, device-to-device); a guest that rewrites the LUT in place
/// right after the flip can show the console one frame with the new table — console-only, never guest-visible.
/// A refusal here is not kept: the copy tries again and reports it.
fn resolve_window_colors(
    colors: &mut ColorDmas,
    engine: &kf_disp::engine::Engine,
    io: &mut Io<'_>,
    sdr: (&'static kf_disp::class::ClassTable, u32, u32),
    so: &kf_disp::engine::Scanout,
) -> Result<(), String> {
    let (t, win, _core) = sdr;
    let armed = |m| engine.armed(ChannelKind::Window, so.window, m).unwrap_or(0);
    let lut = kf_disp::color::input(t, win, armed, color_experiments()).map_err(str::to_owned)?;
    let life = engine
        .generation(so.chn)
        .ok_or("no colour window incarnation")?;
    let pipeline = kf_disp::color::pipeline(
        t,
        win,
        armed,
        engine
            .armed_inline(so.window)
            .ok_or("missing armed inline tables")?,
    )
    .map_err(str::to_owned)?;
    colors.resolve(
        32 + so.window as usize,
        so.client,
        so.chn,
        life,
        pipeline.tmo,
        |handle| io.resolve(so.client, handle, so.chn),
    )?;
    colors.resolve(so.window as usize, so.client, so.chn, life, lut, |handle| {
        io.resolve(so.client, handle, so.chn)
    })?;
    Ok(())
}

/// ★ The layers one scanout copy composes, and the windows it refused (by name).
#[derive(Debug, Default)]
struct Planned {
    layers: Vec<LayerPlan>,
    /// ★ Semi-planar YUV windows (Edge's video overlay), composed after the RGB layers.
    yuv: Vec<kf_disp::scanout::YuvPlan>,
    refused: Vec<String>,
}

/// ★ Whether a lit head scans a window — and, when none does, whether the monitor is black yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Dark {
    /// An armed head scans a window.
    #[default]
    No,
    /// No head is lit (every head disarmed, or the core channel freed): no signal — black.
    Unlit,
    /// A head is lit with no window, for less than [`WINDOWLESS_HOLD`]: a modeset in flight — the
    /// last frame stays.
    WindowlessBrief,
    /// A head has been lit with no window for [`WINDOWLESS_HOLD`] or longer — black.
    Windowless,
}

/// ★ How long a lit head may scan no window before the console shows black. `[measured 2026-10-03,
/// b5f at 4a4b95f7]`: X's modeset on NVKMS left head 3 lit with no window for 22 ms (+147905 → +147927 ms)
/// before its first flip, and the first head's arming took 2–4 ms to latch its window (b1f, b5f) — a
/// black frame there is a flash no real monitor shows. A head left windowless for longer is a black
/// screen on bare metal; ten times the longest measured transient is the margin.
const WINDOWLESS_HOLD: Duration = Duration::from_millis(250);

/// ★ What the console holds when a guest life ends ([`DisplayPlane::rm_unloaded`], a device reset):
/// nothing the guest left — no preserved scanout — and, if a guest scanout was ever shown, it is
/// LOST: with the boot display the boot layer is chosen first anyway (`boot_done` goes back to
/// `false`); without it (`gop=off`) the monitor shows black, the bare-metal answer for a monitor
/// whose scanout went away, never the old life's last frame.
fn held_after_life_reset(prev: &Held) -> Held {
    Held {
        preserved: None,
        scanned: prev.scanned,
        dark: Dark::Unlit,
    }
}

/// ★★ Choose what the console shows. The boot layer is chosen BEFORE the window vocabulary's gate
/// (`console_composition` is `None` without a `ScanVocab`, as on GB20x), so every family shows the
/// boot picture; it is shown only while no head has ever been armed (`boot_done` is sticky), and
/// never again after.
/// ⊘ CORRECTED 2026-10-03 (B5, `V3_DISPLAY.md` §4.11.13): *"an unload that leaves no head armed
/// shows nothing, as on bare metal"* was built as "no new frame", so QEMU kept showing the LAST
/// frame (box run b5: the fbcon text stayed after `rmmod nvidia_drm`). With nothing scanned a real
/// monitor is black, and a scanout freed with `PRESERVE_HW` stays shown ([`Shown::Preserved`]) until
/// a head is armed again.
/// ⊘ CORRECTED again the same day (the review of `v3-gop-unload`): that black was chosen whenever a
/// frame had been shown and nothing was scanned — so the boot layer → first-head handoff showed one
/// black frame (`[measured b1f]` *"+52936 ms the console shows BLACK"*, 4 ms before head 3's window),
/// GB20x's boot layer was followed by black, and `gop=off` went black between any two windows.
/// [`Shown::Blank`] is now chosen only when a head's scanout that WAS shown ([`Held::scanned`]) is
/// lost — its head unlit, or lit with no window past [`WINDOWLESS_HOLD`]. Anything else that has
/// nothing to show is `None`: no new frame, the last one stays.
/// `gop=off`: before a head first scans a window this is `None` (QEMU's placeholder), as always;
/// after, a lost scanout is black where it used to freeze the last frame.
fn choose_shown(
    console: Option<Composition>,
    boot: Option<&BootScan>,
    boot_done: bool,
    held: &Held,
) -> Option<Shown> {
    match (console, boot) {
        (Some(c), _) => Some(Shown::Armed(c)),
        (None, Some(b)) if !boot_done => Some(Shown::Boot(b.layer, b.size)),
        (None, _) => match (&held.preserved, held.scanned, held.dark) {
            (Some((l, size)), _, _) => Some(Shown::Preserved(l.clone(), *size)),
            (None, Some(size), Dark::Unlit | Dark::Windowless) => Some(Shown::Blank(size)),
            (None, _, _) => None,
        },
    }
}

/// ★★ [`choose_shown`] with the boot range's cleanliness (`crate::bootclean`, 2026-10-18): the boot
/// layer is chosen only while store `[0, G)` is CLEAN. While it is tainted — a guest RM life has
/// begun, or a reset or unload scrub has not completed — the boot layer is skipped and the choice
/// falls to what `choose_shown` gives without it: the last frame stays before any guest scanout was
/// ever shown (no black at the first handoff), black after one was lost. An armed composition always
/// wins, as before.
fn choose_shown_gated(
    console: Option<Composition>,
    boot: Option<&BootScan>,
    boot_done: bool,
    boot_clean: bool,
    held: &Held,
) -> Option<Shown> {
    choose_shown(console, boot.filter(|_| boot_clean), boot_done, held)
}

/// FNV-1a over `words` — [`shown_key`]'s hash.
fn fnv(words: impl IntoIterator<Item = u64>) -> u64 {
    words.into_iter().fold(0xcbf2_9ce4_8422_2325, |h, w| {
        w.to_le_bytes()
            .iter()
            .fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
    })
}

/// ★ What a *"console shows"* line is about, as a number compared every loop without formatting
/// anything. ⊘ CORRECTED 2026-10-03 (the review of `v3-gop-unload`): the line's text was the key,
/// formatted every loop, and it named each window's context DMA and offset — which a page flip
/// changes — so `[measured b1f]` 256 lines were spent in ~4 s of flips and later transitions went
/// unlogged. The key leaves out what a flip changes; the line still prints it.
fn shown_key(shown: Option<&Shown>, held: &Held, boot_done: bool) -> u64 {
    match shown {
        // what the heads say matters to the line only once a scanout was shown (`shown_digest`):
        // `[measured 2026-10-03, b0g at 445367a8]` keyed on it before, gop=off printed "NOTHING yet"
        // twice in a row
        None => fnv([
            0,
            u64::from(held.scanned.is_some()),
            u64::from(boot_done),
            if held.scanned.is_some() {
                held.dark as u64
            } else {
                u64::MAX
            },
        ]),
        Some(Shown::Blank((w, h))) => fnv([1, u64::from(*w), u64::from(*h)]),
        Some(Shown::Preserved(l, (w, h))) => fnv([2, u64::from(*w), u64::from(*h)]
            .into_iter()
            .chain(l.iter().flat_map(|p| [u64::from(p.window), p.src]))),
        Some(Shown::Boot(l, (w, h))) => fnv([3, l.src, u64::from(*w), u64::from(*h)]),
        Some(Shown::Armed(c)) => fnv([
            4,
            u64::from(c.head),
            u64::from(c.width),
            u64::from(c.height),
        ]
        .into_iter()
        .chain(c.layers.iter().flat_map(|l| {
            [
                u64::from(l.window),
                u64::from(l.width),
                u64::from(l.height),
                u64::from(l.pitch),
                u64::from(l.format),
            ]
        }))),
    }
}

/// ★ One line naming what the console shows (`V3_DISPLAY.md` §4.11.13) — printed when
/// [`shown_key`] changes.
fn shown_digest(shown: Option<&Shown>, held: &Held, boot_done: bool) -> String {
    match shown {
        None => match (held.scanned, held.dark) {
            (Some(_), Dark::WindowlessBrief) => format!(
                "no new frame: a lit head has no window (held up to {} ms) — the last frame stays",
                WINDOWLESS_HOLD.as_millis()
            ),
            (Some(_), _) => "no new frame — the last frame stays".to_string(),
            (None, _) if boot_done => "no new frame: no armed head has scanned a window yet \
                                       — the boot layer's last frame stays (no black at the handoff)"
                .to_string(),
            (None, _) => "NOTHING yet (no head has scanned a window)".to_string(),
        },
        Some(Shown::Blank((w, h))) => format!(
            "BLACK {w}x{h} ({})",
            if held.dark == Dark::Unlit {
                "the scanout shown is lost: no head is lit"
            } else {
                "the scanout shown is lost: a lit head has had no window past the hold"
            }
        ),
        Some(Shown::Preserved(l, (w, h))) => {
            let srcs: Vec<String> = l
                .iter()
                .map(|p| format!("window {} store {:#x}", p.window, p.src))
                .collect();
            format!(
                "the PRESERVED scanout {w}x{h} [{}] (freed with PRESERVE_HW)",
                srcs.join("; ")
            )
        }
        Some(Shown::Boot(l, (w, h))) => {
            format!("the BOOT layer (store {:#x}, {w}x{h})", l.src)
        }
        Some(Shown::Armed(c)) => {
            let layers: Vec<String> = c
                .layers
                .iter()
                .map(|l| {
                    format!(
                        "window {} iso {:#x}+{:#x} {}x{} pitch {} fmt {:#x}",
                        l.window, l.handle, l.offset, l.width, l.height, l.pitch, l.format
                    )
                })
                .collect();
            format!(
                "head {} {}x{}: [{}]",
                c.head,
                c.width,
                c.height,
                layers.join("; ")
            )
        }
    }
}

/// ★ The bound on *"console shows"* lines (one more says the rest are not logged).
const SHOWN_LINES: u32 = 256;

/// What the worker takes at its start.
struct WorkerInit {
    engine: Engine,
    gpu: Option<DisplayGpu>,
    layout: Layout,
    notifier_finished: u32,
    /// `NV_DISP_NOTIFIER__0_STATUS_BEGUN` as a status word: a window flip's notifier at its latch.
    notifier_begun: u32,
    /// ★ The GPU-copy rung's worker half (§8.11), when the device offers it.
    vram: Option<VramWorker>,
}

/// ★ What the build hands the display plane for the GPU-copy rung: the property's mode and, when
/// realize's probe passed, the rung's host state.
#[derive(Debug, Clone, Copy)]
pub struct BrokerVram {
    /// `display-broker-vram`.
    pub mode: kf_broker::gpucopy::VramMode,
    /// The probe's result (`None`: the rung is not offered on this host).
    pub setup: Option<&'static crate::gpucopy::VramSetup>,
}

/// ★ The GPU-copy rung's worker half (§8.11): slots arrive from the provisioning thread, are
/// adopted here (the worker owns the CUDA context) and installed into the ring while free.
struct VramWorker {
    req: std::sync::mpsc::Sender<kf_broker::gpucopy::Request>,
    got: std::sync::mpsc::Receiver<Result<crate::gpucopy::Provisioned, String>>,
    plan: kf_broker::gpucopy::Provisioning,
    /// Per ring slot: the imported, installed VRAM slot and its bytes.
    slots: [Option<(kf_cuda::display::SlotId, u64)>; SLOTS],
    /// Adopted slots waiting for their ring slot to be free.
    waiting: Vec<(usize, kf_cuda::display::SlotId, u64, kf_broker::VramFds)>,
    selftested: bool,
    refused: Option<String>,
    fence_err_logged: bool,
}

impl VramWorker {
    /// ★ Step 5: import into the worker's CUDA context, clear, self-test the pack on the first
    /// slot (never exported before it passed), close the RM export fd, and queue for the ring.
    fn adopt(
        &mut self,
        gpu: &mut DisplayGpu,
        p: crate::gpucopy::Provisioned,
        counters: &DispCounters,
    ) -> Result<(), String> {
        let (slot, bytes) = (p.slot, p.bytes);
        let sid = gpu.import_slot(p.export_fd(), bytes).map_err(|e| {
            format!(
                "display VRAM slot {slot}: the CUDA import: {e} — the GPU-copy rung is withdrawn"
            )
        })?;
        gpu.zero_slot(sid)
            .map_err(|e| format!("display VRAM slot {slot}: the clear: {e}"))?;
        if !self.selftested {
            selftest_pack(gpu, sid).map_err(|e| {
                format!("display VRAM: the pack kernel self-test FAILED: {e} — the GPU-copy rung is withdrawn")
            })?;
            self.selftested = true;
            eprintln!("kf3: display: pack kernel self-test PASSED (into display VRAM slot {slot})");
        }
        let (slot, v) = p.into_backing()?;
        counters.vram_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.waiting.push((slot, sid, bytes, v));
        Ok(())
    }

    /// Install every adopted slot whose ring slot is free now.
    fn install_waiting(&mut self, ring: &kf_broker::FrameRing) -> Result<(), String> {
        let mut keep = Vec::new();
        for (slot, sid, bytes, v) in self.waiting.drain(..) {
            match ring.install_vram(slot, v) {
                Ok(()) => self.slots[slot] = Some((sid, bytes)),
                Err((kf_broker::InstallRefusal::NotFree, v)) => keep.push((slot, sid, bytes, v)),
                Err((e, _)) => {
                    return Err(format!(
                        "display VRAM slot {slot}: the frame ring refused it: {e:?}"
                    ));
                }
            }
        }
        self.waiting = keep;
        Ok(())
    }

    /// ★ Each pass: adopt what the provisioning thread made, install what is free. A refusal
    /// withdraws the VRAM kind ONLY (the host rungs go on), loudly, once.
    fn poll(&mut self, gpu: &mut DisplayGpu, ring: &kf_broker::FrameRing, counters: &DispCounters) {
        if self.refused.is_some() {
            return;
        }
        while let Ok(made) = self.got.try_recv() {
            let r = made.and_then(|p| self.adopt(gpu, p, counters));
            if let Err(e) = r {
                return self.refuse(ring, e);
            }
        }
        if let Err(e) = self.install_waiting(ring) {
            self.refuse(ring, e);
        }
    }

    fn refuse(&mut self, ring: &kf_broker::FrameRing, e: String) {
        ring.withdraw(kf_broker::Kind::Vram);
        self.plan.refuse();
        eprintln!("kf3: display: {e}");
        self.refused = Some(e);
    }

    fn ask(&mut self, r: Option<kf_broker::gpucopy::Request>, ring: &kf_broker::FrameRing) {
        if let Some(r) = r
            && self.req.send(r).is_err()
        {
            self.refuse(ring, "display VRAM: the provisioning thread is gone".into());
        }
    }

    /// ★ The free slots that can take a pack of `extent` bytes NOW: provisioned that large, free,
    /// and their dma-buf's fences signalled (`DMA_BUF_IOCTL_EXPORT_SYNC_FILE` + `poll(0)` — never
    /// waits; a kernel without the ioctl is logged once and its slots taken as idle).
    fn eligible(&mut self, ring: &kf_broker::FrameRing, extent: u64) -> u32 {
        let mut m = 0;
        for j in 0..ring.slots().min(SLOTS) {
            let Some((_, bytes)) = self.slots[j] else {
                continue;
            };
            if bytes < extent || !ring.is_free(j) {
                continue;
            }
            let idle = ring
                .vram(j)
                .map_or(Ok(true), |v| kf_linux_raw::dma_buf_idle(v.fd()));
            match idle {
                Ok(true) => m |= 1 << j,
                Ok(false) => {}
                Err(e) => {
                    if !self.fence_err_logged {
                        self.fence_err_logged = true;
                        eprintln!(
                            "kf3: display: the dma-buf fence check is unavailable ({e}): VRAM slots \
                             are reused after RELEASE and the LRU order alone"
                        );
                    }
                    m |= 1 << j;
                }
            }
        }
        m
    }
}

/// ★ The pack kernel on SYNTHETIC frames into slot `sid` before it is ever exported, compared byte
/// for byte with `kf_disp::vramslot::pack_reference` (the same reference the host-compiled kernel
/// is held to in kf-disp's `tests/bl_pack_kernel.rs`).
fn selftest_pack(gpu: &mut DisplayGpu, sid: kf_cuda::display::SlotId) -> Result<(), String> {
    use kf_disp::vramslot::{GOB_GA106, pack_reference, slot_geom};
    for (w, h, bh) in crate::gpucopy::selftest_cases() {
        let g = slot_geom(w, h, bh).ok_or("the self-test geometry")?;
        let staging = crate::gpucopy::selftest_staging(w, h);
        let pack = kf_cuda::display::BlPack {
            width: w,
            height: h,
            gobs_per_row: g.gobs_per_row,
            h_log2: bh,
            gobs: g.gobs(),
            bits: GOB_GA106.bits(),
        };
        let got = gpu
            .selftest_bl_pack(sid, &staging, &pack)
            .map_err(|e| format!("did not run ({w}x{h}, h {bh}): {e}"))?;
        let want = pack_reference(&GOB_GA106, &staging, &g)?;
        if let Some(i) = (0..want.len()).find(|&i| got.get(i) != want.get(i)) {
            return Err(format!(
                "{w}x{h} at 2^{bh}-GOB blocks: slot byte {i} is {:?}, the reference says {:#04x}",
                got.get(i),
                want[i]
            ));
        }
    }
    Ok(())
}

/// ★ The plane (leaked for the process, like the device).
pub struct DisplayPlane {
    /// The model the served chain answers from — shared across `ReselectAtFn1` rebuilds.
    pub model: SharedDisplayModel,
    /// The lock-free ports.
    pub ports: Arc<Ports>,
    /// Resolved registers.
    pub map: RegMap,
    /// The caps page, published at seal.
    caps: kf_disp::caps::CapsPage,
    wake: Arc<Notifier>,
    init: Mutex<Option<WorkerInit>>,
    /// Per head: the cursor's posted PIO methods.
    cursor: [CursorPorts; MAX_HEADS],
    /// Counters.
    pub counters: DispCounters,
    /// ★ M2: the frames QEMU's console shows.
    pub console: ConsoleShare,
    /// ★ Display step 3 (`display-broker`): the broker relay's seat — the frame backing and the
    /// relay the C device's main loop drives (`crate::broker`). `None`: the console alone,
    /// exactly as before (`cuMemAllocHost`, three slots).
    pub broker: Option<crate::broker::BrokerSeat>,
    /// The window-class methods a scanout reads (`None`: the family's windows name surfaces by
    /// address — no console yet, M5).
    scan: Option<ScanVocab>,
    /// Bounded SDR colour implementation, enabled for verification independently of
    /// constructor probes. Those probes still refuse all display methods.
    sdr_color: Option<(&'static kf_disp::class::ClassTable, u32, u32)>,
    /// ★ Display step 3d: the cursor methods the composition's top layer reads (`None`: the
    /// family's table lacks one — its cursor is not composed).
    cursor_vocab: Option<kf_disp::engine::CursorVocab>,
    /// ★ Display step 3c: the newest resize request from the VMM's UI (`ui_info`), packed
    /// `width << 48 | height << 32 | refresh_mHz`; 0 = none. Set on the main loop, taken by the
    /// worker — one atomic, no lock.
    ui_request: AtomicU64,
    /// The window formats the console can show.
    formats: ScanFormats,
    /// ★ The boot display's picture (`gop=on`); `None` keeps today's console.
    boot: Option<BootScan>,
    /// ★ `display-max-fps` (§8.16): the property (0 unset) — the cap and the EDID follow it.
    max_fps: u32,
    /// ★ The worker's `fps[...]` status fragment, the last window's (published with `try_lock`,
    /// read with `try_lock`: a diagnostic never waits).
    fps: Mutex<String>,
    /// ★ Device reset (`docs/design/V3_DEVICE_RESET.md`): the epoch the display worker has reset its
    /// per-life state for.
    pub life_reset: crate::reset::Ack,
    /// ★ The guest's RM gave the display up (fn 47 without a PM transition: GSP-RM's own unload) —
    /// how many times; the worker ends the display's life at each ([`DisplayPlane::rm_unloaded`]).
    unloads: AtomicU64,
    /// ★ 2026-10-18: whether store `[0, G)` may be shown as the boot picture
    /// ([`crate::bootclean`]). Tainted by a guest RM life, the seed's retirement and a device
    /// reset; cleaned only by a scrub that completed.
    pub boot_clean: crate::bootclean::BootClean,
}

impl DisplayPlane {
    /// ★ Something other than the firmware console may write store `[0, G)` now (a guest RM life
    /// began, the seed retired): the boot layer is not shown until a scrub completes. Never waits.
    pub fn boot_tainted(&self) {
        self.boot_clean.taint();
        let _ = self.wake.signal();
    }

    /// ★ A scrub of store `[0, G)` COMPLETED: the boot layer may be shown again. Never waits; the
    /// worker is woken to show it.
    pub fn boot_scrubbed(&self) {
        self.boot_clean.clean();
        let _ = self.wake.signal();
    }

    /// ★★ **The guest's RM unloaded** (`UNLOADING_GUEST_DRIVER` without `bInPMTransition`, observed
    /// on the drainer): the display's guest life is over. GSP-RM's own unload destroys every display
    /// object it held, so the worker drops the engine's channels and heads, the model's registry and
    /// what the console held, and — with the boot display (`gop=on`) — shows the BOOT LAYER again:
    /// store `[0, G)`, which BAR1 `[0, G)` reaches again from the same moment (RM's
    /// `kbusStatePreUnload` → `kbusTeardownMailbox` put BAR1 back to physical mode,
    /// `ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:746-787`; the
    /// re-seed is `crate::bar1phys`), so what the firmware console, efifb/simpledrm or Windows'
    /// basic display adapter draws there is what the monitor shows. Never waits: one counter, one
    /// wake.
    pub fn rm_unloaded(&self) {
        self.unloads.fetch_add(1, Ordering::AcqRel);
        let _ = self.wake.signal();
    }

    /// ★ Device reset: ask the worker to end the display's life for `epoch` (acknowledged on
    /// [`DisplayPlane::life_reset`]).
    pub fn request_life_reset(&self, epoch: u64) {
        // the old life's bytes are in store [0, G) until the memory stage's scrub completes
        self.boot_clean.taint();
        self.life_reset.request(epoch);
        let _ = self.wake.signal();
    }
}

impl crate::bootclean::BootSink for DisplayPlane {
    fn begin_life(&self) -> u64 {
        let g = self.boot_clean.begin_life();
        let _ = self.wake.signal();
        g
    }
    fn taint(&self) {
        self.boot_tainted();
    }
    fn generation(&self) -> u64 {
        self.boot_clean.generation()
    }
    fn scrubbed_if(&self, life_id: u64) -> bool {
        let cleaned = self.boot_clean.clean_if(life_id);
        if cleaned {
            let _ = self.wake.signal();
        }
        cleaned
    }
    fn scrubbed(&self) {
        self.boot_scrubbed();
    }
}

impl std::fmt::Debug for DisplayPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayPlane")
            .field("map", &self.map)
            .finish_non_exhaustive()
    }
}

/// ★ The GPU-copy rung's worker half, at build: the ring learns the modifier and the GPU's nodes,
/// the provisioning thread starts, and with `display-broker-vram=on` the class-0 slots are made,
/// adopted (pack self-test included) and installed NOW — a refusal fails realize (§w727).
fn vram_worker(
    mode: kf_broker::gpucopy::VramMode,
    setup: &'static crate::gpucopy::VramSetup,
    ring: &Arc<kf_broker::FrameRing>,
    gpu: &mut DisplayGpu,
) -> Result<(VramWorker, u64), String> {
    use kf_broker::gpucopy::{Provisioning, VramMode};
    ring.set_vram_modifier(setup.modifier());
    ring.set_gpu_nodes(setup.nodes());
    let (req, got) = crate::gpucopy::spawn(setup, ring.clone())?;
    let mut w = VramWorker {
        req,
        got,
        plan: Provisioning::new(
            mode,
            kf_broker::slots::BROKER_SLOTS,
            kf_disp::vramslot::SLOT_CLASS0,
            kf_disp::vramslot::SLOT_MAX,
        ),
        slots: [None; SLOTS],
        waiting: Vec::new(),
        selftested: false,
        refused: None,
        fence_err_logged: false,
    };
    let counters = DispCounters::default();
    if mode == VramMode::On
        && let Some(r) = w.plan.first(true, false)
    {
        for slot in r.slots {
            let p = setup.make(ring, slot, r.bytes)?;
            w.adopt(gpu, p, &counters)?;
        }
        w.install_waiting(ring)?;
        eprintln!(
            "kf3: broker: display-broker-vram=on — {} MiB of display VRAM in {} slots, pack self-test passed",
            counters.vram_bytes.load(Ordering::Relaxed) >> 20,
            kf_broker::slots::BROKER_SLOTS
        );
    }
    let bytes = counters.vram_bytes.load(Ordering::Relaxed);
    Ok((w, bytes))
}

impl DisplayPlane {
    /// ★ Build the plane for a chip's display row and the guest driver `table`: the derived
    /// vocabulary, the caps page, the shared model, and the plane's GPU context with the store
    /// imported (`store_fd` exported from the host RM for this context).
    ///
    /// # Errors
    /// By name — `display=on` never starts on a guessed display.
    pub fn build(
        row: &'static kf_chip::display::DisplayRow,
        table: &kf_abi::versions::DriverAbiTable,
        bdf: &str,
        store_fd: i32,
        store_bytes: u64,
        broker: Option<BrokerVram>,
        max_fps: u32,
    ) -> Result<DisplayPlane, String> {
        kf_disp::pace::check(true, max_fps)?;
        let version = table.driver_version().to_string();
        let regs = Regs::for_ip(&version, row.ip_version).ok_or_else(|| {
            format!(
                "display=on: no derived display registers for driver {version} / IP {:#010x}",
                row.ip_version
            )
        })?;
        let t = kf_disp::class::for_version(&version).ok_or_else(|| {
            format!("display=on: no derived display classes for driver {version}")
        })?;
        let classes = kf_disp::model::Classes::of(row);
        let vocab = Vocab::resolve(t, &classes, &regs)
            .map_err(|e| format!("display=on: method vocabulary: {} is not derived", e.0))?;
        let olut_constructor_probe =
            std::env::var("KF3_DISPLAY_OLUT_CONSTRUCTOR_PROBE").is_ok_and(|v| v == "1");
        let tmo_surface_constructor_probe = olut_constructor_probe
            || std::env::var("KF3_DISPLAY_TMO_SURFACE_CONSTRUCTOR_PROBE").is_ok_and(|v| v == "1");
        let ilut_constructor_probe = tmo_surface_constructor_probe
            || std::env::var("KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE").is_ok_and(|v| v == "1");
        // ILUT always includes TMO construction and the blanket method refusal;
        // setting ILUT alone must never publish these caps with a normal engine.
        let constructor_probe = ilut_constructor_probe
            || std::env::var("KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE").is_ok_and(|v| v == "1");
        let caps_author = if olut_constructor_probe {
            kf_disp::caps::olut_constructor_probe_page
        } else if tmo_surface_constructor_probe {
            kf_disp::caps::tmo_surface_constructor_probe_page
        } else if ilut_constructor_probe {
            kf_disp::caps::ilut_constructor_probe_page
        } else if constructor_probe {
            kf_disp::caps::constructor_probe_page
        } else {
            kf_disp::caps::page
        };
        let sdr_color = std::env::var("KF3_DISPLAY_SDR_COLOR").as_deref() == Ok("1");
        if sdr_color {
            // read (and confirm in the log) the decoder experiments at birth, not at first use
            let _ = color_experiments();
        }
        if sdr_color && constructor_probe {
            return Err(
                "display=on: SDR processing cannot be combined with constructor-only probes".into(),
            );
        }
        let caps = if sdr_color {
            kf_disp::caps::sdr_page(
                t,
                &regs,
                row.classes.caps,
                classes.core,
                row.heads,
                row.windows,
            )
        } else {
            caps_author(t, &regs, row.classes.caps, row.heads, row.windows)
        }
        .map_err(|e| format!("display=on: caps page: {}", e.0))?;
        // ⚠⚠ PROBE (default off, H-caps): the real GPU's caps page instead of the authored one
        let caps = if caps_probe_on() {
            match caps_probe_page(caps.base, row.classes.caps) {
                Some(p) => {
                    eprintln!(
                        "kf3: display: PROBE KF3_DISPLAY_CAPS_PROBE=1 — the caps page is the real GPU's measured page ({} words; H-caps probe, not a shipped behaviour)",
                        p.words.len()
                    );
                    p
                }
                None => {
                    eprintln!(
                        "kf3: display: PROBE KF3_DISPLAY_CAPS_PROBE=1 REFUSED — caps class {:#x} / page {:#x} is not the measured one; the authored page stays",
                        row.classes.caps, caps.base
                    );
                    caps
                }
            }
        } else {
            caps
        };
        let layout = Layout::from_regs(&regs)
            .ok_or("display=on: the instance-memory layout is not derived")?;
        let map = RegMap::resolve(&regs, t, row)?;
        let notifier_finished = t
            .notifier_field("__0_STATUS")
            .zip(t.notifier_value("__0_STATUS_FINISHED"))
            .map(|(fld, v)| kf_disp::class::put(0, fld, v))
            .ok_or("display=on: NV_DISP_NOTIFIER__0_STATUS is not derived")?;
        // ★ 2026-10-10: a WINDOW flip's notifier is BEGUN at its latch and FINISHED at its flip-away (open NVKMS: the
        // display writes it BEGUN when it performs the flip, `ogkm-595.84: nvidia-modeset/src/nvkms-headsurface.c:
        // 1925-1952`; `kf_disp::engine::Effect::Notify`); the core's completion notifier is FINISHED
        // (`nvkms-evo3.c:6224-6243`)
        let notifier_begun = t
            .notifier_field("__0_STATUS")
            .zip(t.notifier_value("__0_STATUS_BEGUN"))
            .map(|(fld, v)| kf_disp::class::put(0, fld, v))
            .ok_or("display=on: NV_DISP_NOTIFIER__0_STATUS_BEGUN is not derived")?;
        let model = kf_rm::display::model_for(table, row, max_fps).ok_or_else(|| {
            format!("display=on: no derived display layouts for driver {version}")
        })?;
        // ★ §8.16: what the guest is told the monitor is — graded from the log alone
        if let Some(m) = model.connectors.first().map(|c| &c.monitor) {
            eprintln!(
                "kf3: display: monitor {}x{} at {} mHz, range max {} Hz, EDID fnv1a64={:016x} \
                 (display-max-fps {})",
                m.preferred.h_active,
                m.preferred.v_active,
                m.preferred.refresh_mhz(),
                m.cap_hz,
                m.edid_fnv().unwrap_or(0),
                max_fps
            );
        }
        let ports = model.ports.clone();
        // ★ 2026-10-09 EXPERIMENT (`KF3_DISPLAY_HEAD_TIMING_EN_BASE=<hex>`, default off, DIAGNOSTIC): the
        // value `NV_PDISP_FE_RM_INTR_EN_HEAD_TIMING(h)` (0x611D80) reads back before the guest writes it.
        // `[measured, VFIO DVI reference boot3, RTX 4070, 2026-10-08/09, traces/windows_reset_20261009
        // README §13.1 point 3]` real hardware reads `0x3f0060` (firmware's enabled RG_LINE_A/B and
        // semaphore events; ogkm names only LAST_DATA 1:1 and RG_LINE_A/B 5/6 in the STATUS register)
        // and the guest read-modify-writes it (`0x3f0062`/`0x3f0060`); kf3 reads back 0 and the guest
        // writes `0x2`/`0x0`. Per-die firmware state: NOT derived, so never a default without an owner
        // ruling (derive, never capture).
        if let Some(base) = std::env::var("KF3_DISPLAY_HEAD_TIMING_EN_BASE")
            .ok()
            .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        {
            for h in 0..MAX_HEADS {
                ports.head_timing_en[h].store(base, Ordering::Release);
            }
            eprintln!(
                "kf3: display: EXPERIMENT KF3_DISPLAY_HEAD_TIMING_EN_BASE={base:#x} — 0x611d80 reads back this base (hardware: 0x3f0060)"
            );
        }
        let wake = Arc::new(Notifier::create().map_err(|e| format!("display eventfd: {e:?}"))?);
        let model: SharedDisplayModel = Arc::new(Mutex::new(model));
        {
            let w = wake.clone();
            model
                .lock()
                .map_err(|_| "display model poisoned")?
                .attach_plane(Waker(Arc::new(move || {
                    let _ = w.signal();
                })));
        }
        let mut gpu = DisplayGpu::bring_up_on(bdf)
            .map_err(|e| format!("display=on: the display plane's GPU context on {bdf}: {e}"))?;
        gpu.import_store(store_fd, store_bytes)
            .map_err(|e| format!("display=on: store import into the display context: {e}"))?;
        let mut engine = if constructor_probe {
            eprintln!(
                "kf3: EXPERIMENT display constructor probe: TMO advertised, ILUT surface loading={ilut_constructor_probe}, TMO surface loading={tmo_surface_constructor_probe}, OLUT surface loading={olut_constructor_probe}; ALL display methods refused before execution"
            );
            Engine::new_constructor_probe(vocab, row.heads, row.windows)
        } else {
            Engine::new(vocab, row.heads, row.windows)
        };
        let scan = ScanVocab::resolve(t, classes.window, classes.window_imm, classes.core);
        let formats = ScanFormats::resolve(t, classes.window);
        let cursor_vocab = kf_disp::engine::CursorVocab::resolve(t, classes.core, classes.cursor);
        let mut vram = None;
        let mut vram_bytes = 0;
        let (console, broker) = if let Some(bv) = broker {
            let ring = Arc::new(kf_broker::FrameRing::new(
                kf_broker::slots::BROKER_SLOTS,
                true,
            ));
            let seat = crate::broker::BrokerSeat::new(ring.clone())?;
            if let Some(setup) = bv.setup {
                let (w, b) = vram_worker(bv.mode, setup, &ring, &mut gpu)?;
                vram = Some(w);
                vram_bytes = b;
            }
            (ConsoleShare::over(ring), Some(seat))
        } else {
            (ConsoleShare::default(), None)
        };
        Ok(DisplayPlane {
            model,
            ports,
            map,
            caps,
            wake,
            init: Mutex::new(Some(WorkerInit {
                engine,
                gpu: Some(gpu),
                layout,
                notifier_finished,
                notifier_begun,
                vram,
            })),
            cursor: core::array::from_fn(|_| CursorPorts::default()),
            counters: DispCounters {
                vram_bytes: AtomicU64::new(vram_bytes),
                // Read once while constructing the plane, never on a worker pass or a vCPU.
                ..DispCounters::with_step_meters(
                    std::env::var("KF3_DIAG_STEP_METERS").is_ok_and(|v| v == "1"),
                )
            },
            console,
            broker,
            scan,
            sdr_color: sdr_color.then_some((t, classes.window, classes.core)),
            cursor_vocab,
            ui_request: AtomicU64::new(0),
            life_reset: crate::reset::Ack::default(),
            unloads: AtomicU64::new(0),
            boot_clean: crate::bootclean::BootClean::new(),
            formats,
            boot: None,
            max_fps,
            fps: Mutex::new(String::new()),
        })
    }

    /// ★ The `display-max-fps` property (0 unset).
    #[must_use]
    pub fn max_fps(&self) -> u32 {
        self.max_fps
    }

    /// ★ The `fps[...]` status fragment (empty before the worker's first window; `fps[busy]` while
    /// the worker publishes one — never a wait).
    #[must_use]
    pub fn fps_fragment(&self) -> String {
        self.fps
            .try_lock()
            .map_or_else(|_| "fps[busy]".to_string(), |g| g.clone())
    }

    /// ★ D2.3 (main loop, the console's `gfx_update`, a `screendump`): ask the worker for a frame no
    /// older than now — it is served at the console head's next tick (a check: sent if it changed),
    /// or at once when nothing can be copied. `false` when no answer will come.
    #[must_use]
    pub fn request_refresh(&self) -> bool {
        let asked = self.console.request_refresh();
        if asked {
            let _ = self.wake.signal();
        }
        asked
    }

    /// ★ Show `boot` (the boot display's layer, `gop=on`) until the guest arms a head.
    #[must_use]
    pub fn with_boot(mut self, boot: Option<BootScan>) -> DisplayPlane {
        self.boot = boot;
        self
    }

    /// The boot display's picture, if any.
    #[must_use]
    pub fn boot(&self) -> Option<&BootScan> {
        self.boot.as_ref()
    }

    /// The caps page's `(BAR0 offset, value)` words, and each cursor's `Free` (what the shadow holds
    /// from the start: static register values of the engine we present).
    pub fn static_words(&self) -> Vec<(u64, u32)> {
        let mut v: Vec<(u64, u32)> = self
            .caps
            .words
            .iter()
            .map(|(o, w)| (self.caps.base + u64::from(*o), *w))
            .collect();
        for h in 0..self.map.heads {
            // `Free` counts the PIO slots the FE has for methods: never full (NVKMS waits for non-zero)
            v.push((
                self.map.user_base(ChannelKind::Cursor, h) + self.map.cursor_free,
                4,
            ));
        }
        v
    }

    /// ★★ **THE vCPU PATH** for a write in the display aperture. Lock-free: atomics, shadow stores,
    /// at most one eventfd write. `store` stores a 32-bit shadow word; `plain` stores the guest's
    /// write at its own width. Returns `true` when the write ENABLED an event that is already pending
    /// — the caller then raises the display interrupt, as a level-triggered source would.
    #[must_use]
    pub fn trap_write(
        &self,
        off: u64,
        val: u64,
        width: u8,
        store: &dyn Fn(u64, u32),
        plain: &dyn Fn(u64, u64, u8),
    ) -> bool {
        self.counters.writes.fetch_add(1, Ordering::Relaxed);
        #[allow(clippy::cast_possible_truncation)]
        let v = val as u32;
        let class = self.map.classify(off);
        if write_trace_on() {
            // ⚠ DIAGNOSTIC (default off, bounded): one line from the vCPU — never on in production
            static N: AtomicU32 = AtomicU32::new(0);
            if trace_slot(&N, display_trace_cap()) {
                eprintln!(
                    "kf3: display: WTRACE t={:.6} WRITE {off:#08x} <- {val:#x} w{width} {class:?}",
                    kf_mem::maplog::t()
                );
            }
        }
        match class {
            DispWrite::Put(chn) => {
                store(off, v);
                if self.ports.post_put(chn, v) {
                    let _ = self.wake.signal();
                }
            }
            DispWrite::ReadOnly => {}
            DispWrite::Cursor(h, rel) => {
                plain(off, val, width);
                if let Some(c) = self.cursor.get(h as usize) {
                    let update = rel == self.map.cursor_update;
                    c.post(rel, v, update);
                    if update {
                        let _ = self.wake.signal();
                    }
                }
            }
            DispWrite::Event(r) => {
                self.ports.guest_write(r, v);
                self.publish_events(store);
                if let EventReg::HeadTimingEn(h) = r {
                    // ⚠ DIAGNOSTIC (2026-10-08, after run78: Windows raised 2 display interrupts in
                    // 161 vblanks, Linux 2123 in 2642): the guest's head-timing interrupt enables,
                    // the first 32, under the existing method-trace switch. A line from the vCPU —
                    // never on in production.
                    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    if *ON.get_or_init(|| {
                        std::env::var("KF3_DISPLAY_METHOD_TRACE").is_ok_and(|x| x == "1")
                    }) && N.fetch_add(1, Ordering::Relaxed) < 32
                    {
                        eprintln!(
                            "kf3: display: RM_INTR_EN_HEAD_TIMING({h}) <- {v:#x} (LAST_DATA bit {:#x} {}, VBLANK bit {:#x} {})",
                            self.map.head_last_data,
                            if v & self.map.head_last_data != 0 {
                                "on"
                            } else {
                                "off"
                            },
                            self.map.head_vblank,
                            if v & self.map.head_vblank != 0 {
                                "on"
                            } else {
                                "off"
                            }
                        );
                    }
                    return self.ports.rm_head_timing(h) != 0;
                }
            }
            DispWrite::Plain => plain(off, val, width),
        }
        false
    }

    /// ★ Publish every event register and the ISR's derived summary registers — from any thread.
    /// ⊘ Repeats until the atomics are stable across a full publication, so a vCPU clearing a bit
    /// and the worker raising one can never leave a stale word behind (the last publisher sees the
    /// final state).
    pub fn publish_events(&self, store: &dyn Fn(u64, u32)) {
        let m = &self.map;
        let p = &self.ports;
        let heads = m.heads as usize;
        let snap = || -> Vec<u32> {
            let mut s = vec![
                p.event(EventReg::AwakenWin),
                p.event(EventReg::AwakenOther),
                p.event(EventReg::SemWin),
            ];
            for h in 0..heads {
                s.push(p.event(EventReg::HeadTiming(h)));
                s.push(p.event(EventReg::HeadTimingEn(h)));
            }
            s
        };
        for _ in 0..8 {
            let before = snap();
            store(m.evt_awaken_win, before[0]);
            store(m.evt_awaken_other, before[1]);
            if let Some(o) = m.evt_sem_win {
                store(o, before[2]);
            }
            for h in 0..heads {
                store(
                    m.evt_head_timing.0 + h as u64 * m.evt_head_timing.1,
                    before[3 + 2 * h],
                );
                if let Some((b, s)) = m.rm_intr_en_head_timing {
                    store(b + h as u64 * s, before[4 + 2 * h]);
                }
                store(
                    m.rm_intr_stat_head_timing.0 + h as u64 * m.rm_intr_stat_head_timing.1,
                    p.rm_head_timing(h),
                );
            }
            store(m.rm_intr_dispatch, p.rm_dispatch(heads));
            store(
                m.rm_ctrl_disp,
                p.rm_ctrl_disp(m.rm_ctrl_awaken_bit, m.rm_ctrl_win_sem_bit),
            );
            if let Some((o, b)) = m.evt_dispatch {
                store(o, if before[2] != 0 { b } else { 0 });
            }
            if snap() == before {
                return;
            }
        }
    }

    /// ★ Display step 3c (QEMU's main loop, the console's `ui_info` — a VNC/GTK resize, or the
    /// broker's SURFACE): ask for a `width` x `height` monitor at `refresh_mhz` (0 = 60 Hz) on
    /// `head`. Only head 0 has a console. Lock-free: one atomic store and one eventfd write; the
    /// worker authors the EDID and queues the hotplug.
    pub fn request_ui(&self, head: u32, width: u32, height: u32, refresh_mhz: u32) -> bool {
        if head != 0 || width == 0 || height == 0 {
            return false;
        }
        let w = u64::from(width.min(0xFFFF));
        let h = u64::from(height.min(0xFFFF));
        self.ui_request.store(
            (w << 48) | (h << 32) | u64::from(refresh_mhz),
            Ordering::Release,
        );
        let _ = self.wake.signal();
        true
    }

    fn take_init(&self) -> Option<WorkerInit> {
        self.init.lock().ok()?.take()
    }
}

/// The worker's I/O side (everything the acquire callback and the effects need, apart from the
/// engine it runs beside).
struct Io<'d> {
    dev: &'d Device,
    dp: &'d DisplayPlane,
    gpu: Option<DisplayGpu>,
    layout: Layout,
    inst: Option<kf_disp::model::InstMem>,
    /// The instance-memory image, read (by the GPU) at most once per worker pass.
    img: Option<Vec<u8>>,
    notifier_finished: u32,
    notifier_begun: u32,
    refusals_logged: u32,
}

impl Io<'_> {
    fn image(&mut self) -> Result<&[u8], String> {
        if self.img.is_none() {
            let im = self
                .inst
                .ok_or("no display instance memory was stated (WRITE_INST_MEM)")?;
            let n = im.size.min(self.layout.inst_bytes());
            let mut buf = vec![0u8; usize::try_from(n).map_err(|_| "instance memory size")?];
            match im.addr_space {
                2 => self
                    .gpu
                    .as_ref()
                    .ok_or("no display GPU context")?
                    .read_store(im.phys, &mut buf)
                    .map_err(|e| format!("instance memory read: {e}"))?,
                1 => {
                    let b = self
                        .dev
                        .ram
                        .block_for(im.phys, n)
                        .ok_or("sysmem instance memory is not in guest RAM")?;
                    if !b.mem.read_into((im.phys - b.gpa) as usize, &mut buf) {
                        return Err("sysmem instance memory read refused".into());
                    }
                }
                s => return Err(format!("instance memory address space {s}")),
            }
            self.img = Some(buf);
        }
        Ok(self.img.as_deref().unwrap_or(&[]))
    }

    fn resolve(&mut self, client: u32, handle: u32, chn: u32) -> Result<CtxDma, String> {
        let layout = self.layout;
        let img = self.image()?;
        layout
            .resolve(img, client, handle, chn)
            .map_err(|m| format!("context DMA {handle:#x} on channel {chn}: {m:?}"))
    }

    fn read(&self, dma: CtxDma, off: u64, buf: &mut [u8]) -> Result<(), String> {
        let at = dma
            .span(off, buf.len() as u64)
            .ok_or_else(|| format!("[{off:#x}, +{:#x}) is past the context DMA", buf.len()))?;
        match dma.target {
            Target::Sysmem => {
                let b = self
                    .dev
                    .ram
                    .block_for(at, buf.len() as u64)
                    .ok_or_else(|| format!("{at:#x} is not guest RAM"))?;
                b.mem
                    .read_into((at - b.gpa) as usize, buf)
                    .then_some(())
                    .ok_or_else(|| "guest RAM read refused".to_string())
            }
            Target::Vidmem => self
                .gpu
                .as_ref()
                .ok_or("no display GPU context")?
                .read_store(at, buf)
                .map_err(|e| e.to_string()),
        }
    }

    fn write(&self, dma: CtxDma, off: u64, bytes: &[u8]) -> Result<(), String> {
        if !dma.writable {
            return Err("the context DMA is read-only".into());
        }
        let at = dma
            .span(off, bytes.len() as u64)
            .ok_or_else(|| format!("[{off:#x}, +{:#x}) is past the context DMA", bytes.len()))?;
        match dma.target {
            Target::Sysmem => {
                let b = self
                    .dev
                    .ram
                    .block_for(at, bytes.len() as u64)
                    .ok_or_else(|| format!("{at:#x} is not guest RAM"))?;
                b.mem
                    .write_from((at - b.gpa) as usize, bytes)
                    .then_some(())
                    .ok_or_else(|| "guest RAM write refused".to_string())
            }
            Target::Vidmem => self
                .gpu
                .as_ref()
                .ok_or("no display GPU context")?
                .write_store(at, bytes)
                .map_err(|e| e.to_string()),
        }
    }

    /// Does acquire `a` hold against guest memory now?
    fn acquired(&mut self, a: &Acquire) -> bool {
        let dma = match self.resolve(a.client, a.handle, a.chn) {
            Ok(d) => d,
            Err(e) => {
                self.refuse(&format!("acquire: {e}"));
                return false;
            }
        };
        let mut b = [0u8; 8];
        let n = if a.wide { 8 } else { 4 };
        if let Err(e) = self.read(dma, a.offset, &mut b[..n]) {
            self.refuse(&format!("acquire read: {e}"));
            return false;
        }
        a.satisfied_by(u64::from_le_bytes(b))
    }

    /// The semaphore value an acquire reads now (diagnostic: `Err` text when it cannot be read).
    fn peek_acquire(&mut self, a: &Acquire) -> Result<u64, String> {
        let dma = self.resolve(a.client, a.handle, a.chn)?;
        let mut b = [0u8; 8];
        let n = if a.wide { 8 } else { 4 };
        self.read(dma, a.offset, &mut b[..n])?;
        Ok(u64::from_le_bytes(b))
    }

    fn refuse(&mut self, why: &str) {
        self.dp.counters.refused.fetch_add(1, Ordering::Relaxed);
        if self.refusals_logged < 64 {
            self.refusals_logged += 1;
            eprintln!("kf3: display: REFUSED {why}");
        }
    }

    fn pushbuffer(&self, pb: PbLoc) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; pb.bytes as usize];
        if pb.sysmem {
            let b = self
                .dev
                .ram
                .block_for(pb.addr, u64::from(pb.bytes))
                .ok_or_else(|| format!("pushbuffer {:#x} is not guest RAM", pb.addr))?;
            if !b.mem.read_into((pb.addr - b.gpa) as usize, &mut buf) {
                return Err("pushbuffer read refused".into());
            }
        } else {
            self.gpu
                .as_ref()
                .ok_or("no display GPU context")?
                .read_store(pb.addr, &mut buf)
                .map_err(|e| format!("pushbuffer read: {e}"))?;
        }
        Ok(buf)
    }
}

impl Device {
    /// ★ **The live scan-line publisher** — its own thread (`kf3-scanline`), never a vCPU, never the
    /// drainer, no lock. BAR0 reads do not trap, so a raster position the guest polls (`GET_RG_SCAN_LINE`,
    /// window `GET_LINE`; `D3DKMTGetScanLine`, the vblank path of the Windows driver) is a shadow word
    /// this thread rewrites every [`SCAN_LINE_STEP`] while a head's raster runs (every frame edge also
    /// writes it, before its interrupt). `[measured run 2990, 2026-10-14]` with the words constant 0 an
    /// OpenGL application with vsync on (Minecraft, swap interval 1) hung 300 s and the ICD aborted
    /// ("Unable to communicate with the display driver", error 2).
    pub fn scanline_loop(&self) {
        let Some(dp) = self.display else { return };
        if dp.map.rg_scan_line.is_none() && dp.map.win_get_line.is_none() {
            eprintln!(
                "kf3: display: no GET_RG_SCAN_LINE / GET_LINE in the class table — no live scan line"
            );
            return;
        }
        while !self.stop.load(Ordering::Acquire) {
            let words = dp.map.scan_words(&dp.ports, dp.ports.now_ns(), None);
            for (o, v) in &words {
                self.shadow_store(*o, u64::from(*v), 4);
            }
            std::thread::sleep(if words.is_empty() {
                SCAN_LINE_IDLE
            } else {
                SCAN_LINE_STEP
            });
        }
    }

    /// ★★★ **The display worker** — its own thread; see the module docs. ⊘ Never a vCPU, never the
    /// drainer, and it holds the model's lock only to take statements.
    pub fn display_loop(&self) {
        let Some(dp) = self.display else { return };
        let Some(init) = dp.take_init() else { return };
        let WorkerInit {
            mut engine,
            gpu,
            layout,
            notifier_finished,
            notifier_begun,
            vram,
        } = init;
        if let Some(g) = &gpu
            && let Err(e) = g.make_current()
        {
            eprintln!(
                "kf3: display: the plane's GPU context cannot be made current: {e} — the display plane is DOWN"
            );
            return;
        }
        // ★ M3: the block-linear scanout kernel proves itself on synthetic data before any guest copy
        let mut gpu = gpu;
        let bl_ok = gpu.as_mut().map(|g| {
            let r = selftest_compose(g);
            match &r {
                Ok(()) => eprintln!("kf3: display: compose kernel self-test PASSED"),
                Err(e) => eprintln!(
                    "kf3: display: compose kernel self-test FAILED: {e} — the console will show nothing"
                ),
            }
            r
        });
        let Ok(poller) = Poller::create() else {
            eprintln!("kf3: display: epoll refused — the display plane is DOWN");
            return;
        };
        if poller.watch(dp.wake.as_source_fd(), 1).is_err() {
            eprintln!("kf3: display: epoll watch refused — the display plane is DOWN");
            return;
        }
        // ★ M2: the scanout copies' completion (`cuLaunchHostFunc` after each copy) wakes the worker
        if let Some(g) = &gpu
            && poller
                .watch(std::os::fd::AsFd::as_fd(g.completion_fd()), 2)
                .is_err()
        {
            eprintln!(
                "kf3: display: epoll watch of the scanout completion refused — the display plane is DOWN"
            );
            return;
        }
        let trace = std::env::var("KF3_DISPLAY_TRACE").is_ok_and(|v| v == "1");
        engine.trace = trace;
        // ⚠ EXPERIMENT (default off, 2026-10-08, H-corelatch): core updates latch at an active
        // head's vblank, as the hardware's do (`kf_disp::engine::Engine::core_latch_at_vblank`)
        engine.core_latch_at_vblank =
            std::env::var("KF3_DISPLAY_CORE_AT_VBLANK").is_ok_and(|v| v == "1");
        if engine.core_latch_at_vblank {
            eprintln!(
                "kf3: display: EXPERIMENT KF3_DISPLAY_CORE_AT_VBLANK=1 — a core update on an active head latches (and notifies) at its next vblank"
            );
        }
        let loadv = loadv_on();
        let (blank_state, armed_defaults) = (blank_state_on(), armed_defaults_on());
        if blank_state {
            eprintln!(
                "kf3: display: EXPERIMENT KF3_DISPLAY_BLANK_STATE=1 — a new core life reads BLANK in every head's SET_GET_BLANKING_CTRL ({} words)",
                dp.map.core_birth_words(true, false).len()
            );
        }
        if armed_defaults {
            eprintln!(
                "kf3: display: EXPERIMENT KF3_DISPLAY_ARMED_DEFAULTS=1 — a new core life's ARMED HEAD_SET_MIN_FRAME_IDLE is the engine default (leading 2, trailing 1; {} words)",
                dp.map.core_birth_words(false, true).len()
            );
        }
        let wtrace = write_trace_on();
        let vsync_traced = AtomicU32::new(0);
        let latch_traced = AtomicU32::new(0);
        if wtrace {
            // anchor the trace clock here, so no vCPU ever pays its first `/proc/uptime` read
            let _ = kf_mem::maplog::t();
            eprintln!(
                "kf3: display: WRITE TRACE diagnostic enabled (guest display writes, raised head-timing interrupts, window latches; {} lines per kind)",
                display_trace_cap()
            );
        }
        if loadv {
            eprintln!(
                "kf3: display: EXPERIMENT KF3_DISPLAY_LOADV=1 — every frame edge also sets EVT_STAT_HEAD_TIMING bit {EVT_STAT_HEAD_TIMING_LOADV:#x} (LOADV), and the event registers are republished at every edge"
            );
        }
        if std::env::var("KF3_DISPLAY_METHOD_TRACE").is_ok_and(|v| v == "1") {
            engine.trace_methods(kf_disp::engine::MAX_METHOD_TRACE);
            eprintln!("kf3: display: bounded METHOD diagnostic enabled (65536 DMA writes maximum)");
        }
        eprintln!(
            "kf3: display worker up — engine {} heads / {} windows, caps page published",
            dp.map.heads, dp.map.windows
        );
        let store = |o: u64, v: u32| self.shadow_store(o, u64::from(v), 4);
        // ★ the display shared-data page (`Statement::SharedData`) and each head's vblank count in it
        let mut shared_data: Option<kf_disp::model::InstMem> = None;
        let mut vblank_counts = [0u32; MAX_HEADS];
        let mut io = Io {
            dev: self,
            dp,
            gpu,
            layout,
            inst: None,
            img: None,
            notifier_finished,
            notifier_begun,
            refusals_logged: 0,
        };
        // ★ `display-max-fps` (§8.16): every head's vblank tick, CAPPED — the pacer owns the periods
        // (this loop does no period arithmetic of its own), on the worker's own clock
        let clock = Instant::now();
        let ns = |t: Instant| {
            u64::try_from(t.saturating_duration_since(clock).as_nanos()).unwrap_or(u64::MAX)
        };
        let cap = kf_disp::pace::cap_hz(dp.max_fps);
        let mut pacer = Pacer::new(cap);
        let mut meter = Meter::default();
        // ★ phase 2 (`V3_DISPLAY_PACING_20261017.md` §11): the broker's ticks and requests come from
        // the relay through a lock-free share; the gate decides when a request may be acted on
        let mut gate_c = CreditGate::new();
        let mut tick_seen = 0u64;
        let mut tick_epoch = 0u32;
        let mut shown_head: Option<u32> = None;
        if let Some(b) = dp.broker.as_ref() {
            b.ticks().set_wake(dp.wake.clone());
            b.ticks().set_cap_hz(cap);
        }
        // per head: ticks, and ticks with the guest's vblank interrupt enabled (presents: the engine)
        let mut counts = [HeadCounts::default(); MAX_HEADS];
        let mut fps_printed: Option<(Instant, String)> = None;
        let mut watch_epoch = dp.console.watch_epoch();
        let mut last_stall_check = Instant::now();
        let mut parked_since: std::collections::HashMap<(u32, u32), Instant> =
            std::collections::HashMap::new();
        let mut stall_reported: std::collections::HashSet<(u32, u32)> =
            std::collections::HashSet::new();
        let mut stall_logged = 0u32;
        let mut handoff_lines = 0u32;
        // ★ the last effects the plane performed (notifier writes, releases, latches), for the STALL report
        let mut fx_ring: VecDeque<(u64, &'static str, u32, u32, u64, u64, bool)> = VecDeque::new();
        let mut fx_seq = 0u64;
        // Boot/preserved pictures have no armed head: their non-flip checks run at the preferred
        // rate under the cap
        let idle_period = Duration::from_nanos(kf_disp::pace::paced_period_ns(
            kf_disp::pace::cap_period_ns(kf_disp::pace::DEFAULT_PREFERRED_HZ),
            cap,
        ));
        eprintln!(
            "kf3: display: display-max-fps {} — every head's vblank tick is capped at {cap} Hz; \
             copies without a flip are made at the console head's tick, while watched, and sent \
             only when the frame's checksum changed",
            if dp.max_fps == 0 {
                "unset".to_string()
            } else {
                format!("{} Hz", dp.max_fps)
            }
        );
        if let Some(e) = io.gpu.as_ref().and_then(|g| g.checksum_refused()) {
            eprintln!(
                "kf3: display: the checksum kernel is REFUSED ({e}) — every non-flip check is sent"
            );
        }
        let mut scan = ScanState {
            trace,
            bl_ok,
            vram,
            ..ScanState::default()
        };
        let mut queue: VecDeque<Queued> = VecDeque::new();
        // ★ 2026-10-10: a head's frame edge is OWED until its vblank's completions are published (`vblankgate`)
        let mut gate = crate::vblankgate::VblankGate::default();
        let mut edge_frame = [0u32; MAX_HEADS];
        let mut forced_logged = 0u64;
        let mut put_head = 0u64;
        let mut put_dropped = 0u64;
        let mut cursor_seen = [0u32; MAX_HEADS];
        let mut published_get = [u32::MAX; kf_disp::ports::NUM_CHANNELS];
        let mut logged_updates = 0u32;
        // ★ The boot display: sticky once the guest arms its first head — until a guest life ends
        // (`DisplayPlane::rm_unloaded`, a device reset).
        let mut boot_done = false;
        let mut unloads_seen = dp.unloads.load(Ordering::Acquire);
        let mut life_ends = 0u64;
        // ★ B5 (`V3_DISPLAY.md` §4.11.13): what the console shows when nothing is scanned
        let mut shown_last: Option<u64> = None;
        let mut shown_lines = 0u32;
        let mut held = Held::default();
        let mut windowless_since: Option<Instant> = None;
        let started = Instant::now();
        if let Some(b) = dp.boot.as_ref() {
            eprintln!(
                "kf3: display: boot layer — store [0, {:#x}) as {}x{} pitch {}, shown until the guest arms a head",
                b.layer.extent, b.size.0, b.size.1, b.layer.pitch
            );
        }
        self.display_up.store(true, Ordering::Release);
        let _up = crate::device::Up(&self.display_up);
        while !self.stop.load(Ordering::Acquire) {
            // the deadline: the earliest (capped) vblank, a 2 ms acquire poll, the boot picture's
            // check clock, or 50 ms
            let now = Instant::now();
            let mut deadline = now + Duration::from_millis(50);
            if let Some(t) = pacer.next_ns() {
                deadline = deadline.min(clock + Duration::from_nanos(t));
            }
            if engine.acquire_pending() {
                deadline = deadline.min(now + Duration::from_millis(2));
            }
            if let Some(t) = scan.idle_at {
                deadline = deadline.min(t);
            }
            if let Some(t) = gate_c.next_ns() {
                deadline = deadline.min(clock + Duration::from_nanos(t));
            }
            if let Some(t) = gate.deadline() {
                deadline = deadline.min(t);
            }
            // a lit head without a window: wake when its hold ends (then the console may go black)
            if let Some(t) = windowless_since.map(|t| t + WINDOWLESS_HOLD)
                && t > now
            {
                deadline = deadline.min(t);
            }
            let ms = u32::try_from(deadline.saturating_duration_since(now).as_millis())
                .unwrap_or(50)
                .max(1);
            let mut ready = ReadyTokens::new();
            let _ = poller.wait(&mut ready, PollTimeout::Millis(ms));
            let _ = dp.wake.drain();
            io.img = None;
            // ★★ A guest life ended (`docs/design/V3_DEVICE_RESET.md`): a device reset, or the
            // guest's RM unloading. Once no copy is in flight (its completion wakes this loop), every
            // per-life piece of the worker goes back to what the loop started with; the queued
            // completions of the old life are DROPPED — never written, never forged.
            let owed = dp.life_reset.owed();
            let unloads = dp.unloads.load(Ordering::Acquire);
            if (owed.is_some() || unloads != unloads_seen) && scan.inflight.is_none() {
                unloads_seen = unloads;
                let channels = dp.model.lock().map_or(0, |mut m| m.reset_life());
                engine.reset_life();
                scan.reset_life();
                held = held_after_life_reset(&held);
                boot_done = false;
                windowless_since = None;
                let dropped = queue.len();
                queue.clear();
                gate = crate::vblankgate::VblankGate::default();
                pacer = Pacer::new(cap);
                gate_c.reset();
                edge_frame = [0u32; MAX_HEADS];
                cursor_seen = [0u32; MAX_HEADS];
                published_get = [u32::MAX; kf_disp::ports::NUM_CHANNELS];
                parked_since.clear();
                stall_reported.clear();
                io.inst = None;
                io.img = None;
                shown_last = None;
                life_ends += 1;
                if crate::reset::loud(life_ends) {
                    eprintln!(
                        "kf3: display: +{} ms the guest's display life ENDED ({}): {channels} model channel(s), {dropped} queued completion(s) dropped (never written){}",
                        started.elapsed().as_millis(),
                        if owed.is_some() {
                            "device reset"
                        } else {
                            "its RM unloaded"
                        },
                        if dp.boot.is_some() {
                            " — the boot layer is shown again"
                        } else {
                            ""
                        }
                    );
                }
                if let Some(e) = owed {
                    dp.life_reset.done(e);
                    let _ = self.drainer_efd.signal();
                }
                continue;
            }
            let pass_t0 = dp.counters.lat_pass.stamp();
            let mut effects: Vec<Effect> = Vec::new();
            let mut gets: Vec<(u32, u32, u32)> = Vec::new();
            // 1. statements (the model's lock, only to take them)
            let st = dp
                .model
                .lock()
                .map(|mut g| g.take_statements())
                .unwrap_or_default();
            for s in st {
                eprintln!("kf3: display: {s:?}");
                match s {
                    Statement::SharedData(page) => {
                        shared_data = page;
                        vblank_counts = [0; MAX_HEADS];
                    }
                    Statement::InstMem(im) => {
                        io.inst = Some(im);
                        // a new instance memory voids every resolution made from the old one
                        scan.latched = LatchedDmas::default();
                        scan.colors.slots.fill(None);
                        // ⊘ The guest never zeroes instance memory (`disp_inst_mem.c:170-201`): a stale
                        // hash entry from an earlier driver life would resolve. Zeroed by the GPU.
                        if im.addr_space == 2
                            && let Some(g) = &io.gpu
                            && let Err(e) =
                                g.zero_store(im.phys, im.size.min(io.layout.inst_bytes()))
                        {
                            io.refuse(&format!("zeroing instance memory: {e}"));
                        }
                    }
                    Statement::ChannelAllocated {
                        kind,
                        instance,
                        offset,
                        client,
                        pb,
                        life,
                    } => {
                        // ⊘ bounded by the declared size, the context DMA's limit and the decoder's 4 KiB
                        let loc = pb.map(|p| {
                            let lim = u32::try_from(p.limit.saturating_add(1)).unwrap_or(u32::MAX);
                            PbLoc {
                                sysmem: p.addr_space == 1,
                                addr: p.phys,
                                bytes: p.bytes().min(lim).min(kf_disp::pushbuf::MAX_PUSHBUFFER)
                                    & !3,
                            }
                        });
                        if pb.is_some_and(|p| p.addr_space != 1 && p.addr_space != 2) {
                            io.refuse(&format!("{kind:?} {instance}: pushbuffer address space is neither sysmem nor vidmem"));
                        }
                        if kind == ChannelKind::Window {
                            scan.latched.forget(instance);
                            scan.colors.forget_window(instance as usize);
                        } else if kind == ChannelKind::Core {
                            scan.colors.forget(64);
                        }
                        if let Some(chn) = engine.alloc(kind, instance, client, life, loc, offset) {
                            let base = dp.map.user_base(kind, instance);
                            store(base + dp.map.put, offset);
                            store(base + dp.map.get, offset);
                            published_get[chn as usize] = offset;
                            if kind == ChannelKind::Core {
                                // a new core life starts with an empty ARMED mirror
                                for o in (0..dp.map.core_len / 2).step_by(4) {
                                    store(dp.map.core_armed + o, 0);
                                }
                                // ⚠ H-blankstate / H-armeddefault (default off): the hardware's
                                // read-backs at a core channel's birth
                                for (o, v) in dp.map.core_birth_words(blank_state, armed_defaults) {
                                    store(o, v);
                                }
                            }
                            self.display_chan_status(kind, instance, Some(true));
                        }
                    }
                    Statement::ChannelFreed {
                        kind,
                        instance,
                        preserve,
                    } => {
                        // ★ B5: a preserving free keeps the last armed scanout on the monitor.
                        held.channel_freed(preserve, scan.last_plan.as_ref());
                        if kind == ChannelKind::Window {
                            scan.latched.forget(instance);
                            scan.colors.forget_window(instance as usize);
                        } else if kind == ChannelKind::Core {
                            scan.colors.forget(64);
                        }
                        engine.free(kind, instance);
                        self.display_chan_status(kind, instance, None);
                    }
                }
            }
            // 1b. ★ 3c: a resize asked for by the VMM's UI — a new monitor, then a hotplug
            let req = dp.ui_request.swap(0, Ordering::AcqRel);
            if req != 0 {
                self.apply_ui_request(dp, req);
            }
            // 2. every DMA channel whose PUT moved — ★ 2026-10-11 in the ORDER THE GUEST WROTE THE PUTs. The hardware
            // fetches each channel as its PUT arrives, so UPDATEs become pending in arrival order; applying the
            // channels in number order at whatever moment the worker wakes pairs them differently (the overlay
            // stall H1: window 0's UPDATE joined an earlier group of window 4's, leaving window 4's first flip
            // without partners). The latest-PUT pass below runs only to resynchronise after the log lost writes.
            let mut step_put = |chn: u32, put: u32| {
                let Some((pb, decoded, life)) = engine.pushbuffer(chn) else {
                    return;
                };
                if put == decoded || dp.ports.generation(chn) != life {
                    return;
                }
                match io.pushbuffer(pb) {
                    Ok(bytes) => {
                        let s = engine.step(chn, &bytes, put, &mut |a| io.acquired(a));
                        if trace {
                            eprintln!(
                                "kf3: display: chn {chn} PUT {put:#x}: {} effects",
                                s.effects.len()
                            );
                        }
                        effects.extend(s.effects);
                        gets.extend(s.gets);
                    }
                    Err(e) => io.refuse(&format!("channel {chn}: {e}")),
                }
            };
            while queue.len() < QUEUE_CAP {
                // backpressure otherwise: PUT stays behind in the guest's ring (see QUEUE_CAP) and the log keeps the rest
                match dp.ports.put_log.next(&mut put_head) {
                    kf_disp::ports::PutPoll::Put(chn, put) => step_put(chn, put),
                    kf_disp::ports::PutPoll::Empty | kf_disp::ports::PutPoll::Pending => break,
                }
            }
            if dp.ports.put_log.dropped() != put_dropped {
                put_dropped = dp.ports.put_log.dropped();
                for chn in 0..kf_disp::ports::NUM_CHANNELS as u32 {
                    if queue.len() >= QUEUE_CAP {
                        break;
                    }
                    step_put(chn, dp.ports.put(chn));
                }
            }
            // 3. cursors: every posted Update
            for h in 0..dp.map.heads {
                if queue.len() >= QUEUE_CAP {
                    break; // backpressure, as above: the posts stay pending
                }
                let c = &dp.cursor[h as usize];
                let n = c.updates.load(Ordering::Acquire);
                if n == cursor_seen[h as usize] {
                    continue;
                }
                cursor_seen[h as usize] = n;
                for i in 0..c.offs.len() {
                    let o = c.offs[i].load(Ordering::Acquire);
                    if o != 0 {
                        let s = engine.cursor_write(
                            h,
                            o - 1,
                            c.regs[i].load(Ordering::Acquire),
                            &mut |a| io.acquired(a),
                        );
                        effects.extend(s.effects);
                    }
                }
                let s = engine.cursor_write(h, dp.map.cursor_update, 0, &mut |a| io.acquired(a));
                effects.extend(s.effects);
                gets.extend(s.gets);
            }
            // 4. vblanks whose time has come — each head's CAPPED tick (§8.16), from the pacer
            let mut raised = false;
            let mut ticked = 0u32;
            // ★ phase 2: heads whose tick came from the FREE clock — only those grant a copy credit
            // by themselves; a broker tick grants one only when it carries a request
            let mut ticked_free = 0u32;
            let now_ns = ns(Instant::now());
            let mut due_ticks = pacer.due(now_ns);
            // a NEW broker connection numbers its ticks from 1 again: forget the old numbers
            if let Some(b) = dp.broker.as_ref()
                && b.ticks().epoch() != tick_epoch
            {
                tick_epoch = b.ticks().epoch();
                gate_c.reset();
            }
            if let Some(b) = dp.broker.as_ref()
                && let Some(snap) = b.ticks().snapshot(tick_seen)
            {
                let coalesced = snap.count.saturating_sub(tick_seen + 1);
                tick_seen = snap.count;
                let want = b.ticks().take_want();
                let t = if gate_c.fresh(snap.tick_seq) {
                    shown_head.and_then(|h| pacer.broker_tick(h, now_ns))
                } else {
                    None
                };
                let (last, guard) = shown_head
                    .and_then(|h| pacer.source(h as usize))
                    .map_or((None, 0), |s| (s.last_tick_ns(), s.guard_ns()));
                gate_c.on_tick(
                    snap.tick_seq,
                    coalesced,
                    want,
                    t.map(|_| now_ns),
                    last,
                    guard,
                );
                due_ticks.extend(t);
            }
            if gate_c.poll(now_ns).is_some() {
                scan.mailbox.grant();
            }
            for t in due_ticks {
                let h = t.head as usize;
                if h >= dp.map.heads as usize {
                    continue;
                }
                if queue.len() >= QUEUE_CAP {
                    continue; // backpressure, as above: no new vblank effects while the queue is full
                }
                dp.counters.vblanks.fetch_add(1, Ordering::Relaxed);
                let s = engine.vblank(t.head, &mut |a| io.acquired(a));
                effects.extend(s.effects);
                gets.extend(s.gets);
                // ★ the frame edge itself (frame counters, head-timing event, interrupt) is NOT raised here: the
                // hardware raises it after the latch it announces — it is owed until step 7b
                edge_frame[h] = dp.ports.frames[h]
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1);
                counts[h].ticks += 1;
                ticked |= 1 << h;
                if t.from == kf_disp::tick::Source::Free {
                    ticked_free |= 1 << h;
                }
            }
            // ★ §AK: who ticks a head changed (phase 2: the broker speaks, or has been silent for
            // `kf_disp::tick::BROKER_SILENCE_NS`) — one line per handoff, bounded
            for (h, ho, at) in pacer.take_handoffs() {
                if handoff_lines < 32 {
                    handoff_lines += 1;
                    eprintln!(
                        "kf3: display: +{} ms head {h} tick source -> {} ({})",
                        at / 1_000_000,
                        match ho {
                            kf_disp::tick::Handoff::ToBroker => "BROKER",
                            kf_disp::tick::Handoff::ToFree => "FREE-RUNNING clock",
                        },
                        match ho {
                            kf_disp::tick::Handoff::ToBroker => "a broker tick event arrived",
                            kf_disp::tick::Handoff::ToFree =>
                                "no broker tick event within the silence deadline",
                        }
                    );
                }
            }
            // 4b. ★ a loud, bounded STALL report (2026-10-10, runs 400/401: an overlay-plane flip never
            // completed after ~10 min of playback and the guest declared a TDR 2 s later): when an
            // update has been parked at its UPDATE for over a second, name what it waits for,
            // once per episode and at most 16 lines per VM. Read-only; decides nothing.
            if last_stall_check.elapsed() >= Duration::from_millis(250) {
                last_stall_check = Instant::now();
                let parked = engine.parked();
                let now = Instant::now();
                parked_since.retain(|k, _| parked.iter().any(|p| (p.chn, p.update) == *k));
                for p in &parked {
                    let first = *parked_since.entry((p.chn, p.update)).or_insert(now);
                    if now.duration_since(first) >= Duration::from_secs(1)
                        && stall_logged < 16
                        && !stall_reported.contains(&(p.chn, p.update))
                    {
                        stall_reported.insert((p.chn, p.update));
                        stall_logged += 1;
                        let seen = p.acquire.map(|a| io.peek_acquire(&a));
                        eprintln!(
                            "kf3: display: STALL {:?} {} (chn {}) update {:#x} parked {} ms: {} group/interlock bits {:#x} head {:?} acquire {:?} current {:?}; every parked channel: {:?}",
                            p.kind,
                            p.instance,
                            p.chn,
                            p.update,
                            now.duration_since(first).as_millis(),
                            if p.waiting_for_interlock {
                                "waiting for its interlock group"
                            } else {
                                "ready, waiting for a vblank or its acquire"
                            },
                            p.set,
                            p.head,
                            p.acquire,
                            seen,
                            parked
                                .iter()
                                .map(|q| (q.chn, q.update, q.waiting_for_interlock))
                                .collect::<Vec<_>>()
                        );
                        eprintln!(
                            "kf3: display: STALL recent effects (seq, kind, chn, handle, offset, awaken-or-value, ok; oldest first): {:?}",
                            fx_ring.iter().rev().take(60).rev().collect::<Vec<_>>()
                        );
                        eprintln!(
                            "kf3: display: STALL recent events (seq, event; oldest first): {:?}",
                            {
                                let e = engine.recent_events();
                                let from = e.len().saturating_sub(200);
                                e[from..].to_vec()
                            }
                        );
                        for chn in [0u32, 1, 5, 33, 37] {
                            let words = engine.pushbuffer(chn).and_then(|(pb, decoded, _)| {
                                io.pushbuffer(pb).ok().map(|b| {
                                    let at = (decoded as usize & !3).saturating_sub(16);
                                    b.get(at..(at + 64).min(b.len()))
                                        .unwrap_or(&[])
                                        .chunks_exact(4)
                                        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                                        .collect::<Vec<_>>()
                                })
                            });
                            eprintln!(
                                "kf3: display: STALL chn {chn}: PUT {:#x} ports-generation {} engine {:?} generation {:?}; ring words from decoded-16: {:x?}",
                                dp.ports.put(chn),
                                dp.ports.generation(chn),
                                engine.chan_state(chn),
                                engine.generation(chn),
                                words
                            );
                        }
                        eprintln!(
                            "kf3: display: STALL context: queue {}/{QUEUE_CAP} copies started {} done {} barrier {} inflight {} want {} failed {} nonflip_pending {} vblanks_total {} ticks_per_head {:?} heads_armed {:?}",
                            queue.len(),
                            scan.started,
                            scan.done,
                            scan.barrier,
                            scan.inflight.is_some(),
                            scan.want,
                            scan.failed,
                            scan.nonflip.pending(),
                            dp.counters.vblanks.load(Ordering::Relaxed),
                            counts.iter().map(|c| c.ticks).collect::<Vec<_>>(),
                            engine
                                .heads_armed()
                                .iter()
                                .map(|m| (m.head, m.period_ns))
                                .collect::<Vec<_>>()
                        );
                    }
                }
                stall_reported.retain(|k| parked.iter().any(|p| (p.chn, p.update) == *k));
            }
            // 5. acquires waiting without a vblank
            if engine.acquire_pending() {
                let s = engine.poll_acquires(&mut |a| io.acquired(a));
                effects.extend(s.effects);
                gets.extend(s.gets);
            }
            // 6. ★ M2: the scanout. A flip of the window the console shows needs a copy of its NEW
            // surface, and every completion from that flip on waits for that copy to COMPLETE (the
            // flip-complete notifier and the release that frees the old surface, then GET): the
            // queue below keeps effect order and holds each item until its copy is done.
            if !boot_done
                && dp.boot.is_some()
                && engine.heads_armed().iter().any(|m| m.period_ns > 0)
            {
                boot_done = true;
                let ms = u64::try_from(started.elapsed().as_millis())
                    .unwrap_or(u64::MAX)
                    .max(1);
                dp.counters.boot_done_ms.store(ms, Ordering::Relaxed);
                eprintln!(
                    "kf3: display: the guest armed its first head at +{ms} ms — the boot layer is retired after {} boot frame(s)",
                    dp.counters.boot_frames.load(Ordering::Relaxed)
                );
            }
            let console = console_composition(&engine, dp);
            let lit = engine.heads_armed().iter().any(|m| m.period_ns > 0);
            held.dark = match (&console, lit) {
                (Some(_), _) => Dark::No,
                (None, false) => Dark::Unlit,
                (None, true) => {
                    let now = Instant::now();
                    let since = *windowless_since.get_or_insert(now);
                    if now.duration_since(since) >= WINDOWLESS_HOLD {
                        Dark::Windowless
                    } else {
                        Dark::WindowlessBrief
                    }
                }
            };
            if !matches!(held.dark, Dark::WindowlessBrief | Dark::Windowless) {
                windowless_since = None;
            }
            if console.is_some() {
                held.preserved = None;
            }
            let shown = choose_shown_gated(
                console,
                dp.boot.as_ref(),
                boot_done,
                dp.boot_clean.is_clean(),
                &held,
            );
            shown_head = match &shown {
                Some(Shown::Armed(c)) if c.head < 32 => Some(c.head),
                _ => None,
            };
            match &shown {
                Some(Shown::Armed(c)) => held.scanned = Some((c.width, c.height)),
                // only the composition shown right before a preserving free is kept
                _ => scan.last_plan = None,
            }
            // ★ 2026-10-03 (B5, `V3_DISPLAY.md` §4.11.13): every change of WHAT the console shows,
            // timed — the boot layer, a head's windows, black, the preserved scanout, or no new frame
            // and why. Keyed without what a flip changes; bounded.
            let key = shown_key(shown.as_ref(), &held, boot_done);
            if shown_last != Some(key) {
                if shown_lines < SHOWN_LINES {
                    eprintln!(
                        "kf3: display: +{} ms the console shows {} (core channel {})",
                        started.elapsed().as_millis(),
                        shown_digest(shown.as_ref(), &held, boot_done),
                        if engine.generation(0).is_some() {
                            "allocated"
                        } else {
                            "FREE"
                        }
                    );
                } else if shown_lines == SHOWN_LINES {
                    eprintln!(
                        "kf3: display: {SHOWN_LINES} 'console shows' lines logged — later changes are not"
                    );
                }
                shown_lines = shown_lines.saturating_add(1);
                shown_last = Some(key);
                scan.want = true;
            }
            scan.active = matches!(
                shown,
                Some(Shown::Armed(_) | Shown::Boot(..) | Shown::Preserved(..))
            );
            // ★ 3d: the cursor on top of the console's head (an ARMED composition only: the boot,
            // preserved and blank pictures have no cursor channel behind them); a move or a new
            // image recomposes
            let cursor = match &shown {
                Some(Shown::Armed(c)) => dp
                    .cursor_vocab
                    .as_ref()
                    .and_then(|cv| engine.cursor_scan(cv, c.head)),
                _ => None,
            };
            // ★ §8.13: where the cursor image's top-left is, for the console's cursor in hover (a
            // move makes no frame then, but the console still follows it)
            let point = cursor.as_ref().map(|cs| {
                (
                    cs.x.saturating_sub(i32::try_from(cs.hot_x).unwrap_or(0)),
                    cs.y.saturating_sub(i32::try_from(cs.hot_y).unwrap_or(0)),
                )
            });
            dp.console.note_cursor_point(point);
            // ★ the same point for a streaming broker (`CAP_CURSOR_STREAM`): the relay sends it as a
            // MOVE under grab, when it changed — lock-free, once per pass, never per input event
            if let Some(b) = dp.broker.as_ref()
                && b.cursor().note_point(point)
                && b.cursor().stream()
            {
                // the top-left changed, or a confirmation the relay armed is due: one non-blocking
                // eventfd write wakes the relay (no frame would — the cursor is not in the frames)
                b.frame_published();
            }
            // ★ §O: where the cursor goes, as the relay decided (hover: the host shows it; grab or
            // no cursor-capable broker: the frame). A switch recomposes at once — the cursor goes
            // into the frame or out of it — and in hover a MOVE makes no frame (the host pointer
            // is the guest's), unless the last frame composed the cursor (one the host cannot
            // show). Off and grab recompose on a move exactly as before.
            let cursor_mode = dp
                .broker
                .as_ref()
                .map_or(CursorMode::Off, |b| b.cursor().mode());
            // ★ §8.16 (D2): a cursor-mode switch, a cursor move, front-buffer drawing — none makes a
            // copy of its own any more: the console head's next tick composes and checksums the
            // frame and sends it when it changed (a hover move, which composes nothing, changes
            // nothing). A switch forces that send (the frame's cursor flag changes either way).
            if cursor_mode != scan.cursor_mode {
                scan.cursor_mode = cursor_mode;
                scan.nonflip.force();
            }
            let epoch = dp.console.watch_epoch();
            if epoch != watch_epoch {
                watch_epoch = epoch;
                scan.nonflip.force();
            }
            scan.nonflip.request(dp.console.refresh_requested());
            for e in effects {
                if dp.sdr_color.is_some()
                    && matches!(e, Effect::CoreArmed(_))
                    && matches!(shown, Some(Shown::Armed(_)))
                {
                    // Output colour belongs to the core update, not a window flip.
                    scan.barrier = scan.started + 1;
                    scan.want = true;
                    scan.colors.forget(64);
                }
                if let Effect::Latched { window } = &e {
                    // ★ review hitch-20261017: ONE call into ScanState (the property tests drive that method); this
                    // block holds nothing else but what the call resolves (`the_worker_latch_is_one_method_call_...`)
                    let so = match &shown {
                        Some(Shown::Armed(c)) => {
                            c.layers.iter().find(|l| l.window == *window).copied()
                        }
                        _ => None,
                    };
                    scan.on_window_latch(*window, so.is_some(), |scan| {
                        if let Some(so) = so {
                            let t_res = dp.counters.lat_resolve.stamp();
                            scan.resolve_now(&mut io, &so);
                            if let Some(sdr) = dp.sdr_color {
                                // the colour path: its LUT / TMO context DMAs are resolved now too (a refusal is never kept)
                                let _ = resolve_window_colors(
                                    &mut scan.colors,
                                    &engine,
                                    &mut io,
                                    sdr,
                                    &so,
                                );
                            }
                            dp.counters.lat_resolve.finish(t_res);
                        }
                    });
                }
                queue.push_back(Queued {
                    need: scan.barrier,
                    item: Item::Effect(e),
                });
                gate.pushed();
            }
            for (chn, life, get) in gets {
                queue.push_back(Queued {
                    need: scan.barrier,
                    item: Item::Get(chn, life, get),
                });
                gate.pushed();
            }
            // every head that ticked: its edge is owed until everything queued up to its vblank is published
            let tick_at = Instant::now();
            for h in 0..MAX_HEADS {
                if ticked & (1 << h) != 0 {
                    gate.tick(h, tick_at);
                }
            }
            if io
                .gpu
                .as_ref()
                .is_some_and(|g| g.completion_fd().drain() > 0)
            {
                scan.completed(&mut io);
            }
            scan.give_up_if_stuck(&io);
            // ★ D2: the console head's tick — the armed head's own, or for a picture with no armed
            // head (the boot layer, a preserved scanout) the check clock
            let watched = dp.console.wanted_within(WATCHED_MS);
            let idle_run = matches!(shown, Some(Shown::Boot(..) | Shown::Preserved(..)))
                && (watched || scan.nonflip.pending());
            let console_tick = match &shown {
                Some(Shown::Armed(c)) => c.head < 32 && ticked & (1 << c.head) != 0,
                _ => scan.idle_tick(Instant::now(), idle_period, idle_run),
            };
            if !idle_run {
                scan.idle_at = None;
            }
            if scan.nonflip.pending() && (!scan.active || io.gpu.is_none()) {
                // nothing can be copied: the frame the console has is the answer
                scan.serve_now(dp);
            }
            // ★ §AK: the shown head's tick allows ONE flip copy (of the newest latched frame)
            // (a BROKER tick grants its copy credit through the gate, above; only a free-clock tick
            // grants one by itself)
            if match &shown {
                Some(Shown::Armed(c)) => c.head < 32 && ticked_free & (1 << c.head) != 0,
                _ => false,
            } {
                scan.on_console_tick(true);
            }
            if (scan.want || scan.mailbox.due(scan.inflight.is_some())) && scan.inflight.is_none() {
                let t_st = dp.counters.lat_start.stamp();
                scan.start(&mut io, &engine, shown.as_ref(), cursor.as_ref(), false);
                dp.counters.lat_start.finish(t_st);
            } else if scan.nonflip.due(
                console_tick,
                watched,
                scan.active,
                scan.inflight.is_none(),
                scan.flip_pending(),
            ) {
                let t_st = dp.counters.lat_start.stamp();
                scan.start(&mut io, &engine, shown.as_ref(), cursor.as_ref(), true);
                dp.counters.lat_start.finish(t_st);
            }
            if scan.failed {
                // No synthetic success after rejected colour state or work sent to the GPU.
                // Clear bounded pending effects and stop decoding, keeping published GETs.
                queue.clear();
                engine.halt_scanout();
            }
            // 7. completions, IN ORDER — each after the state it reports and the copy it follows
            while queue.front().is_some_and(|q| q.need <= scan.done) {
                let Some(Queued { item, .. }) = queue.pop_front() else {
                    break;
                };
                let e = match item {
                    Item::Effect(e) => e,
                    Item::Get(chn, life, get) => {
                        if published_get[chn as usize] == get && dp.ports.get(chn) == get {
                            continue;
                        }
                        if dp.ports.publish_get(chn, life, get) {
                            published_get[chn as usize] = get;
                            if let Some((kind, inst)) = chan_of(chn) {
                                store(dp.map.user_base(kind, inst) + dp.map.get, get);
                                self.display_chan_status(
                                    kind,
                                    inst,
                                    Some(engine_idle(&engine, dp, chn)),
                                );
                            }
                        }
                        continue;
                    }
                };
                match e {
                    Effect::CoreArmed(words) => {
                        for (m, v) in words {
                            store(dp.map.core_armed + u64::from(m), v);
                        }
                        // the display each head lights (`SYSTEM_GET_ACTIVE`), from the state just armed
                        for (h, sor) in engine.lit_sors().into_iter().enumerate() {
                            dp.ports.set_lit_sor(h, sor);
                        }
                        // and its armed mode (`0x73011a`)
                        for (h, t) in engine.head_timings().into_iter().enumerate() {
                            dp.ports.set_head_timing(h, t);
                        }
                        // and the raster its live scan line follows, and the head of every window
                        for (h, g) in engine.head_scan_geoms().into_iter().enumerate() {
                            dp.ports.set_scan_geom(h, g);
                        }
                        for (w, h) in engine.window_heads().into_iter().enumerate() {
                            dp.ports.set_window_head(w, h);
                        }
                    }
                    Effect::Notify {
                        chn,
                        client,
                        handle,
                        offset,
                        awaken,
                        finished,
                    } => {
                        let status = if finished {
                            io.notifier_finished
                        } else {
                            io.notifier_begun
                        };
                        let r = io.resolve(client, handle, chn).and_then(|dma| {
                            let ts = self.rm.gpu_time_ns().unwrap_or(0);
                            let mut n = [0u8; 16];
                            n[8..12].copy_from_slice(&(ts as u32).to_le_bytes());
                            n[12..16].copy_from_slice(&((ts >> 32) as u32).to_le_bytes());
                            // the timestamp words first, the status word (what the guest polls) last
                            io.write(dma, offset + 4, &n[4..16])?;
                            io.write(dma, offset, &status.to_le_bytes())
                        });
                        fx_seq += 1;
                        if fx_ring.len() >= 192 {
                            fx_ring.pop_front();
                        }
                        fx_ring.push_back((
                            fx_seq,
                            if finished {
                                "NOTIFY-FINISHED"
                            } else {
                                "NOTIFY-BEGUN"
                            },
                            chn,
                            handle,
                            offset,
                            u64::from(awaken),
                            r.is_ok(),
                        ));
                        if trace {
                            eprintln!(
                                "kf3: display: TRACE notify chn {chn} handle {handle:#x} +{offset:#x} awaken={awaken} -> {r:?}"
                            );
                        }
                        match r {
                            Ok(()) => {
                                dp.counters.notifies.fetch_add(1, Ordering::Relaxed);
                                if awaken {
                                    if chn == 0 {
                                        dp.ports
                                            .raise(EventReg::AwakenOther, dp.map.awaken_core_bit);
                                    } else if (1..=32).contains(&chn) {
                                        dp.ports.raise(EventReg::AwakenWin, 1 << (chn - 1));
                                    }
                                    raised = true;
                                }
                            }
                            Err(e) => io.refuse(&format!("notifier: {e}")),
                        }
                    }
                    Effect::Release {
                        chn,
                        client,
                        handle,
                        offset,
                        value,
                        wide,
                        awaken,
                    } => {
                        let r = io.resolve(client, handle, chn).and_then(|dma| {
                            let b = value.to_le_bytes();
                            io.write(dma, offset, if wide { &b[..] } else { &b[..4] })
                        });
                        fx_seq += 1;
                        if fx_ring.len() >= 192 {
                            fx_ring.pop_front();
                        }
                        fx_ring.push_back((
                            fx_seq,
                            "RELEASE",
                            chn,
                            handle,
                            offset,
                            value,
                            r.is_ok(),
                        ));
                        if trace {
                            eprintln!(
                                "kf3: display: TRACE release chn {chn} handle {handle:#x} +{offset:#x} value {value:#x} -> {r:?}"
                            );
                        }
                        match r {
                            Ok(()) => {
                                dp.counters.releases.fetch_add(1, Ordering::Relaxed);
                                if awaken && (1..=32).contains(&chn) {
                                    dp.ports.raise(EventReg::SemWin, 1 << (chn - 1));
                                    raised = true;
                                }
                            }
                            Err(e) => io.refuse(&format!("semaphore release: {e}")),
                        }
                    }
                    Effect::Heads => {
                        // ★ §8.16: the pacer arms each head at its CAPPED period, in phase when that
                        // did not change
                        let heads = engine.heads_armed();
                        for a in pacer.on_heads(&heads, ns(Instant::now())) {
                            let active = a.period_ns > 0;
                            if let Some((b, s, fld, awake, sleep)) = dp.map.core_head_state {
                                store(
                                    b + u64::from(a.head) * s,
                                    kf_disp::class::put(0, fld, if active { awake } else { sleep }),
                                );
                            }
                            eprintln!("kf3: display: {}", a.line(cap));
                        }
                    }
                    Effect::Latched { window } => {
                        if trace {
                            eprintln!("kf3: display: TRACE window {window} latched");
                        }
                        if wtrace && trace_slot(&latch_traced, display_trace_cap()) {
                            eprintln!(
                                "kf3: display: WTRACE t={:.6} LATCH window {window}",
                                kf_mem::maplog::t()
                            );
                        }
                    }
                    Effect::Trace(line) => eprintln!("kf3: display: TRACE {line}"),
                    Effect::Exception { chn, at, what } => {
                        dp.counters.exceptions.fetch_add(1, Ordering::Relaxed);
                        eprintln!("kf3: display: channel {chn} STOPPED at {at:#x}: {what}");
                    }
                }
            }
            // 7b. ★ 2026-10-10 (`vblankgate`): the owed frame edges whose vblank's completions are all published —
            // the hardware's order: latch, notifier/semaphore/GET, THEN the vblank event and its interrupt. Every
            // guest-visible word above is written (the vidmem writes are synchronous) before the fence; the edge after.
            gate.remaining(queue.len());
            let due = gate.due(Instant::now());
            if !due.is_empty() {
                std::sync::atomic::fence(Ordering::SeqCst);
            }
            for h in due {
                if let Some(page) = shared_data {
                    vblank_counts[h] = vblank_counts[h].wrapping_add(1);
                    if let Err(e) = write_vblank_count(&io, page, h, vblank_counts[h]) {
                        io.refuse(&format!("display shared data: {e}"));
                        shared_data = None;
                    }
                }
                let irq =
                    frame_edge_out(dp, &store, h, edge_frame[h], loadv, wtrace, &vsync_traced);
                dp.counters.lat_edge.finish(pass_t0);
                raised |= irq != 0;
                if irq & dp.map.head_vblank != 0 {
                    counts[h].vblirq += 1;
                }
            }
            if gate.forced > forced_logged {
                if forced_logged < 20 {
                    eprintln!(
                        "kf3: display: a frame edge waited {} ms for its completions (a console copy behind the host) and was raised without them — {} so far",
                        crate::vblankgate::EDGE_CAP.as_millis(),
                        gate.forced
                    );
                }
                forced_logged = gate.forced;
            }
            if engine.updates > u64::from(logged_updates) && logged_updates < 64 {
                logged_updates = u32::try_from(engine.updates.min(64)).unwrap_or(64);
                eprintln!(
                    "kf3: display: {} updates completed, {} methods",
                    engine.updates, engine.methods
                );
            }
            dp.counters.updates.store(engine.updates, Ordering::Relaxed);
            dp.counters.methods.store(engine.methods, Ordering::Relaxed);
            // 8. the display interrupt — after the registers it announces. ★ H-loadv: the event
            // registers are also republished at every frame edge without an interrupt, so a read
            // shows what the frame latched (the hardware's `0x611800` is live)
            if raised || (loadv && ticked != 0) {
                dp.publish_events(&store);
                if raised && dp.ports.anything_pending(dp.map.heads as usize) {
                    dp.counters.irqs.fetch_add(1, Ordering::Relaxed);
                    self.latch_and_deliver(kf_rm::authored::DISP_STALL_VECTOR);
                }
            }
            dp.counters.lat_pass.finish(pass_t0);
            // 9. ★ §8.16: the achieved rates per path, per window of at least 1 s — published for
            // the status line (never waited for) and printed on a line of their own, at most every
            // 2 s and only when they changed
            for (h, c) in counts.iter_mut().enumerate() {
                let p = engine.pace.get(h).copied().unwrap_or_default();
                (c.presents, c.tearing) = (p.presents, p.tearing);
            }
            let o = Ordering::Relaxed;
            if let Some(w) = meter.sample(
                ns(Instant::now()),
                &counts,
                &pacer.periods(),
                dp.counters.scanouts.load(o),
                dp.counters.checks.load(o),
            ) {
                let mut st = kf_disp::pace::Status {
                    cfg_hz: dp.max_fps,
                    cap_hz: cap,
                    window: Some(w),
                    over: meter.over,
                    skipped: scan.mailbox.skipped,
                    same: dp.counters.same.load(o),
                    ondemand: dp.counters.ondemand.load(o),
                    lock: dp.broker.as_ref().map(|_| kf_disp::pace::LockStatus {
                        ticks_coalesced: gate_c.c.ticks_coalesced,
                        credits_coalesced: gate_c.c.credits_coalesced,
                        mailbox_coalesced: scan.mailbox.coalesced,
                        credits_deferred: gate_c.c.credits_deferred,
                        granted: gate_c.c.granted,
                        stale: gate_c.c.stale + gate_c.c.stale_credits,
                        probation: (0..MAX_HEADS)
                            .filter_map(|h| pacer.source(h))
                            .map(kf_disp::tick::TickSource::probation_events)
                            .sum(),
                    }),
                    ..kf_disp::pace::Status::default()
                };
                for (h, hs) in st.heads.iter_mut().enumerate() {
                    let p = engine.pace.get(h).copied().unwrap_or_default();
                    if let Some(slot) = pacer.slot(h) {
                        *hs = kf_disp::pace::HeadStatus {
                            period_ns: slot.period_ns,
                            raster_ns: slot.raster_ns,
                            late_max_us: pacer.late_max_ns(h) / 1000,
                            source: pacer.source(h).map(|t| t.source()),
                            handoffs: pacer.source(h).map_or((0, 0), |t| t.handoffs()),
                            core_imm: p.core_imm,
                        };
                    }
                }
                let line = st.fragment();
                if let Ok(mut g) = dp.fps.try_lock() {
                    g.clone_from(&line);
                }
                let due = fps_printed
                    .as_ref()
                    .is_none_or(|(t, last)| *last != line && t.elapsed() >= Duration::from_secs(2));
                if due {
                    eprintln!("kf3: display {line}");
                    fps_printed = Some((Instant::now(), line));
                }
            }
        }
        // ⊘ QEMU's console may still point at a frame after the worker stops (its main loop refreshes
        // until it ends): the frames and their context stay mapped until the process exits.
        std::mem::forget(scan);
        std::mem::forget(io.gpu.take());
    }

    /// ★ Display step 3c (the worker): author the monitor a resize asked for — clamped, fitted under
    /// the connector's pixel-clock limit at the same aspect ratio (`Monitor::for_window`) — put it
    /// behind connector 0 under the model's lock, and when it CHANGED and a hotplug registration
    /// is live, queue the hotplug for the drainer.
    fn apply_ui_request(&self, dp: &DisplayPlane, req: u64) {
        let (w, h, mhz) = (
            (req >> 48) as u32,
            ((req >> 32) & 0xFFFF) as u32,
            req as u32,
        );
        let changed = dp.model.lock().ok().and_then(|mut g| {
            let max = g
                .connectors
                .first()
                .map_or(165_000, |c| c.monitor.max_pixel_khz);
            // ★ §8.16: the preferred rate is the host's, under the cap; the cap never moves
            let m = kf_disp::edid::Monitor::for_window(w, h, mhz, max, dp.max_fps);
            let (mw, mh) = (m.preferred.h_active, m.preferred.v_active);
            let (rate, fnv) = (m.preferred.refresh_mhz(), m.edid_fnv().unwrap_or(0));
            let id = g.set_monitor(0, m)?;
            Some((id, mw, mh, rate, fnv, g.hotplug_target().is_some()))
        });
        match changed {
            Some((id, mw, mh, rate, fnv, true)) => {
                eprintln!(
                    "kf3: display: resize {w}x{h} at {mhz} mHz -> monitor {mw}x{mh} at {rate} mHz \
                     (EDID fnv1a64={fnv:016x}) on display {id:#x}; hotplug queued"
                );
                self.queue_hotplug(id);
            }
            Some((id, mw, mh, rate, fnv, false)) => eprintln!(
                "kf3: display: resize {w}x{h} at {mhz} mHz -> monitor {mw}x{mh} at {rate} mHz \
                 (EDID fnv1a64={fnv:016x}) on display {id:#x}; no hotplug registration (the next \
                 probe reads it)"
            ),
            None => {}
        }
    }

    /// Publish a channel's CHNCTL allocation bit and CHNSTATUS state: `Some(idle)` allocated, `None`
    /// freed.
    fn display_chan_status(&self, kind: ChannelKind, instance: u32, state: Option<bool>) {
        let Some(dp) = self.display else { return };
        let m = &dp.map;
        let k = kind_index(kind);
        let (cb, cs) = m.chnctl[k];
        let (sb, ss) = m.chnstatus[k];
        let ctl_off = cb + u64::from(instance) * cs;
        let st_off = sb + u64::from(instance) * ss;
        let ctl = self.shadow_word(ctl_off);
        let fld = m.chnstatus_state[k];
        match state {
            Some(idle) => {
                self.shadow_store(ctl_off, u64::from(ctl | m.chnctl_alloc), 4);
                let v = if idle {
                    m.chnstatus_idle[k]
                } else {
                    m.chnstatus_busy[k]
                };
                self.shadow_store(st_off, u64::from(kf_disp::class::put(0, fld, v)), 4);
            }
            None => {
                self.shadow_store(ctl_off, u64::from(ctl & !m.chnctl_alloc), 4);
                self.shadow_store(st_off, 0, 4);
            }
        }
    }
}

/// `(kind, instance)` of a channel number.
fn chan_of(chn: u32) -> Option<(ChannelKind, u32)> {
    match chn {
        0 => Some((ChannelKind::Core, 0)),
        1..=32 => Some((ChannelKind::Window, chn - 1)),
        33..=64 => Some((ChannelKind::WindowImm, chn - 33)),
        73..=80 => Some((ChannelKind::Cursor, chn - 73)),
        _ => None,
    }
}

fn engine_idle(engine: &Engine, dp: &DisplayPlane, chn: u32) -> bool {
    !engine.waiting(chn) && dp.ports.idle(chn)
}

/// ★ What the console shows — the lowest running head's composition (every enabled window it owns,
/// back to front), if any.
fn console_composition(engine: &Engine, dp: &DisplayPlane) -> Option<Composition> {
    let sv = dp.scan.as_ref()?;
    engine
        .heads_armed()
        .iter()
        .filter(|m| m.period_ns > 0)
        .find_map(|m| engine.composition(sv, m.head))
}

/// ★ One composed item of a copy, in back-to-front order ([`z_order`]): an RGB layer or a YUV window,
/// by index into the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Z {
    /// `planned.layers[i]`.
    Rgb(usize),
    /// `planned.yuv[j]`.
    Yuv(usize),
}

/// ★ The back-to-front order of a copy's RGB layers and YUV windows: deepest
/// `SET_COMPOSITION_CONTROL.DEPTH` first, equal depths in window order (the engine's own order for
/// [`Composition::layers`]). `depth` maps a window to its armed depth. Without a YUV window the
/// RGB layers keep the order they were planned in (a preserved composition is already ordered).
fn z_order(
    layers: &[LayerPlan],
    yuv: &[kf_disp::scanout::YuvPlan],
    depth: impl Fn(u32) -> u32,
) -> Vec<Z> {
    let mut v: Vec<(u32, u32, Z)> = layers
        .iter()
        .enumerate()
        .map(|(i, l)| (depth(l.window), l.window, Z::Rgb(i)))
        .chain(
            yuv.iter()
                .enumerate()
                .map(|(j, y)| (depth(y.window), y.window, Z::Yuv(j))),
        )
        .collect();
    if !yuv.is_empty() {
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    v.into_iter().map(|(_, _, z)| z).collect()
}

/// The kf-cuda mirror of a planned YUV window.
fn yuv_layer(l: &kf_disp::scanout::YuvPlan) -> kf_cuda::display::YuvLayer {
    kf_cuda::display::YuvLayer {
        y_src: l.y_src,
        y_extent: l.y_extent,
        c_src: l.c_src,
        c_extent: l.c_extent,
        block_linear: l.block_linear,
        y_pitch: l.y_pitch,
        c_pitch: l.c_pitch,
        block_height_log2: l.block_height_log2,
        sx0: l.sx0,
        sy0: l.sy0,
        sw: l.sw,
        sh: l.sh,
        ox: l.ox,
        oy: l.oy,
        dw: l.dw,
        dh: l.dh,
        sub_x_log2: l.sub_x_log2,
        sub_y_log2: l.sub_y_log2,
        vu_first: l.vu_first,
        layout: l.layout,
        filter: l.filter,
        fmt: l.fmt,
    }
}

/// The kf-cuda mirror of a planned layer.
fn compose_layer(l: &LayerPlan) -> ComposeLayer {
    ComposeLayer {
        src: l.src,
        extent: l.extent,
        block_linear: l.block_linear,
        pitch: l.pitch,
        block_height_log2: l.block_height_log2,
        x0_bytes: l.x0_bytes,
        y0: l.y0,
        width: l.width,
        rows: l.rows,
        ox: l.ox,
        oy: l.oy,
        flags: l.flags,
        a_s: l.a_s,
        b_s: l.b_s,
        a_d: l.a_d,
        b_d: l.b_d,
    }
}

/// An effect or a GET on the worker's completion queue.
enum Item {
    Effect(Effect),
    Get(u32, u32, u32),
}

/// A queued completion: `item`, held until scanout copy number `need` completed.
struct Queued {
    need: u64,
    item: Item,
}

/// How long a scanout copy may take before its stream is asked whether it failed (§8.18; it used
/// to be the time after which the display stopped for good).
const STUCK_COPY: Duration = Duration::from_secs(2);

/// ★ Review 2026-10-08 (b5c9f717..ac5d086f, finding 1): a copy whose stream says it FINISHED
/// (`Ok(true)`) but whose completion signal has not arrived this long past [`STUCK_COPY`] is a lost
/// signal on kayfabe's side, not a copy waiting behind host RM: the late-copy wait is bounded for
/// that case. (A copy still queued behind a busy host RM, `Ok(false)`, is waited for as before:
/// registering a big guest-RAM object takes seconds, and the queue below is bounded by
/// [`QUEUE_CAP`] instead.)
const SIGNAL_GRACE: Duration = Duration::from_secs(10);

/// ★ Review 2026-10-08, finding 1: the most completions (effects, GETs) held behind a copy that has
/// not completed. Past it the worker stops reading the guest's display pushbuffers, cursor posts
/// and vblank effects until the queue drains: the guest's PUT waits in the guest's own ring, so
/// nothing is dropped, nothing is forged as complete, no vCPU waits, and a lost completion cannot
/// make the VMM's memory grow with the guest's writes. Far above any normal depth (a few entries
/// per frame).
const QUEUE_CAP: usize = 4096;

/// ★ §8.18: what a copy still in flight after `elapsed` is, given its stream's state (`None`: no
/// display GPU; `Ok(false)` queued, `Ok(true)` done with its signal on the way, `Err` failed).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stuck {
    /// Not yet late.
    Running,
    /// Late, but the stream reports no failure: wait for the real signal.
    Waiting,
    /// The stream failed (or there is no GPU to ask): the completion will never come.
    Lost(String),
}

fn stuck_verdict(elapsed: Duration, state: Option<Result<bool, String>>) -> Stuck {
    if elapsed <= STUCK_COPY {
        return Stuck::Running;
    }
    match state {
        Some(Ok(true)) if elapsed > STUCK_COPY + SIGNAL_GRACE => Stuck::Lost(format!(
            "the stream finished the copy but its completion signal did not arrive within \
             {SIGNAL_GRACE:?} of the {STUCK_COPY:?} mark"
        )),
        Some(Ok(_)) => Stuck::Waiting,
        Some(Err(e)) => Stuck::Lost(format!("the display stream failed: {e}")),
        None => Stuck::Lost("no display GPU context to ask".into()),
    }
}

/// A frame the console shows up to 1080p fits here; a larger mode grows the slot once, to the max.
const FRAME_SMALL: usize = 1920 * 1080 * 4;
/// The largest frame ([`kf_disp::scanout::MAX_PIXELS`] at 4 bytes).
const FRAME_MAX: usize = kf_disp::scanout::MAX_PIXELS as usize * 4;

/// ★ §O, the worker's first cursor decision (GPU-free): whether this frame composes the head's
/// cursor — always without a cursor-capable broker (`want` is `None`: nothing was read) and under
/// grab; in hover only a cursor the host cannot show ([`CursorMode::composes`]).
/// ★ Measurement only: `KF3_DEBUG_CURSOR=1` logs every distinct (by content) cursor the host path or
/// the composer refuses — format, composition word, size, hot spot, pitch, the first 64 bytes in hex
/// and a text rendering of the image read as `A1R5G5B5` — and `KF3_DEBUG_CURSOR=/some/dir` also
/// writes the surface bytes there. Never required to boot; off it costs one atomic load.
fn debug_cursor_env() -> Option<&'static str> {
    static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        std::env::var("KF3_DEBUG_CURSOR")
            .ok()
            .filter(|v| !v.is_empty() && v != "0")
    })
    .as_deref()
}

/// Bookkeeping of the debug dump: the contents already dumped (at most [`CursorDebug::DISTINCT`]) and
/// when the surface was last copied (at most one copy per [`CursorDebug::EVERY_MS`]).
#[derive(Default)]
struct CursorDebug {
    seen: Vec<u64>,
    last: Option<Instant>,
}

impl CursorDebug {
    const DISTINCT: usize = 16;
    const EVERY_MS: u128 = 200;
}

/// ★ Whether this copy composes the head's cursor into the frame (the rule is `kf_broker::composes`:
/// a streaming broker draws the cursor itself in every mode, so never; otherwise the mode's).
fn cursor_composed(mode: CursorMode, stream: bool, want: Option<&CursorWant>) -> bool {
    want.is_none_or(|w| kf_broker::composes(mode, stream, w))
}

/// ★ Who a failed scanout copy is the failure of ([`ScanState::fault`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fault {
    /// The console cannot show it; nothing reached the GPU or the guest's display. The flips
    /// behind the copy complete.
    Console(String),
    /// Work sent to the GPU failed or was lost. The display stops; nothing is forged.
    Gpu(String),
}

/// ★ M2 — the worker's scanout copies (`V3_DISPLAY.md` §4.6). One copy in flight at a time, on the
/// plane's own stream; its completion is the `cuLaunchHostFunc` signal queued after it.
#[derive(Default)]
struct ScanState {
    /// A failed GPU/colour operation cannot release its queued successful completions.
    failed: bool,
    /// Copies started and completed; a flip of the console window makes every completion after it
    /// wait for copy `started + 1` — the first one that starts after the flip latched.
    started: u64,
    done: u64,
    /// The copy number items queued now wait for.
    barrier: u64,
    /// A FLIP-class copy is wanted (a change of what is shown, a modeset, a colour program) — made
    /// at once and always sent. ★ Copies without a flip are [`ScanState::nonflip`]'s (§8.16).
    want: bool,
    /// ★ §AK: the flips of the shown head. A latch marks a newer frame; one copy per tick copies the
    /// newest ([`kf_disp::pace::Mailbox`]); the flip itself completes at once, or — buffer-reuse
    /// rule — when the copy that was in flight at its latch is done ([`flip_completion_need`]).
    mailbox: Mailbox,
    /// The console showed a surface at the last pass (non-flip checks run only then).
    active: bool,
    /// The copy in flight.
    inflight: Option<Inflight>,
    /// Page-locked frames per slot — grown, never freed while the device lives ([`ConsoleShare`]).
    frames: [Option<Frame>; SLOTS],
    retired: Vec<Frame>,
    /// ★ Display step 3: why the broker's frame backing was refused (logged once); the console
    /// then keeps its own frames, the ring is withdrawn from the broker ([`broker_backing`]) so
    /// the broker is sent no frame again — never a CPU copy.
    broker_refused: Option<String>,
    /// ★ `display-max-fps` D2 (§8.16): copies made without a flip — at the console head's tick,
    /// while watched, sent only when the checksum changed; screendump requests.
    nonflip: NonFlip,
    /// The check clock of a picture with no armed head (the boot layer, a preserved scanout).
    idle_at: Option<Instant>,
    serial: u64,
    /// Which scanout refusals were already logged: one line per distinct reason ([`ReasonLog`]).
    refusal_log: ReasonLog,
    /// The last copy number logged as late but not lost ([`ScanState::give_up_if_stuck`]).
    slow_logged: u64,
    /// `KF3_DISPLAY_TRACE`: each copy's source, and a digest of what it copied.
    trace: bool,
    /// The compose kernel's bring-up self-test verdict (the console shows nothing on its failure).
    bl_ok: Option<Result<(), String>>,
    /// ★ B5: the planned layers of the last ARMED composition copied whole (no window refused) —
    /// what a `PRESERVE_HW` free keeps on the monitor ([`Shown::Preserved`]).
    /// ⊘ CORRECTED 2026-10-04 (B5 on vmb): a window whose context DMA the guest unbound while it
    /// stayed armed was refused by the next refresh copy, and that copy set this to `None` — so the
    /// preserving free that follows the unbind in NVKMS's teardown kept nothing. Windows now
    /// resolve through [`LatchedDmas`]: such a copy is whole and this keeps the console.
    last_plan: Option<(Vec<LayerPlan>, (u32, u32))>,
    /// ★ 2026-10-04: each window's context DMA as its ARMED state resolved it ([`LatchedDmas`]).
    latched: LatchedDmas,
    colors: ColorDmas,
    /// ★ The GPU-copy rung's worker half (§8.11).
    vram: Option<VramWorker>,
    /// ★ §O: the cursor mode the last pass saw (a change recomposes) …
    cursor_mode: CursorMode,
    /// … whether the last copy composed the head's cursor (only then does a move recompose) …
    cursor_in_frame: bool,
    /// … the key of the cursor last posted to the relay (posted again only when it changes) …
    cursor_posted: Option<(u8, u64)>,
    /// … host-cursor refusals logged so far (bounded) …
    cursor_refusals: u64,
    /// … the cursor composition word's line (once per change, §8.12's open alpha question;
    /// bounded, `kf_disp::scanout::CompositionLog`) …
    cursor_comp: kf_disp::scanout::CompositionLog,
    /// … the `KF3_DEBUG_CURSOR` dump's bookkeeping (measurement only) …
    cursor_debug: CursorDebug,
    /// … and the hot spot NVKMS does not program, derived from the injected pointer.
    hot: HotTracker,
    /// The worker's clock for [`HotTracker`] (its first use).
    hot_epoch: Option<Instant>,
}

/// ★ §8.16: what the copy in flight is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// A non-flip CHECK: the frame composed and checksummed, nothing sent yet — its completion
    /// decides whether a [`Phase::Send`] of the same composition follows.
    Check,
    /// The pack and/or D2H of the composed frame, then its publication.
    Send,
}

/// ★ The copy in flight: its number, phase, slot, frame size and start — and which copies it made:
/// the pack into the slot's VRAM backing (at this block-linear shape) and/or the D2H into its host
/// frame. Only those backings are published as holding the frame.
#[derive(Debug, Clone, Copy)]
struct Inflight {
    color: bool,
    n: u64,
    phase: Phase,
    slot: usize,
    wh: (u32, u32),
    t0: Instant,
    vram: Option<kf_broker::VramGeom>,
    d2h: bool,
    /// ★ §8.13: it composes the guest's cursor (the console's cursor follows the frame it shows).
    cursor: bool,
    /// ★ §8.16: the refresh request it serves when it completes (it started after it).
    req: u64,
    /// A checksum was queued with the composition (the row sums are read at completion).
    summed: bool,
    /// The boot layer was composed (counted when it is sent).
    boot: bool,
}

/// What a composed frame's sends take: the copies the plan chose, the slot and its pack shape.
struct SendPlan {
    plan: kf_broker::gpucopy::Plan,
    eligible: u32,
    geom: Option<kf_disp::vramslot::SlotGeom>,
}

/// ★ Run the compose kernel on SYNTHETIC surfaces — a block-linear window composed opaque, a
/// premultiplied-alpha pixel blended, and (§O, 2026-10-04) an XOR pixel — and compare with the
/// reference address function (`kf_disp::scanout::bl_offset`) and the blend arithmetic
/// (`kf_disp::scanout::compose_reference`), at bring-up, before any guest copy.
fn selftest_compose(gpu: &mut DisplayGpu) -> Result<(), String> {
    // 4 GOBs wide, 2-GOB blocks, 3 block rows; a rectangle offset in both axes, landing at (5, 2)
    let (gpr, bh, block_rows) = (4u32, 1u32, 3u64);
    let extent = block_rows * u64::from(gpr) * (512 << bh);
    let surface: Vec<u8> = (0..extent)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    let (x0_bytes, y0, width, rows, ox, oy, fw, fh) = (4u32, 3u32, 62u32, 37u32, 5, 2, 70, 40);
    let l = ComposeLayer {
        src: 0,
        extent,
        block_linear: true,
        pitch: gpr,
        block_height_log2: bh,
        x0_bytes,
        y0,
        width,
        rows,
        ox,
        oy,
        flags: 4,
        a_s: 255,
        b_s: 0,
        a_d: 0,
        b_d: 0,
    };
    let got = gpu
        .selftest_compose(&surface, &l, fw, fh)
        .map_err(|e| format!("did not run: {e}"))?;
    for y in 0..fh {
        for x in 0..fw * 4 {
            let inside = (ox * 4..(ox + width) * 4).contains(&x) && (oy..oy + rows).contains(&y);
            let want = if inside {
                let at = kf_disp::scanout::bl_offset(
                    u64::from(x0_bytes + x - ox * 4),
                    u64::from(y0 + y - oy),
                    u64::from(gpr),
                    bh,
                );
                surface[at as usize]
            } else {
                0 // the staging frame is cleared to black
            };
            let have = got[(y * fw * 4 + x) as usize];
            if want != have {
                return Err(format!(
                    "frame ({}, {y}) byte {}: the kernel wrote {have:#04x}, the reference says {want:#04x}",
                    x / 4,
                    x % 4
                ));
            }
        }
    }
    // a premultiplied ARGB pixel (a = 0x80) over black: out = src*1 + 0*(1 - a)
    let px = [0x40u8, 0x20, 0x10, 0x80]; // B G R A
    let one = ComposeLayer {
        src: 0,
        extent: 4,
        block_linear: false,
        pitch: 4,
        block_height_log2: 0,
        x0_bytes: 0,
        y0: 0,
        width: 1,
        rows: 1,
        ox: 0,
        oy: 0,
        flags: 1,
        a_s: 255,
        b_s: 0,
        a_d: 255,
        b_d: -255,
    };
    let got = gpu
        .selftest_compose(&px, &one, 1, 1)
        .map_err(|e| format!("did not run (blend): {e}"))?;
    if got[..3] != px[..3] {
        return Err(format!(
            "the blend wrote {:?}, not {:?}",
            &got[..3],
            &px[..3]
        ));
    }
    // ★ §O (2026-10-04): the XOR blend over the black frame, from an alpha-0 pixel with factors
    // that would make a blend write black: XOR leaves exactly the colour (and the X byte 0), so a
    // kernel without the XOR path, or one that lets alpha gate it, fails here before any guest
    // cursor is composed (`kf_disp::scanout::compose_reference` is the arithmetic)
    let xpx = [0x5a, 0xc3, 0x3c, 0x00]; // B G R A
    let xor = ComposeLayer {
        flags: kf_disp::scanout::COMPOSE_XOR,
        a_s: 0,
        b_s: 0,
        a_d: 255,
        b_d: 0,
        ..one
    };
    let got = gpu
        .selftest_compose(&xpx, &xor, 1, 1)
        .map_err(|e| format!("did not run (XOR): {e}"))?;
    if got[..4] != xpx {
        return Err(format!("the XOR blend wrote {:?}, not {xpx:?}", &got[..4]));
    }
    Ok(())
}

// ★ §O: the compose kernel's flag bits as kf-disp plans them and as kf-cuda's launch check knows
// them — one set (a drift is a layer refused, or worse, composed as a blend)
const _: () = assert!(kf_disp::scanout::COMPOSE_FLAGS == kf_cuda::display::COMPOSE_FLAGS);

/// ★ Display step 3 — the worker's choice of memory for a FREE slot it must (re)allocate, kept
/// GPU-free so a test drives the refusal (the second review of `v3-broker`, 2026-10-03: the
/// first fix's call site was untested). `seat` (the broker on) makes a broker-visible frame
/// (`BrokerSeat::frame`); it is not asked once a refusal is recorded. `None` = the console's own
/// memory (`DisplayGpu::frame`).
///
/// ⊘ CORRECTED 2026-10-03: at the first refusal this WITHDRAWS THE HOST KIND from the broker
/// ([`kf_broker::FrameRing::withdraw`]`(Kind::Host)`) — every slot, reallocated or not. The first
/// fix withdrew only the slot being reallocated, so a slot that kept its broker memfd went on
/// feeding the broker while the line below said it would be shown nothing. ⊘ And the same day
/// (§8.11) it no longer withdraws the WHOLE ring (`withdraw_all`): a refused memfd registration
/// says nothing about the VRAM slots, so the GPU-copy rung goes on.
fn broker_backing<F>(
    ring: &kf_broker::FrameRing,
    refused: &mut Option<String>,
    seat: Option<impl FnOnce() -> Result<F, String>>,
) -> Option<F> {
    let seat = seat.filter(|_| refused.is_none())?;
    match seat() {
        Ok(f) => Some(f),
        Err(e) => {
            // before the slot is refilled with memory the broker cannot receive
            ring.withdraw(kf_broker::Kind::Host);
            eprintln!(
                "kf3: display: the BROKER host-memory frame backing is REFUSED ({e}) — the console \
                 keeps working; the host-memory rungs are withdrawn from every frame slot and the \
                 broker is sent no frame through host memory from now on (the GPU-copy rung, if \
                 offered, is unaffected)"
            );
            *refused = Some(e);
            None
        }
    }
}

/// FNV-1a over a frame's visible pixels as R,G,B bytes — the digest `kfdisp_probe` prints for its
/// patterns (`KFDISP_PATTERN_A_FNV`), so a trace line says WHICH surface the copy read.
fn fnv_rgb_xrgb8888(bytes: &[u8], stride: usize, width: usize, height: usize) -> u64 {
    let mut h: u64 = 1_469_598_103_934_665_603;
    for y in 0..height {
        let Some(row) = bytes.get(y * stride..y * stride + width * 4) else {
            return 0;
        };
        for px in row.as_chunks::<4>().0 {
            for b in [px[2], px[1], px[0]] {
                h ^= u64::from(b);
                h = h.wrapping_mul(1_099_511_628_211);
            }
        }
    }
    h
}

impl ScanState {
    /// ★ A guest life ends: forget what the old life's windows resolved to and what was composed,
    /// and want a copy at once (the next pass shows what the new life shows: the boot layer, or
    /// black). ⊘ Only with no copy in flight (the caller waits for its completion): its frame slot
    /// and the completions waiting on it are the old life's. Kept: the frames (process-lifetime
    /// page-locked memory), the GPU-copy rung, the broker refusal, the self-test verdict, a failed
    /// GPU context (`failed` describes the device's own context, not the guest), the counters,
    /// and the monotonic colour token counter (the GPU LUT snapshot cache also survives).
    fn reset_life(&mut self) {
        debug_assert!(self.inflight.is_none());
        self.barrier = self.started;
        self.done = self.started;
        self.mailbox.reset_life();
        self.want = true;
        self.active = false;
        self.idle_at = None;
        self.last_plan = None;
        self.latched = LatchedDmas::default();
        self.colors.reset_life();
        self.cursor_mode = CursorMode::default();
        self.cursor_in_frame = false;
        self.cursor_posted = None;
        self.hot = HotTracker::default();
        self.hot_epoch = None;
    }

    /// ★ §8.16: the check clock of a picture with no armed head — due every `period` while `run`
    /// (late ticks rescheduled as the pacer does), stopped otherwise.
    fn idle_tick(&mut self, now: Instant, period: Duration, run: bool) -> bool {
        if !run {
            self.idle_at = None;
            return false;
        }
        match self.idle_at {
            None => {
                self.idle_at = Some(now + period);
                false
            }
            Some(t) if now >= t => {
                self.idle_at = Some(if now.duration_since(t) > period {
                    now + period
                } else {
                    t + period
                });
                true
            }
            Some(_) => false,
        }
    }

    /// ★ §AK: copy `started + 1` begins. A FLIP-class copy (`check` false) reads the newest state, so it serves the
    /// wanted copy, the pending latched frame and the tick's credit; a non-flip check serves none of them.
    fn begin_copy(&mut self, check: bool) -> u64 {
        if !check {
            self.want = false;
            self.mailbox.copied();
        }
        self.started += 1;
        self.started
    }

    /// ★ §AK: the shown head ticked (`armed`: what the console shows is a guest scanout) — one flip copy is allowed.
    fn on_console_tick(&mut self, armed: bool) {
        if armed {
            self.mailbox.tick();
        }
    }

    /// ★ §AK: a flip of the shown head latched. A newer frame exists; the flip's completion items
    /// (queued next, with [`ScanState::barrier`]) wait for the copy in flight now — the one that read
    /// the buffer this flip replaces — and for nothing else
    /// ([`kf_disp::pace::flip_completion_need`], the buffer-reuse rule).
    fn on_flip_latched(&mut self) {
        self.mailbox.latch();
        self.barrier = flip_completion_need(self.barrier, self.inflight.as_ref().map(|f| f.n));
    }

    /// ★ review hitch-20261017: **everything the worker does when a window's UPDATE latches**, in one place the property
    /// tests can drive. The window's armed state changed, so its context-DMA and colour bindings are forgotten (the next
    /// copy resolves them afresh); when the window is one the console shows, the flip completes behind the copy in flight
    /// at this latch and the newest frame becomes a pending mailbox copy ([`ScanState::on_flip_latched`] — the only place
    /// that sets a barrier for a window flip, with or without the colour path), and `resolve` then runs the resolutions
    /// the worker passes in (context DMA, LUT/TMO bindings) while the guest has not yet been told the flip completed.
    fn on_window_latch(&mut self, window: u32, shown: bool, resolve: impl FnOnce(&mut ScanState)) {
        self.latched.forget(window);
        self.colors.forget_window(window as usize);
        if shown {
            self.on_flip_latched();
            resolve(self);
        }
    }

    /// ★ §AK: window `so` just latched — resolve its context DMA now (the first copy after a latch
    /// resolves it afresh; with the copy up to a tick away, a guest that unbinds the context DMA
    /// before the copy must still find it resolved). A refusal is never kept ([`LatchedDmas`]): the
    /// copy tries again.
    fn resolve_now(&mut self, io: &mut Io<'_>, so: &kf_disp::engine::Scanout) {
        let _ = self
            .latched
            .resolve(so, || io.resolve(so.client, so.handle, so.chn));
    }

    /// Is a FLIP-class copy waiting — a change of what is shown, a barrier the next copy serves, or a
    /// latched frame the mailbox has not copied? It publishes the newer frame anyway, so a non-flip
    /// check yields to it.
    fn flip_pending(&self) -> bool {
        self.want || self.barrier > self.started || self.mailbox.pending()
    }

    /// ★ Copy `n` is over (published, found unchanged, refused, or given up): the completions
    /// behind it go, and the refresh requests it started after are served.
    fn finish(&mut self, dp: &DisplayPlane, n: u64, req: u64) {
        self.finish_with(&dp.counters, &dp.console, n, req);
    }

    fn finish_with(&mut self, c: &DispCounters, console: &ConsoleShare, n: u64, req: u64) {
        self.done = self.done.max(n);
        if self.nonflip.serve(req) {
            c.ondemand.fetch_add(1, Ordering::Relaxed);
            console.serve_refresh(req);
        }
    }

    /// ★ Nothing can be copied now (nothing shown, no GPU): every waiting request is served by the
    /// frame the console already has.
    fn serve_now(&mut self, dp: &DisplayPlane) {
        self.serve_now_with(&dp.counters, &dp.console);
    }

    fn serve_now_with(&mut self, c: &DispCounters, console: &ConsoleShare) {
        let req = self.nonflip.snapshot();
        if self.nonflip.serve(req) {
            c.ondemand.fetch_add(1, Ordering::Relaxed);
            console.serve_refresh(req);
        }
    }

    /// ★ A copy `n` that cannot be made. The two kinds are NOT alike:
    ///
    /// * [`Fault::Console`]: the console (the host's viewer, not the guest's scanout) cannot
    ///   show something: a colour program it cannot build. The display engine here is emulated
    ///   and no GPU work stands behind the guest's flip, so the flips behind this copy COMPLETE
    ///   ([`ScanState::finish`]) with the console keeping its last good frame; the refusal is
    ///   loud (named counter, the first lines in the log). Never sets `failed`: a console-only
    ///   refusal must not halt the guest's display channels (run 296: it did, and the guest's
    ///   flips stopped until the TDR).
    /// * [`Fault::Gpu`]: work sent to the GPU failed or was lost: no synthetic success, `failed`
    ///   stops the display (`engine.halt_scanout`).
    fn fault(&mut self, c: &DispCounters, console: &ConsoleShare, n: u64, req: u64, f: Fault) {
        match f {
            Fault::Console(why) => {
                c.console_copies_skipped.fetch_add(1, Ordering::Relaxed);
                self.note_refusal(c, &why);
                self.finish_with(c, console, n, req);
            }
            Fault::Gpu(why) => {
                self.note_refusal(c, &why);
                self.failed = true;
                self.serve_now_with(c, console);
            }
        }
    }

    /// The composed frame's checksum, when one was queued with it and its rows came back.
    fn digest_of(io: &Io<'_>, f: &Inflight) -> Option<u64> {
        let (w, h) = f.wh;
        f.summed
            .then(|| io.gpu.as_ref()?.checksum_rows(h).ok())
            .flatten()
            .map(|r| kf_disp::pace::digest(&r, w, h))
    }

    /// The copy in flight completed. ★ §8.16: a CHECK decides here — a flip copy waiting publishes
    /// the newer frame anyway; otherwise the frame is sent when [`NonFlip::wants_send`] says so,
    /// from the composition still in the staging frame. A SEND is published to the console.
    fn completed(&mut self, io: &mut Io<'_>) {
        let Some(f) = self.inflight.take() else {
            return;
        };
        let dp = io.dp;
        if f.color {
            let verdict = io
                .gpu
                .as_ref()
                .ok_or_else(|| "colour GPU disappeared".to_owned())
                .and_then(DisplayGpu::color_verdict);
            if let Err(e) = verdict {
                self.refuse(dp, &e);
                self.failed = true;
                self.serve_now(dp);
                return;
            }
        }
        let digest = ScanState::digest_of(io, &f);
        if f.phase == Phase::Check {
            if self.flip_pending() {
                self.finish(dp, f.n, f.req);
                return;
            }
            let p = self.plan_send(io, f.wh);
            let now = Sent {
                digest: digest.unwrap_or_default(),
                wh: f.wh,
                cursor: f.cursor,
                host: p.plan.d2h,
                vram: p.plan.pack,
            };
            if digest.is_some() && !self.nonflip.wants_send(&now) {
                dp.counters.same.fetch_add(1, Ordering::Relaxed);
                self.finish(dp, f.n, f.req);
                return;
            }
            self.send(io, &f, &p);
            return;
        }
        let Inflight {
            n,
            slot,
            wh: (w, h),
            t0,
            vram,
            d2h,
            cursor,
            req,
            ..
        } = f;
        let us = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
        dp.counters
            .scanout_us_total
            .fetch_add(us, Ordering::Relaxed);
        dp.counters.scanout_us_max.fetch_max(us, Ordering::Relaxed);
        // the host frame is read (and published) only when the D2H copy filled it
        let host = self.frames[slot].as_ref().filter(|_| d2h);
        if host.is_none() && vram.is_none() {
            self.finish(dp, n, req);
            return;
        }
        if let Some(fr) = host
            && self.trace
            && (n <= 8 || n.is_multiple_of(50))
        {
            let (wu, hu, st) = (w as usize, h as usize, w as usize * 4);
            let fnv = fnv_rgb_xrgb8888(&fr.read(0, st * hu), st, wu, hu);
            eprintln!("kf3: display: TRACE scanout copy {n} done: {w}x{h} fnv={fnv:016x}");
        }
        self.serial += 1;
        dp.console.publish(
            slot,
            FrameView {
                addr: host.map_or(0, Frame::addr),
                width: w,
                height: h,
                stride: w * 4,
                format: kf_disp::scanout::PixelFormat::Xrgb8888 as u32,
                serial: self.serial,
                cursor,
            },
            host.is_some(),
            vram,
        );
        dp.counters.scanouts.fetch_add(1, Ordering::Relaxed);
        if let Some(d) = digest {
            self.nonflip.sent(Sent {
                digest: d,
                wh: (w, h),
                cursor,
                host: host.is_some(),
                vram: vram.is_some(),
            });
        }
        // ★ the relay (main loop) learns of it through one non-blocking eventfd write
        if let Some(b) = &dp.broker {
            b.frame_published();
        }
        self.finish(dp, n, req);
    }

    /// The pack of a `w`x`h` frame into `slot`'s VRAM backing: its CUDA slot, the launch's shape and
    /// what the ring publishes (`None` when the slot has no imported VRAM or the frame no shape).
    fn pack_for(
        &self,
        slot: usize,
        geom: Option<kf_disp::vramslot::SlotGeom>,
        w: u32,
        h: u32,
    ) -> Option<(
        kf_cuda::display::SlotId,
        kf_cuda::display::BlPack,
        kf_broker::VramGeom,
    )> {
        let g = geom?;
        let (sid, _) = (*self.vram.as_ref()?.slots.get(slot)?)?;
        Some((
            sid,
            kf_cuda::display::BlPack {
                width: w,
                height: h,
                gobs_per_row: g.gobs_per_row,
                h_log2: g.h_log2,
                gobs: g.gobs(),
                bits: kf_disp::vramslot::GOB_GA106.bits(),
            },
            kf_broker::VramGeom {
                stride: g.stride,
                extent: g.extent,
            },
        ))
    }

    /// A lost GPU completion stops the display. Neither the slot nor successful guest
    /// completion is published; a timeout is not evidence that GPU work completed.
    ///
    /// ⊘ CORRECTED 2026-10-08 (`V3_DISPLAY.md` §8.18, runs e1-a…d, kf3 `0e64a960`, host
    /// 595.91.07): nor is a timeout evidence that the completion was LOST. The first copy, asked
    /// for within ~0.3 s of QEMU starting (a broker already listening, a console readback), sat
    /// behind the VA manager's prewarm — host RM registering the 8 GiB guest-RAM memfd took 3.53 s
    /// (`ram_obj 3528088 us`; 0.88 s at 2 GiB, where no copy was given up) — and was given up at
    /// 2 s with `failed` set for the VM's life and its barrier never served, so the display and the
    /// guest's core channel stayed dead. A copy past [`STUCK_COPY`] is now given up only when its
    /// stream REPORTS a failure ([`stuck_verdict`]); still queued, it is waited for, with one line
    /// naming the wait.
    fn give_up_if_stuck(&mut self, io: &Io<'_>) {
        let dp = io.dp;
        let Some(Inflight { n, t0: t, .. }) = self.inflight else {
            return;
        };
        let state = io
            .gpu
            .as_ref()
            .map(|g| g.signal_state().map_err(|e| e.to_string()));
        match stuck_verdict(t.elapsed(), state) {
            Stuck::Running => {}
            Stuck::Waiting => {
                if self.slow_logged < n {
                    self.slow_logged = n;
                    eprintln!(
                        "kf3: display: copy {n} has not completed in {STUCK_COPY:?} and its stream \
                         reports no failure (the host RM or the GPU has not run it yet): waiting for \
                         its signal, nothing is forged meanwhile"
                    );
                }
            }
            Stuck::Lost(why) => {
                self.inflight = None;
                self.failed = true;
                self.serve_now(dp);
                let line =
                    format!("copy {n} did not complete in {STUCK_COPY:?} and is lost: {why}");
                self.refuse(dp, &line);
            }
        }
    }

    /// ★ Measurement only ([`debug_cursor_env`]): copy the start of the refused cursor's surface
    /// (the GPU copies; at most 64 KiB, wholly inside the context DMA, at most every 200 ms) and log
    /// it once per distinct content, at most 16 contents.
    fn debug_cursor(&mut self, io: &mut Io<'_>, cs: &kf_disp::engine::CursorScan, who: &str) {
        let Some(env) = debug_cursor_env() else {
            return;
        };
        let d = &mut self.cursor_debug;
        if d.seen.len() >= CursorDebug::DISTINCT
            || d.last
                .is_some_and(|t| t.elapsed().as_millis() < CursorDebug::EVERY_MS)
        {
            return;
        }
        d.last = Some(Instant::now());
        let Ok(dma) = io.resolve(cs.client, cs.handle, 0) else {
            return;
        };
        let (Some((src, extent)), Some(gpu)) = (
            kf_disp::scanout::cursor_debug_span(cs, &dma),
            io.gpu.as_ref(),
        ) else {
            return;
        };
        let mut raw = vec![0u8; usize::try_from(extent).unwrap_or(0)];
        if gpu.read_store(src, &mut raw).is_err() {
            return;
        }
        let key = kf_disp::scanout::fnv1a(&raw) ^ u64::from(cs.format) ^ (u64::from(cs.mode) << 40);
        if self.cursor_debug.seen.contains(&key) {
            return;
        }
        self.cursor_debug.seen.push(key);
        for line in kf_disp::scanout::cursor_debug_text(cs, &raw, true) {
            eprintln!("kf3: display: DEBUG cursor ({who}) {line}");
        }
        if env.starts_with('/') {
            let f = format!("{env}/cursor-{key:016x}.bin");
            if let Err(e) = std::fs::write(&f, &raw) {
                eprintln!("kf3: display: DEBUG cursor: writing {f}: {e}");
            }
        }
    }

    fn refuse(&mut self, dp: &DisplayPlane, why: &str) {
        self.note_refusal(&dp.counters, why);
    }

    fn note_refusal(&mut self, c: &DispCounters, why: &str) {
        c.scanout_refused.fetch_add(1, Ordering::Relaxed);
        match self.refusal_log.admit(why) {
            Admit::Seen => {}
            Admit::New => eprintln!(
                "kf3: display: scanout REFUSED {why} (logged once per distinct reason; repeats are counted in scanout_refused)"
            ),
            Admit::Full => eprintln!(
                "kf3: display: scanout refusals: {} distinct reasons logged; further new ones are only counted",
                ReasonLog::MAX
            ),
        }
    }

    /// ★ §O: what the guest shows as its cursor NOW, for the host — `Hidden` without an armed
    /// composition or an enabled cursor, or for a wholly transparent image; the image (copied by the
    /// GPU from the store into a buffer kf owns, converted to premultiplied ARGB) when the host can
    /// show it; `Composed` — refused by name, at a bounded rate — when it cannot (XOR, an additive
    /// blend, a surface that does not resolve or could not be copied).
    fn host_cursor_want(
        &mut self,
        io: &mut Io<'_>,
        shown: &Shown,
        cursor: Option<&kf_disp::engine::CursorScan>,
    ) -> CursorWant {
        let dp = io.dp;
        // ★ the active broker advertised CAP_CURSOR_STREAM: it draws the cursor itself in every
        // mode, an XOR cursor goes as its inversion mask (nothing is composed). The decisions are
        // `kf_broker::want_of` / `kf_disp::scanout::plan_host_cursor_xor`; this reads the GPU.
        let stream = dp.broker.as_ref().is_some_and(|b| b.cursor().stream());
        let (Shown::Armed(comp), Some(cs)) = (shown, cursor) else {
            // no armed head or no enabled cursor plane: the guest draws its own, or none
            return kf_broker::want_of(
                kf_broker::CursorRead::PlaneOff,
                stream,
                &mut self.hot,
                0,
                0,
                None,
            )
            .unwrap_or(CursorWant::Hidden);
        };
        let frame = (comp.width, comp.height);
        let hover = self.cursor_mode == CursorMode::Hover;
        let (seq, abs) = dp.broker.as_ref().map_or((0, None), |b| b.cursor().abs());
        let now_ms = u64::try_from(
            self.hot_epoch
                .get_or_insert_with(Instant::now)
                .elapsed()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        let hot = &mut self.hot;
        let comp_log = &mut self.cursor_comp;
        let got = io
            .resolve(cs.client, cs.handle, 0)
            .and_then(|dma| kf_disp::scanout::plan_host_cursor_xor(cs, &dma, stream).map_err(|r| r.0))
            .and_then(|h| {
                let gpu = io.gpu.as_ref().ok_or("no display GPU context")?;
                let n = usize::try_from(h.extent).map_err(|_| "an impossible cursor extent")?;
                let mut raw = vec![0u8; n];
                gpu.read_store(h.src, &mut raw)
                    .map_err(|e| format!("copying the cursor image: {e}"))?;
                dp.counters
                    .host_cursor_reads
                    .fetch_add(1, Ordering::Relaxed);
                // ★ §8.12's open question (premultiplied pixels under a straight blend?): the
                // composition word the guest programmed, once per change, beside what the pixels
                // say about their own alpha — the next box run reads the answer off this line
                let (word, mode) = kf_disp::scanout::cursor_composition(cs);
                if let Some(n) = comp_log.changed(word) {
                    let c = h.alpha_census(&raw).unwrap_or_default();
                    eprintln!(
                        "kf3: display: guest cursor composition {word:#07x} = {mode} (K1 {}, cursor \
                         factor {}, viewport factor {}, mode {}); its {}x{} pixels: {} partially \
                         transparent, {} with a colour channel above alpha ({}) — change {n} (the \
                         first {} are logged, then every {}th)",
                        cs.k1,
                        cs.cursor_factor,
                        cs.viewport_factor,
                        cs.mode,
                        h.size,
                        h.size,
                        c.partial,
                        c.above_alpha,
                        if c.above_alpha > 0 {
                            "straight pixels"
                        } else if c.partial > 0 {
                            "consistent with premultiplied pixels"
                        } else {
                            "no partial alpha to tell"
                        },
                        kf_disp::scanout::CompositionLog::LINES,
                        kf_disp::scanout::CompositionLog::EVERY
                    );
                }
                // ★ NVKMS programs hot spot 0 (`nvkms-evo3.c:6565-6569`): in hover the hot
                // spot is derived from the pointer the relay injected
                let derived = abs.filter(|_| hover && !stream).and_then(|a| {
                    kf_disp::scanout::hot_from_pointer(cs, (a.x, a.y), (a.w, a.h), frame)
                });
                let read = match h.image(&raw).map_err(|r| r.0)? {
                    None => kf_broker::CursorRead::Empty,
                    Some(pixels) => kf_broker::CursorRead::Image {
                        size: h.size,
                        hot: h.hot,
                        pixels,
                        xor: h.is_xor(),
                    },
                };
                kf_broker::want_of(read, stream, hot, seq, now_ms, derived)
            });
        match got {
            Ok(w) => w,
            Err(e) => {
                dp.counters
                    .host_cursor_refused
                    .fetch_add(1, Ordering::Relaxed);
                self.cursor_refusals += 1;
                let n = self.cursor_refusals;
                self.debug_cursor(io, cs, "host");
                if n <= 4 || n.is_multiple_of(256) {
                    // an XOR cursor is composed BY DESIGN (§O: no ARGB "over" inverts): not a refusal
                    let verb = if e.contains("XOR cursor has no ARGB") {
                        "host cursor COMPOSED"
                    } else {
                        "host cursor REFUSED"
                    };
                    eprintln!(
                        "kf3: display: {verb}: {e} — composed into the frame instead, \
                         and the host's is hidden ({n} so far)"
                    );
                }
                // composed into the frame — or, under the stream, hidden (nothing composes)
                kf_broker::want_of(
                    kf_broker::CursorRead::Unshowable,
                    stream,
                    &mut self.hot,
                    0,
                    0,
                    None,
                )
                .unwrap_or(CursorWant::Hidden)
            }
        }
    }

    /// ★ Start the next copy of what the console shows: every enabled window of the head composed,
    /// back to front, into the device staging frame, then copied into a free console frame. A copy
    /// that cannot be made (nothing shown, no kernel) completes at once — the flip it follows still
    /// completes (the engine latched it); only the console keeps its previous frame. A window that
    /// cannot be composed is refused by name and left out. ★ The boot layer (`gop=on`) is one
    /// VMM-authored layer: no context DMA to resolve and nothing to plan; the head's cursor
    /// (display step 3d) is composed on an ARMED composition only.
    ///
    /// ★ §8.16: the composition is checksummed on the GPU too. A FLIP-class copy (`check` false)
    /// sends at once; a non-flip `check` signals after the checksum and [`ScanState::completed`]
    /// decides whether the composition is sent.
    fn start(
        &mut self,
        io: &mut Io<'_>,
        engine: &Engine,
        shown: Option<&Shown>,
        cursor: Option<&kf_disp::engine::CursorScan>,
        check: bool,
    ) {
        if self.failed {
            self.serve_now(io.dp);
            return;
        }
        let n = self.begin_copy(check);
        let req = self.nonflip.snapshot();
        let t0 = Instant::now();
        let dp = io.dp;
        if check {
            dp.counters.checks.fetch_add(1, Ordering::Relaxed);
        }
        let Some(shown) = shown else {
            self.finish(dp, n, req);
            return;
        };
        // A preserving free retains the already transformed console frame; it must not
        // read a freed LUT/handle or compose the old windows without their colour program.
        if dp.sdr_color.is_some() && matches!(shown, Shown::Preserved(..)) {
            self.finish(dp, n, req);
            return;
        }
        if let Some(Err(e)) = &self.bl_ok {
            let e = format!("the compose kernel failed its self-test ({e})");
            self.refuse(dp, &e);
            self.finish(dp, n, req);
            return;
        }
        let (w, h) = match shown {
            Shown::Armed(comp) => (comp.width, comp.height),
            Shown::Boot(_, size) | Shown::Preserved(_, size) | Shown::Blank(size) => *size,
        };
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > kf_disp::scanout::MAX_PIXELS {
            self.refuse(dp, &format!("a {w}x{h} composition"));
            self.finish(dp, n, req);
            return;
        }
        // plan every window (each bounded by its own context DMA) before the GPU sees one
        let planned = self.plan(shown, &dp.formats, (w, h), n, |so| {
            io.resolve(so.client, so.handle, so.chn)
        });
        for e in &planned.refused {
            self.refuse(dp, e);
            dp.counters
                .console_windows_left_out
                .fetch_add(1, Ordering::Relaxed);
        }
        let color = if let (Some((t, win, core)), Shown::Armed(comp)) = (dp.sdr_color, shown) {
            let program = (|| -> Result<_, String> {
                // ★ a window the console cannot compose (planned.refused, already counted and named)
                // is simply not among `planned.layers`: the colour frame is composed without it.
                // It used to fail the whole copy, and `failed` halted the guest's display.
                let mut inputs = Vec::new();
                let x = color_experiments();
                let mut matrices = Vec::new();
                let mut ilut_mirror = false;
                for layer in &planned.layers {
                    let so = comp
                        .layers
                        .iter()
                        .find(|so| so.window == layer.window)
                        .ok_or("colour layer has no armed window")?;
                    let lut = kf_disp::color::input(
                        t,
                        win,
                        |m| engine.armed(ChannelKind::Window, so.window, m).unwrap_or(0),
                        x,
                    )
                    .map_err(str::to_owned)?;
                    let life = engine
                        .generation(so.chn)
                        .ok_or("no colour window incarnation")?;
                    let pipeline = kf_disp::color::pipeline(
                        t,
                        win,
                        |m| engine.armed(ChannelKind::Window, so.window, m).unwrap_or(0),
                        engine
                            .armed_inline(so.window)
                            .ok_or("missing armed inline tables")?,
                    )
                    .map_err(str::to_owned)?;
                    matrices.extend(pipeline.matrices);
                    ilut_mirror |= lut.is_some_and(|l| l.mirror);
                    let tmo = self.colors.resolve(
                        32 + so.window as usize,
                        so.client,
                        so.chn,
                        life,
                        pipeline.tmo,
                        |handle| io.resolve(so.client, handle, so.chn),
                    )?;
                    let program = kf_cuda::display::ColorPipeline {
                        matrices: pipeline.matrices,
                        segments: std::array::from_fn(|i| {
                            pipeline.inline[i]
                                .as_ref()
                                .map_or([0; 64], |p| p.segments.map(|v| u32::from(v.unwrap_or(0))))
                        }),
                        entries: std::array::from_fn(|i| {
                            pipeline.inline[i]
                                .as_ref()
                                .map_or([0; 1025], |p| p.entries.map(|v| u32::from(v.unwrap_or(0))))
                        }),
                        enable: pipeline.inline.each_ref().map(Option::is_some),
                    };
                    let input = self.colors.resolve(
                        so.window as usize,
                        so.client,
                        so.chn,
                        life,
                        lut,
                        |handle| io.resolve(so.client, handle, so.chn),
                    )?;
                    inputs.push((input, tmo, program));
                }
                let out = kf_disp::color::output(
                    t,
                    core,
                    comp.head,
                    |m| engine.armed(ChannelKind::Core, 0, m).unwrap_or(0),
                    x,
                )
                .map_err(str::to_owned)?;
                // ⚠ KF3_DISPLAY_LUT_MIRROR: the output kernel samples a mirrored OLUT's negative side
                // as the odd extension (`[inferred]`, `kf_disp::color::model::value`). A signed
                // matrix in front of it (the NVIDIA app's Hue / Digital Vibrance, run 4500) used to
                // be refused here, which froze the console; it is rendered now.
                if ilut_mirror || out.lut.is_some_and(|l| l.mirror) {
                    static NOTED: std::sync::Once = std::sync::Once::new();
                    NOTED.call_once(|| {
                        eprintln!(
                            "kf3: display: EXPERIMENT KF3_DISPLAY_LUT_MIRROR=1 — a mirrored LUT accepted (ILUT mirrored={ilut_mirror}, OLUT {:?})",
                            out.lut
                        );
                    });
                }
                if kf_disp::color::negative_side_reachable(matrices.iter(), &out) {
                    static NEGATIVE: std::sync::Once = std::sync::Once::new();
                    NEGATIVE.call_once(|| {
                        eprintln!(
                            "kf3: display: a mirrored OLUT is behind a signed matrix: its negative side is sampled as the odd extension, lut(-x) = -lut(x) [inferred, unconfirmed on hardware]"
                        );
                    });
                }
                let client = engine.client(0).ok_or("no core colour client")?;
                let life = engine.generation(0).ok_or("no core colour incarnation")?;
                let lut = self
                    .colors
                    .resolve(64, client, 0, life, out.lut, |handle| {
                        io.resolve(client, handle, 0)
                    })?;
                Ok((inputs, lut, out.matrix))
            })();
            match program {
                Ok(p) => Some(p),
                Err(e) => {
                    // the console's colour conversion of this frame is what failed: console-only
                    self.fault(
                        &dp.counters,
                        &dp.console,
                        n,
                        req,
                        Fault::Console(format!("SDR colour program: {e}")),
                    );
                    return;
                }
            }
        } else {
            None
        };
        let mut layers = planned.layers.clone();
        // ★ §O: with a cursor-capable broker the guest's cursor image is read (a GPU copy into a
        // buffer kf owns) and posted to the relay; in hover it is then left out of the frame —
        // unless the host cannot show it, which is composed in every mode
        let mode = self.cursor_mode;
        let stream = dp.broker.as_ref().is_some_and(|b| b.cursor().stream());
        // the stream is mode-free: the cursor is read whenever the broker streams (the mode is the
        // composing fallback's)
        let want = (stream || mode.reads()).then(|| self.host_cursor_want(io, shown, cursor));
        let compose = cursor.filter(|_| cursor_composed(mode, stream, want.as_ref()));
        if let (Some(w), Some(seat)) = (want, dp.broker.as_ref()) {
            let key = w.key();
            if self.cursor_posted != Some(key) && seat.cursor().post(w) {
                self.cursor_posted = Some(key);
            }
        }
        self.cursor_in_frame = compose.is_some();
        // ★ 3d: the head's cursor, last — the top layer (its context DMA is the core channel's)
        let mut expand = None;
        if let Some(cs) = compose {
            let planned = io.resolve(cs.client, cs.handle, 0).and_then(|dma| {
                let p = kf_disp::scanout::plan_cursor_any(cs, &dma, w, h).map_err(|r| r.0);
                if p.is_err() {
                    self.debug_cursor(io, cs, "scanout");
                }
                p
            });
            match planned {
                Ok(Some(kf_disp::scanout::CursorPlan::Layer(l))) => layers.push(l),
                Ok(Some(kf_disp::scanout::CursorPlan::Expand(x))) => expand = Some(x),
                Ok(None) => {}
                Err(e) => self.refuse(dp, &format!("cursor: {e}")),
            }
        }
        let Some(gpu) = io.gpu.as_mut() else {
            self.finish(dp, n, req);
            return;
        };
        // ★ an A1R5G5B5 cursor: the GPU copies the (bounded, inside-the-context-DMA) image into a
        // buffer kf owns, the CPU expands that copy to 32 bits, and the kernel composes the
        // expansion from a device buffer kf owns — never guest memory read by the CPU
        let cursor_px = expand.and_then(|x| {
            let up = (|| -> Result<kf_disp::scanout::LayerPlan, String> {
                let mut raw = vec![0u8; usize::try_from(x.extent()).map_err(|e| e.to_string())?];
                gpu.read_store(x.src(), &mut raw)
                    .map_err(|e| format!("copying the cursor image: {e}"))?;
                let (l, px) = x.expand(&raw).map_err(|r| r.0)?;
                gpu.cursor_upload(&px).map_err(|e| e.to_string())?;
                Ok(l)
            })();
            match up {
                Ok(l) => Some(l),
                Err(e) => {
                    self.refuse(dp, &format!("cursor: {e}"));
                    None
                }
            }
        });
        // ★ the YUV windows: composed after the RGB layers and before the cursor. Without the
        // kernel, or for a window the kernel's own bounds refuse, the window is left out of the
        // console copy by name (console-only); a real CUDA error still fails the copy.
        let yuv_run: &[kf_disp::scanout::YuvPlan] = match (planned.yuv.is_empty(), gpu.yuv_ready())
        {
            (true, _) => &[],
            (false, Ok(())) => &planned.yuv,
            (false, Err(e)) => {
                for y in &planned.yuv {
                    self.note_refusal(
                        &dp.counters,
                        &format!("window {} left out of the console copy: {e}", y.window),
                    );
                    dp.counters
                        .console_windows_left_out
                        .fetch_add(1, Ordering::Relaxed);
                }
                &[]
            }
        };
        let mut yuv_left_out: Vec<String> = Vec::new();
        // ★ one YUV window onto the staging frame (`colour`: the FP32 frame of the SDR pipeline)
        let mut yuv_one = |gpu: &kf_cuda::display::DisplayGpu,
                           y: &kf_disp::scanout::YuvPlan,
                           colour: bool|
         -> Result<(), kf_cuda::CudaError> {
            let l = yuv_layer(y);
            let r = if colour {
                gpu.color_yuv(&l, w, h)
            } else {
                gpu.compose_yuv(&l, w, h)
            };
            match r {
                Err(kf_cuda::CudaError::Refused { code: 0, name, .. }) => {
                    yuv_left_out.push(format!("window {}: {name}", y.window));
                    Ok(())
                }
                r => r,
            }
        };
        // ★ the programmed depth order: RGB layers and YUV windows back to front by
        // `SET_COMPOSITION_CONTROL.DEPTH` (deepest first), not "YUV last"
        let depth_of = |win: u32| match shown {
            Shown::Armed(c) => c
                .layers
                .iter()
                .find(|l| l.window == win)
                .map_or(0, |l| l.depth),
            _ => 0,
        };
        let order = z_order(&planned.layers, yuv_run, depth_of);
        if !yuv_run.is_empty() {
            static ORDER_LOGGED: AtomicU32 = AtomicU32::new(0);
            if trace_slot(&ORDER_LOGGED, 4) {
                let depths: Vec<_> = match shown {
                    Shown::Armed(c) => c
                        .layers
                        .iter()
                        .map(|l| {
                            (
                                l.window,
                                l.depth,
                                l.format,
                                l.src_factor,
                                l.dst_factor,
                                l.k1,
                            )
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                eprintln!(
                    "kf3: display: composition with a YUV window: (window, depth, format, src_factor, dst_factor, k1) {depths:?}; back-to-front order {order:?}"
                );
            }
        }
        let n_rgb = planned.layers.len();
        let composed = if let Some((inputs, out, matrix)) = &color {
            gpu.color_begin(w, h)
                .and_then(|()| {
                    order.iter().try_for_each(|z| match *z {
                        Z::Rgb(i) => {
                            let (lut, tmo, program) = &inputs[i];
                            gpu.color_pipeline_layer(
                                planned.layers[i].window,
                                &compose_layer(&planned.layers[i]),
                                *lut,
                                *tmo,
                                Some(program),
                                w,
                                h,
                            )
                        }
                        Z::Yuv(j) => yuv_one(gpu, &yuv_run[j], true),
                    })
                })
                .and_then(|()| gpu.color_output(w, h, *out, matrix))
                .and_then(|()| {
                    layers[n_rgb..]
                        .iter()
                        .try_for_each(|l| gpu.compose_layer(&compose_layer(l), w, h))
                })
                .and_then(|()| {
                    cursor_px
                        .as_ref()
                        .map_or(Ok(()), |l| gpu.compose_cursor(&compose_layer(l), w, h))
                })
                .map_err(|e| format!("SDR composition {w}x{h}: {e}"))
        } else {
            gpu.compose_begin(w, h)
                .map_err(|e| format!("composition {w}x{h}: {e}"))
                .and_then(|()| {
                    order.iter().try_for_each(|z| match *z {
                        Z::Rgb(i) => gpu
                            .compose_layer(&compose_layer(&layers[i]), w, h)
                            .map_err(|e| format!("window {}: {e}", layers[i].window)),
                        Z::Yuv(j) => {
                            yuv_one(gpu, &yuv_run[j], false).map_err(|e| format!("YUV window: {e}"))
                        }
                    })
                })
                .and_then(|()| {
                    layers[n_rgb..].iter().try_for_each(|l| {
                        gpu.compose_layer(&compose_layer(l), w, h)
                            .map_err(|e| format!("window {}: {e}", l.window))
                    })
                })
                .and_then(|()| {
                    cursor_px.as_ref().map_or(Ok(()), |l| {
                        gpu.compose_cursor(&compose_layer(l), w, h)
                            .map_err(|e| format!("cursor: {e}"))
                    })
                })
        };
        for why in std::mem::take(&mut yuv_left_out) {
            self.note_refusal(&dp.counters, &format!("YUV {why}"));
            dp.counters
                .console_windows_left_out
                .fetch_add(1, Ordering::Relaxed);
        }
        if let Err(e) = composed {
            // work sent to the GPU failed: no synthetic success
            self.fault(&dp.counters, &dp.console, n, req, Fault::Gpu(e));
            return;
        }
        // ★ §8.16: the change detector, queued behind the composition (a refusal costs only the
        // detection: the frame is then always sent)
        let summed = gpu.checksum_refused().is_none() && gpu.compose_checksum(w, h).is_ok();
        // Preserve the bounded window plans, without the independently managed cursor.
        self.copied(shown, &planned, (w, h));
        let f = Inflight {
            color: color.is_some(),
            n,
            phase: Phase::Check,
            slot: 0,
            wh: (w, h),
            t0,
            vram: None,
            d2h: false,
            cursor: self.cursor_in_frame,
            req,
            summed,
            boot: matches!(shown, Shown::Boot(..)),
        };
        if check {
            match gpu.compose_signal() {
                Ok(()) => self.inflight = Some(f),
                Err(e) => {
                    self.refuse(dp, &format!("the completion signal: {e}"));
                    self.failed = true;
                    self.serve_now(dp);
                }
            }
            return;
        }
        let p = self.plan_send(io, (w, h));
        self.send(io, &f, &p);
    }

    /// ★ Display step 3 / §8.11: which copies a `w` x `h` frame gets NOW — the pack into a VRAM slot
    /// (rung 0), the D2H into host memory, both, or neither (`kf_broker::gpucopy::plan`, GPU-free
    /// and tested) — provisioning VRAM slots on the way.
    fn plan_send(&mut self, io: &mut Io<'_>, (w, h): (u32, u32)) -> SendPlan {
        let dp = io.dp;
        let ring = dp.console.ring().clone();
        let geom = kf_disp::vramslot::slot_geom(w, h, kf_disp::vramslot::SLOT_H_LOG2);
        let want_vram = dp.broker.is_some() && ring.want_vram();
        let mut eligible = 0u32;
        if let (Some(v), Some(gpu)) = (self.vram.as_mut(), io.gpu.as_mut()) {
            v.poll(gpu, &ring, &dp.counters);
            if want_vram {
                let r = v.plan.first(false, true);
                v.ask(r, &ring);
                if let Some(g) = geom {
                    let r = v.plan.grow(g.extent);
                    v.ask(r, &ring);
                    eligible = v.eligible(&ring, g.extent);
                }
            }
        }
        let choice = kf_broker::gpucopy::Choice {
            broker: dp.broker.is_some() && dp.console.broker_wanted_within(WATCHED_MS),
            want_vram,
            vram_slot: eligible != 0,
            console: dp.console.console_wanted_within(WATCHED_MS),
            host_withdrawn: ring.withdrawn(kf_broker::Kind::Host),
            vram_withdrawn: ring.withdrawn(kf_broker::Kind::Vram),
        };
        let plan = kf_broker::gpucopy::plan(&choice);
        if choice.broker && want_vram && !plan.pack {
            dp.counters
                .scanout_pack_skipped
                .fetch_add(1, Ordering::Relaxed);
        }
        SendPlan {
            plan,
            eligible,
            geom,
        }
    }

    /// ★ Send the composition the staging frame holds (copy `f.n`): the pack and/or the D2H the plan
    /// chose into a free slot, then the completion signal; [`ScanState::completed`] publishes it.
    fn send(&mut self, io: &mut Io<'_>, f: &Inflight, p: &SendPlan) {
        let dp = io.dp;
        let (w, h) = f.wh;
        let plan = p.plan;
        let ring = dp.console.ring().clone();
        let Some(gpu) = io.gpu.as_mut() else {
            self.failed = true;
            self.serve_now(dp);
            return;
        };
        if plan.none() {
            // Composition already reached the GPU. Even without a consumer, wait
            // for its real signal before completing the flip.
            match gpu.compose_signal() {
                Ok(()) => {
                    self.inflight = Some(Inflight {
                        phase: Phase::Send,
                        d2h: false,
                        vram: None,
                        ..*f
                    })
                }
                Err(e) => {
                    self.refuse(dp, &format!("unpublished frame signal: {e}"));
                    self.failed = true;
                    self.serve_now(dp);
                }
            }
            return;
        }
        // the pack takes the eligible free slot the broker gave back longest ago (a RELEASE is
        // not GPU-idle); otherwise any free slot
        let picked = if plan.pack {
            ring.fill_target_lru(None, !p.eligible & 0x1f)
        } else {
            dp.console.free_slot()
        };
        let Some(slot) = picked else {
            dp.counters.scanout_no_slot.fetch_add(1, Ordering::Relaxed);
            self.refuse(
                dp,
                "no free frame slot (the frame ring's cap argument broke)",
            );
            self.failed = true;
            self.serve_now(dp);
            return;
        };
        let need = w as usize * h as usize * 4;
        if plan.d2h && self.frames[slot].as_ref().is_none_or(|f| f.len() < need) {
            let cap = if need <= FRAME_SMALL {
                FRAME_SMALL
            } else {
                FRAME_MAX
            };
            // ★ with the broker on: a sealed memfd the broker also receives (registered for the
            // copy, a udmabuf over it when /dev/udmabuf opened); refused once, by name, the
            // console falls back to its own frames and the ring is withdrawn from the broker
            // ([`broker_backing`])
            let shared: &DisplayGpu = gpu;
            let seat = dp
                .broker
                .as_ref()
                .map(|b| move || b.frame(shared, slot, cap));
            let made = match broker_backing(dp.console.ring(), &mut self.broker_refused, seat) {
                Some(f) => Ok(f),
                None => gpu.frame(cap).map_err(|e| e.to_string()),
            };
            match made {
                Ok(f) => {
                    if let Some(old) = self.frames[slot].replace(f) {
                        self.retired.push(old);
                    }
                }
                Err(e) => {
                    self.refuse(dp, &format!("a {cap:#x}-byte console frame: {e}"));
                    self.failed = true;
                    self.serve_now(dp);
                    return;
                }
            }
        }
        let frame = if plan.d2h {
            let Some(f) = self.frames[slot].as_ref() else {
                self.failed = true;
                self.serve_now(dp);
                return;
            };
            Some(f)
        } else {
            None
        };
        // the pack's slot and shape (only when it was planned: an eligible slot exists)
        let pack = if plan.pack {
            self.pack_for(slot, p.geom, w, h)
        } else {
            None
        };
        let queued = match &pack {
            Some((sid, pk, _)) => gpu
                .compose_to_slot(*sid, pk)
                .map_err(|e| format!("the pack into display VRAM: {e}")),
            None => Ok(()),
        }
        .and_then(|()| match frame {
            Some(fr) => gpu
                .compose_to_host(w, h, fr)
                .map_err(|e| format!("the frame copy: {e}")),
            None => Ok(()),
        })
        .and_then(|()| {
            gpu.compose_signal()
                .map_err(|e| format!("the completion signal: {e}"))
        });
        match queued {
            Ok(()) => {
                if f.boot {
                    dp.counters.boot_frames.fetch_add(1, Ordering::Relaxed);
                }
                if pack.is_some() {
                    dp.counters.scanout_pack.fetch_add(1, Ordering::Relaxed);
                }
                if frame.is_some() {
                    dp.counters.scanout_d2h.fetch_add(1, Ordering::Relaxed);
                }
                self.inflight = Some(Inflight {
                    phase: Phase::Send,
                    slot,
                    vram: pack.map(|(_, _, g)| g),
                    d2h: frame.is_some(),
                    ..*f
                });
            }
            Err(e) => {
                self.refuse(dp, &e);
                self.failed = true;
                self.serve_now(dp);
            }
        }
    }

    /// ★ Plan the layers copy `n` composes for `shown`: the boot layer, the preserved layers, or
    /// every window of an armed composition planned against the context DMA its ARMED state resolved
    /// to ([`LatchedDmas`]; `resolve` reads the guest's instance memory). A window that cannot be
    /// composed is refused by name and left out.
    fn plan(
        &mut self,
        shown: &Shown,
        formats: &ScanFormats,
        (w, h): (u32, u32),
        n: u64,
        mut resolve: impl FnMut(&kf_disp::engine::Scanout) -> Result<CtxDma, String>,
    ) -> Planned {
        let mut p = Planned::default();
        let windows: &[kf_disp::engine::Scanout] = match shown {
            Shown::Armed(comp) => &comp.layers,
            Shown::Boot(layer, _) => {
                p.layers.push(*layer);
                &[]
            }
            // ⊘ planned (and bounded) when they were armed; the compose kernel bounds each read
            // again against the store (`DisplayGpu::compose_layer`)
            Shown::Preserved(kept, _) => {
                p.layers.extend_from_slice(kept);
                &[]
            }
            // no layer: `compose_begin` clears the frame to black
            Shown::Blank(_) => &[],
        };
        for so in windows {
            // ★ a semi-planar YUV window (Edge's video overlay, `FORMAT 0x38`): its own planner
            if let Some(yf) = formats.yuv_of(so.format) {
                let planned = self.latched.resolve(so, || resolve(so)).and_then(|y_dma| {
                    let c_dma = if so.iso1 == 0 || so.iso1 == so.handle {
                        y_dma
                    } else {
                        resolve(&kf_disp::engine::Scanout {
                            handle: so.iso1,
                            ..*so
                        })?
                    };
                    kf_disp::scanout::plan_yuv(so, yf, &y_dma, &c_dma, w, h).map_err(|r| r.0)
                });
                match planned {
                    Ok(Some(l)) => {
                        static LOGGED: AtomicU32 = AtomicU32::new(0);
                        if trace_slot(&LOGGED, 6) {
                            eprintln!(
                                "kf3: display: YUV window {} composed for the console: armed {so:?} -> {l:?}",
                                so.window
                            );
                        }
                        p.yuv.push(l);
                    }
                    Ok(None) => {}
                    Err(e) => p.refused.push(e),
                }
                continue;
            }
            let planned = self.latched.resolve(so, || resolve(so)).and_then(|dma| {
                kf_disp::scanout::plan_layer(so, &dma, formats, w, h).map_err(|r| r.0)
            });
            match planned {
                Ok(Some(l)) => {
                    if self.trace && (n <= 8 || n.is_multiple_of(50)) {
                        eprintln!(
                            "kf3: display: TRACE scanout copy {n}: window {} depth {} iso {:#x} -> src {:#x} {} pitch {} {}x{} at ({}, {}) flags {:#x} blend ({},{})/({},{})",
                            so.window,
                            so.depth,
                            so.handle,
                            l.src,
                            if l.block_linear { "BL" } else { "pitch" },
                            l.pitch,
                            l.width,
                            l.rows,
                            l.ox,
                            l.oy,
                            l.flags,
                            l.a_s,
                            l.b_s,
                            l.a_d,
                            l.b_d
                        );
                    }
                    p.layers.push(l);
                }
                Ok(None) => {}
                Err(e) => p.refused.push(e),
            }
        }
        p
    }

    /// A copy of `shown` was queued: an armed composition composed whole is what a `PRESERVE_HW`
    /// free keeps ([`Self::last_plan`]); one that refused a window keeps nothing.
    fn copied(&mut self, shown: &Shown, planned: &Planned, size: (u32, u32)) {
        if matches!(shown, Shown::Armed(_)) {
            self.last_plan = planned
                .refused
                .is_empty()
                .then(|| (planned.layers.clone(), size));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_broker::CursorImage;

    /// ★ §8.18, the interleaving of runs e1-a…d (2026-10-08): the first copy is queued at +0.3 s,
    /// host RM spends 3.53 s registering the guest-RAM memfd, the copy's signal comes after that.
    /// At every point of that wait the copy is NOT lost (the old rule gave it up at 2 s for good);
    /// only a stream that reports a failure, or no GPU to ask, loses it. Known-positive: the old
    /// rule (`elapsed > STUCK_COPY` alone) answers "lost" at 2.1 s, which this test refuses.
    #[test]
    fn a_late_copy_behind_a_busy_host_rm_is_waited_for_not_given_up() {
        let ms = Duration::from_millis;
        assert_eq!(stuck_verdict(ms(300), Some(Ok(false))), Stuck::Running);
        assert_eq!(stuck_verdict(STUCK_COPY, Some(Ok(false))), Stuck::Running);
        for t in [2_100, 3_528, 10_000] {
            assert_eq!(
                stuck_verdict(ms(t), Some(Ok(false))),
                Stuck::Waiting,
                "{t} ms, still queued"
            );
            assert_eq!(
                stuck_verdict(ms(t), Some(Ok(true))),
                Stuck::Waiting,
                "{t} ms, done: its signal is on the way"
            );
        }
        // review finding 1: a FINISHED copy whose signal never comes is bounded, a queued one is not
        let late = STUCK_COPY + SIGNAL_GRACE + ms(1);
        assert!(matches!(
            stuck_verdict(late, Some(Ok(true))),
            Stuck::Lost(w) if w.contains("completion signal")
        ));
        assert_eq!(
            stuck_verdict(STUCK_COPY + SIGNAL_GRACE, Some(Ok(true))),
            Stuck::Waiting
        );
        assert_eq!(stuck_verdict(ms(600_000), Some(Ok(false))), Stuck::Waiting);
        let old_rule = |e: Duration| e > STUCK_COPY;
        assert!(
            old_rule(ms(2_100))
                && stuck_verdict(ms(2_100), Some(Ok(false))) != Stuck::Lost(String::new()),
            "the case the old rule lost"
        );
        assert!(matches!(
            stuck_verdict(ms(2_100), Some(Err("CUDA_ERROR_ILLEGAL_ADDRESS".into()))),
            Stuck::Lost(w) if w.contains("ILLEGAL_ADDRESS")
        ));
        assert!(matches!(stuck_verdict(ms(2_100), None), Stuck::Lost(_)));
        assert_eq!(stuck_verdict(ms(100), None), Stuck::Running);
    }

    #[test]
    fn color_snapshot_binding_survives_unbind_but_not_update_or_incarnation_change() {
        use kf_disp::color::{Binding, LUT_BYTES, Lut};
        let mut cache = ColorDmas::default();
        let lut = Some(Lut {
            entries: 1025,
            binding: Binding::Dma {
                handle: 9,
                offset: 256,
            },
            interpolate: false,
            mirror: false,
        });
        let dma = CtxDma {
            target: Target::Vidmem,
            base: 4096,
            limit: 4096 + 256 + LUT_BYTES - 1,
            block_linear: false,
            writable: false,
        };
        let first = cache
            .resolve(0, 7, 1, 42, lut, |_| Ok(dma))
            .unwrap()
            .unwrap();
        let kept = cache
            .resolve(0, 7, 1, 42, lut, |_| Err("unbound".into()))
            .unwrap()
            .unwrap();
        assert_eq!((first.src, first.token), (kept.src, kept.token));
        assert!(
            cache
                .resolve(0, 7, 1, 43, lut, |_| Err("new channel".into()))
                .is_err()
        );
        cache.forget(0);
        assert!(
            cache
                .resolve(0, 7, 1, 42, lut, |_| Err("new UPDATE".into()))
                .is_err()
        );
        let next = cache
            .resolve(0, 7, 1, 42, lut, |_| Ok(dma))
            .unwrap()
            .unwrap();
        assert!(next.token > first.token);
        cache
            .resolve(0, 7, 1, 42, None, |_| unreachable!())
            .unwrap();
        assert!(
            cache
                .resolve(0, 7, 1, 42, lut, |_| Err("disabled then re-enabled".into()))
                .is_err()
        );
    }

    /// Run 4500: a mirrored OLUT reaches the output kernel as mirrored (the flag the kernel reads
    /// is the one the register decode produced), and arming MIRROR on the same table is a new
    /// binding, never the cached one without it.
    #[test]
    fn a_mirrored_olut_reaches_the_kernel_as_mirrored() {
        use kf_disp::color::{Binding, LUT_BYTES, Lut};
        let mut cache = ColorDmas::default();
        let dma = CtxDma {
            target: Target::Vidmem,
            base: 4096,
            limit: 4096 + 256 + LUT_BYTES - 1,
            block_linear: false,
            writable: false,
        };
        let lut = |mirror| {
            Some(Lut {
                entries: 1025,
                binding: Binding::Dma {
                    handle: 9,
                    offset: 256,
                },
                interpolate: true,
                mirror,
            })
        };
        let plain = cache
            .resolve(64, 7, 0, 1, lut(false), |_| Ok(dma))
            .unwrap()
            .unwrap();
        assert!(!plain.mirror);
        let mirrored = cache
            .resolve(64, 7, 0, 1, lut(true), |_| Ok(dma))
            .unwrap()
            .unwrap();
        assert!(mirrored.mirror, "the kernel is told the table is mirrored");
        assert!(
            mirrored.token > plain.token,
            "a changed control is a new binding"
        );
        assert_eq!((mirrored.src, mirrored.entries), (plain.src, plain.entries));
        let again = cache
            .resolve(64, 7, 0, 1, lut(true), |_| Err("cached".into()))
            .unwrap()
            .unwrap();
        assert_eq!((again.token, again.mirror), (mirrored.token, true));
    }

    /// The other direction, and the input table: dropping MIRROR is a new binding that is not
    /// mirrored (a stale cached "mirrored" would sample a plain table's negative side).
    #[test]
    fn a_table_that_stops_being_mirrored_stops_reaching_the_kernel_as_mirrored() {
        use kf_disp::color::{Binding, LUT_BYTES, Lut};
        let mut cache = ColorDmas::default();
        let dma = CtxDma {
            target: Target::Vidmem,
            base: 0,
            limit: LUT_BYTES - 1,
            block_linear: false,
            writable: false,
        };
        let lut = |mirror| {
            Some(Lut {
                entries: 1025,
                binding: Binding::Dma {
                    handle: 9,
                    offset: 0,
                },
                interpolate: false,
                mirror,
            })
        };
        let on = cache
            .resolve(0, 7, 1, 1, lut(true), |_| Ok(dma))
            .unwrap()
            .unwrap();
        let off = cache
            .resolve(0, 7, 1, 1, lut(false), |_| Ok(dma))
            .unwrap()
            .unwrap();
        assert!(on.mirror && !off.mirror && off.token > on.token);
    }

    /// Every refusal is counted; only a new reason is logged. The state a frozen console leaves
    /// behind: the counter says how many frames, the set says which reasons.
    #[test]
    fn a_repeated_refusal_is_counted_every_time_and_remembered_once() {
        let counters = DispCounters::default();
        let mut scan = ScanState::default();
        for _ in 0..1000 {
            scan.note_refusal(&counters, "SDR colour program: active OCSC1");
        }
        assert_eq!(counters.scanout_refused.load(Ordering::Relaxed), 1000);
        assert_eq!(scan.refusal_log.seen.len(), 1);
        for i in 0..200 {
            scan.note_refusal(&counters, &format!("reason {i}"));
        }
        assert_eq!(counters.scanout_refused.load(Ordering::Relaxed), 1200);
        assert_eq!(scan.refusal_log.seen.len(), ReasonLog::MAX);
    }

    /// A refusal is logged once per distinct reason, however often it repeats (run 4500 repeated
    /// one reason for thousands of frames; a different reason after sixteen earlier ones was never
    /// logged at all), and the set is bounded against a guest that varies the text.
    #[test]
    fn each_distinct_refusal_reason_is_logged_once_and_the_set_is_bounded() {
        let mut log = ReasonLog::default();
        let signed = "SDR colour program: x";
        assert_eq!(log.admit(signed), Admit::New);
        for _ in 0..1000 {
            assert_eq!(log.admit(signed), Admit::Seen, "repeats are silent");
        }
        assert_eq!(log.admit("SDR colour program: y"), Admit::New);
        for i in 0..ReasonLog::MAX {
            let r = log.admit(&format!("a {i}x1 composition"));
            if i + 2 < ReasonLog::MAX {
                assert_eq!(r, Admit::New, "{i}");
            }
        }
        assert_eq!(log.seen.len(), ReasonLog::MAX, "bounded");
        assert_eq!(log.admit("one more"), Admit::Seen, "full: counted only");
        assert_eq!(log.admit(signed), Admit::Seen);
        // a long text is keyed by its prefix: one reason, not one per tail
        let mut log = ReasonLog::default();
        let long = "z".repeat(ReasonLog::LEN + 40);
        assert_eq!(log.admit(&long), Admit::New);
        assert_eq!(log.admit(&format!("{long}tail")), Admit::Seen);
    }

    /// ★ §O: the worker's hover/grab switch. Without a cursor-capable broker (nothing read) and under
    /// grab the cursor is composed; in hover it is left out unless the host cannot show it.
    /// ⊘ CORRECTED 2026-10-04 (`display-max-fps`, §8.16): a cursor move no longer makes a copy of
    /// its own (`move_recomposes` is gone) — the console head's next tick composes the frame and
    /// sends it only when its checksum changed, so "a hover move makes no frame" now holds because
    /// a hover frame composes no cursor (`kf_disp::pace::NonFlip`, tested there).
    #[test]
    fn hover_leaves_the_cursor_out_of_the_frame() {
        let img = CursorWant::Image(Arc::new(
            CursorImage::new(32, 32, (0, 0), vec![1; 32 * 32 * 4]).unwrap(),
        ));
        assert!(
            cursor_composed(CursorMode::Off, false, None),
            "today's path"
        );
        for w in [&img, &CursorWant::Hidden, &CursorWant::Composed] {
            assert!(
                cursor_composed(CursorMode::Grabbed, false, Some(w)),
                "{w:?}"
            );
        }
        assert!(!cursor_composed(CursorMode::Hover, false, Some(&img)));
        assert!(!cursor_composed(
            CursorMode::Hover,
            false,
            Some(&CursorWant::Hidden)
        ));
        assert!(
            cursor_composed(CursorMode::Hover, false, Some(&CursorWant::Composed)),
            "XOR"
        );
    }

    /// ★ The pointer pair the relay sends the broker (`kf_broker::guest_point`) is the guest-frame
    /// pixel the composing fallback's `hot_from_pointer` works from: both are the two truncating
    /// scalings, so the broker's `hot = pointer - top-left` agrees with the tracker's.
    #[test]
    fn the_streamed_pointer_is_the_pixel_the_fallback_derives_its_hot_spot_from() {
        let mut cs = kf_disp::engine::CursorScan {
            head: 0,
            client: 0,
            handle: 0,
            offset: 0,
            size: 256,
            argb8888: true,
            a1r5g5b5: false,
            format: 0xCF,
            hot_x: 0,
            hot_y: 0,
            x: 300,
            y: 200,
            k1: 255,
            cursor_factor: 2,
            viewport_factor: 7,
            mode: 0,
        };
        for (abs, range, frame) in [
            ((48, 8), (1024, 695), (1024u32, 695u32)),
            ((350, 260), (1000, 700), (1920, 1080)),
            ((410, 255), (1024, 768), (1024, 768)),
        ] {
            let g = kf_broker::guest_point(abs, range, frame).unwrap();
            // the image's top-left at (g.0 - 17, g.1 - 9): the derived hot spot is the difference
            cs.x = g.0 - 17;
            cs.y = g.1 - 9;
            let hot = kf_disp::scanout::hot_from_pointer(&cs, abs, range, frame);
            assert_eq!(hot, Some((17, 9)), "{abs:?} {range:?} {frame:?}");
        }
    }

    /// ★ A streaming broker draws the cursor itself in EVERY mode: nothing is ever composed (the rule
    /// is `kf_broker::composes`, tested there with plain inputs).
    #[test]
    fn a_streaming_broker_gets_no_cursor_composed_in_any_mode() {
        let img = CursorWant::Image(Arc::new(
            CursorImage::new(32, 32, (0, 0), vec![1; 32 * 32 * 4]).unwrap(),
        ));
        for m in [CursorMode::Off, CursorMode::Grabbed, CursorMode::Hover] {
            for w in [&img, &CursorWant::Hidden, &CursorWant::Composed] {
                assert!(!cursor_composed(m, true, Some(w)), "{m:?} {w:?}");
            }
        }
    }

    /// ★ §8.16: the check clock of a picture with no armed head (the boot layer, a preserved
    /// scanout) ticks every period only while it runs, reschedules a late tick as the pacer does,
    /// and stops — forgetting its phase — when it does not. (Mutation: a clock that keeps running
    /// unwatched makes copies nobody sees.)
    #[test]
    fn the_boot_check_clock_ticks_only_while_it_runs() {
        let mut s = ScanState::default();
        let p = Duration::from_millis(16);
        let t0 = Instant::now();
        assert!(!s.idle_tick(t0, p, true), "armed, not due");
        assert_eq!(s.idle_at, Some(t0 + p));
        assert!(!s.idle_tick(t0 + p / 2, p, true));
        assert!(s.idle_tick(t0 + p, p, true));
        assert_eq!(s.idle_at, Some(t0 + 2 * p));
        // a stall of many periods: one tick, re-phased from now
        let late = t0 + 10 * p;
        assert!(s.idle_tick(late, p, true));
        assert_eq!(s.idle_at, Some(late + p));
        assert!(!s.idle_tick(late + p, p, false), "unwatched: no tick");
        assert_eq!(s.idle_at, None);
    }

    /// ★ §8.16 D2.3: a refresh request is counted and a served one makes the descriptor readable
    /// (the C device's handler ends the screendump's wait); a console or broker that starts watching
    /// bumps the new-watcher counter once, not on every request.
    #[test]
    fn the_console_refresh_and_new_watchers() {
        let c = ConsoleShare::default();
        assert!(c.refresh_fd() >= 0);
        assert_eq!(c.refresh_requested(), 0);
        assert!(c.request_refresh());
        assert!(c.request_refresh());
        assert_eq!(c.refresh_requested(), 2);
        c.serve_refresh(2);
        assert_eq!(c.refresh_served(), 2);
        c.serve_refresh(1);
        assert_eq!(c.refresh_served(), 2, "never backwards");
        let e = c.watch_epoch();
        c.note_demand();
        assert_eq!(c.watch_epoch(), e + 1, "a new console watcher");
        c.note_demand();
        let _ = c.take();
        assert_eq!(c.watch_epoch(), e + 1, "still the same watcher");
        c.note_broker_demand();
        assert_eq!(c.watch_epoch(), e + 2, "a new broker watcher");
        c.note_new_watcher();
        assert_eq!(c.watch_epoch(), e + 3, "a new broker session");
        c.refresh_drain();
    }

    /// ★ Phase 2 (§11.4): the worker's wiring of the lock step, as a source scan (the loop cannot
    /// run without a GPU) — the broker's ticks reach the pacer only through the gate's freshness
    /// check, a broker credit reaches the mailbox only through `CreditGate::poll`, and only a
    /// FREE-clock tick grants a copy credit by itself. Known-positive on a planted line. The
    /// constants the broker, the pacer and the relay share are the same numbers.
    #[test]
    fn the_worker_feeds_broker_ticks_through_the_gate_only() {
        let src = include_str!("display.rs");
        let code: String = src
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .lines()
            .map(|l| l.split("//").next().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        for needed in [
            "gate_c.fresh(",
            "pacer.broker_tick(",
            "gate_c.on_tick(",
            "gate_c.poll(",
            "scan.mailbox.grant()",
            "ticked_free &",
            "gate_c.next_ns()",
            "b.ticks().set_wake(",
            "b.ticks().set_cap_hz(",
            "gate_c.reset()",
            "b.ticks().epoch()",
        ] {
            assert!(code.contains(needed), "the worker no longer calls {needed}");
        }
        // a broker tick must not grant a copy credit by itself: `on_console_tick` is reached only
        // through the free-clock mask
        assert_eq!(code.matches("scan.on_console_tick(").count(), 1, "{code}");
        assert!(
            !code.contains("scan.mailbox.tick()"),
            "no direct tick grant in the worker"
        );
        // the numbers the broker (the C header), the relay and the pacer share
        assert_eq!(kf_broker::wire::GUEST_MAX_HZ, kf_disp::pace::MAX_HZ);
        assert_eq!(kf_broker::wire::GUEST_MIN_HZ, kf_disp::pace::MIN_HZ);
        assert_eq!(
            kf_broker::ticks::BROKER_SILENCE_MS * 1_000_000,
            kf_disp::tick::BROKER_SILENCE_NS
        );
        assert_eq!(kf_broker::ticks::MAX_FRAMES_IN_FLIGHT, 2);
        assert_eq!(kf_disp::lockstep::MAX_PENDING_CREDITS, 1);
        assert_eq!(kf_broker::ticks::MAX_OPEN_REQUESTS, 1);
    }

    /// ★ §8.16 (the design's test 17): the worker arms vblanks ONLY through the pacer — a source
    /// scan of this file for period arithmetic of its own (the pre-cap `next_vblank` slots, a
    /// `Duration` built from a raster period) and for the copy gates the pacer and `NonFlip`
    /// replaced (`refresh_due`, `move_recomposes`). Known-positive: the scanner finds each pattern
    /// in a planted line. (Mutation: a helper that is never called passes every pure test; this
    /// fails when the worker computes ticks itself again.)
    #[test]
    fn the_worker_arms_vblanks_only_through_the_pacer() {
        let src = include_str!("display.rs");
        let code: String = src
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .lines()
            .map(|l| l.split("//").next().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let banned = [
            "next_vblank",
            "from_nanos(m.period_ns)",
            "period_ns / 1000",
            "fn refresh_due",
            "move_recomposes",
        ];
        let hits = |text: &str| -> Vec<&str> {
            banned
                .iter()
                .copied()
                .filter(|b| text.contains(b))
                .collect()
        };
        assert_eq!(hits(&code), Vec::<&str>::new());
        for planted in banned {
            assert_eq!(hits(&format!("let x = {planted};")), vec![planted]);
        }
        for needed in [
            "pacer.due(",
            "pacer.on_heads(",
            "pacer.next_ns()",
            "scan.nonflip.due(",
            "self.nonflip.wants_send(",
            "gpu.compose_checksum(",
        ] {
            assert!(code.contains(needed), "the worker no longer calls {needed}");
        }
    }

    /// ★ §AK: the worker's flip rules. A flip's latch queues its completions behind the copy in flight
    /// at that moment (the buffer-reuse rule) and behind nothing when none is in flight; the frame it
    /// latched is a pending flip copy until a flip-class copy starts (any: it reads the newest state);
    /// a second latch before that replaces the first; a guest life's end forgets the pending frame.
    /// (Mutations: `on_flip_latched` leaving the barrier alone; `flip_completion_need` returning the
    /// floor only; `flip_pending` ignoring the mailbox; the old `started + 1`.)
    #[test]
    fn a_flip_waits_for_the_copy_in_flight_and_a_latched_frame_is_a_pending_copy() {
        let flight = |n| Inflight {
            color: false,
            n,
            phase: Phase::Send,
            slot: 0,
            wh: (1920, 1080),
            t0: Instant::now(),
            vram: None,
            d2h: true,
            cursor: false,
            req: 0,
            summed: false,
            boot: false,
        };
        let mut scan = ScanState::default();
        assert!(!scan.flip_pending());
        // no copy in flight: the completion waits for nothing (copy 0 is done by definition)
        scan.on_flip_latched();
        assert_eq!(scan.barrier, 0, "nothing in flight, nothing to wait for");
        assert!(scan.flip_pending() && scan.mailbox.pending());
        // copy 4 starts and serves the pending frame
        scan.started = 4;
        scan.mailbox.copied();
        scan.inflight = Some(flight(4));
        assert!(!scan.flip_pending());
        // a flip latches under it: its items wait for copy 4 — not for copy 5 (its own)
        scan.on_flip_latched();
        assert_eq!(scan.barrier, 4);
        assert_eq!(scan.mailbox.latched, 2);
        assert!(
            scan.flip_pending(),
            "the frame it latched waits for its copy"
        );
        // and a second flip before the copy replaces the first, unseen; the barrier never lowers
        scan.on_flip_latched();
        assert_eq!((scan.barrier, scan.mailbox.skipped), (4, 1));
        scan.done = 4;
        scan.inflight = None;
        scan.on_flip_latched();
        assert_eq!(
            scan.barrier, 4,
            "no copy in flight: the floor stays, nothing newer is waited for"
        );
        // a modeset/colour barrier ahead of the queue is kept
        scan.barrier = scan.started + 1;
        scan.inflight = Some(flight(4));
        scan.on_flip_latched();
        assert_eq!(scan.barrier, 5);
        // a life's end forgets the pending frame
        scan.inflight = None;
        scan.reset_life();
        assert!(!scan.mailbox.pending());
        assert!(
            scan.flip_pending(),
            "reset_life wants a copy of what the new life shows"
        );
    }

    /// ★ §AK: the worker's copy bookkeeping. A flip-class copy serves the wanted copy, the pending latched frame
    /// and the tick's credit; a non-flip check serves none of them. The shown head's tick grants a copy only
    /// while a guest scanout is shown. ⊘ CORRECTED 2026-10-18 (hitch-20261017): the colour path no longer has a rule of
    /// its own (its own copy, wanted at once); every window flip waits for the copy in flight only. (Mutations:
    /// `begin_copy` not spending the mailbox, or spending it for a check; a tick granting a credit for a boot picture.)
    #[test]
    fn a_flip_copy_serves_the_pending_frame_and_a_check_does_not() {
        let mut scan = ScanState::default();
        scan.mailbox.latch();
        scan.want = true;
        assert_eq!(scan.begin_copy(true), 1, "a check is copy 1");
        assert!(
            scan.want && scan.mailbox.pending(),
            "a check serves nothing"
        );
        scan.on_console_tick(false);
        assert!(!scan.mailbox.due(false), "no credit for a boot picture");
        scan.on_console_tick(true);
        assert!(scan.mailbox.due(false));
        assert_eq!(scan.begin_copy(false), 2);
        assert!(!scan.want && !scan.mailbox.pending() && !scan.mailbox.due(false));
        assert_eq!(scan.mailbox.copies, 1);
        // one rule for every window flip, colour path included (hitch-20261017): nothing in flight, nothing to
        // wait for, and the latched frame waits for the mailbox's copy (never "its own copy, wanted at once")
        scan.barrier = 0;
        scan.want = false;
        scan.on_flip_latched();
        assert_eq!(
            (scan.barrier, scan.want, scan.mailbox.latched),
            (0, false, 2)
        );
    }

    /// ★ hitch-20261017 — **copies are bound by ticks, whatever the path that wants one.** The phase-1 rule
    /// (`V3_DISPLAY_PACING` §6.3): the newest latched frame is copied at most once per tick, so in any window of N
    /// ticks at most N + 1 flip copies start. Clock-free: ticks and latches are events, a copy completes at once.
    /// Each tick sees a flip latch just BEFORE it (an immediate flip) and one AT it (the vsync flip of the same frame,
    /// kms = 2 x tear) — the production order with the colour path on (`KF3_DISPLAY_SDR_COLOR=1`), where the
    /// colour rule wanted a copy of its own at every latch (`want`), outside the mailbox: two copies per tick,
    /// the measured `copies > tick + 2` seconds. Red on 54266072 (400 copies in 200 ticks: the colour rule), green once the colour path completes flips by the mailbox rule.
    #[test]
    fn copies_started_in_n_ticks_never_exceed_n_plus_1_on_any_latch_path() {
        const N: u64 = 200;
        let mut scan = ScanState::default();
        let mut copies = 0u64;
        let serve = |scan: &mut ScanState, copies: &mut u64| {
            while (scan.want || scan.mailbox.due(scan.inflight.is_some()))
                && scan.inflight.is_none()
            {
                scan.begin_copy(false);
                *copies += 1; // the copy completes at once: nothing stays in flight
            }
        };
        for _ in 0..N {
            scan.on_window_latch(0, true, |_| {}); // the immediate flip, between ticks (the method the worker calls)
            serve(&mut scan, &mut copies);
            scan.on_console_tick(true); // the tick: one copy credit
            scan.on_window_latch(0, true, |_| {}); // the vsync flip latched at the tick
            serve(&mut scan, &mut copies);
        }
        assert!(
            copies <= N + 1,
            "{copies} flip copies started in {N} ticks (bound {}): a latch path copies outside the tick",
            N + 1
        );
        assert!(
            copies >= N,
            "{copies} copies in {N} ticks, each with a latched frame: the mailbox lost a frame"
        );
    }

    fn flight(n: u64) -> Inflight {
        Inflight {
            color: false,
            n,
            phase: Phase::Send,
            slot: 0,
            wh: (1920, 1080),
            t0: Instant::now(),
            vram: None,
            d2h: true,
            cursor: false,
            req: 0,
            summed: false,
            boot: false,
        }
    }

    /// ★ review hitch-20261017 (1): what the worker does at a latch, through the method the worker calls. A window the
    /// console shows: the flip's items wait for the copy in flight at the latch and for nothing else (never its own copy,
    /// never a wanted one), the frame is a pending mailbox copy, the bindings are forgotten and `resolve` runs exactly
    /// once. A window it does not show: the bindings are forgotten, nothing else moves, `resolve` does not run.
    /// (Mutations killed here and by the call-site scans: M1 the colour rule back (`barrier = started + 1`, `want`),
    /// M2 `want = true` after the call, M3 `flip_completion_need(barrier, None)`, M4 no `mailbox.latch()`, M5 no `resolve`,
    /// M6 no `forget`.)
    #[test]
    fn the_latch_method_waits_for_the_copy_in_flight_and_runs_the_resolutions_once() {
        let mut scan = ScanState {
            started: 4,
            inflight: Some(flight(4)),
            ..ScanState::default()
        };
        let mut ran = 0;
        scan.on_window_latch(3, true, |_| ran += 1);
        assert_eq!(ran, 1, "the resolutions run once per shown latch");
        assert_eq!(
            scan.barrier, 4,
            "the flip waits for the copy in flight (4), not for copy 5"
        );
        assert!(!scan.want, "a latch wants no copy of its own");
        assert!(
            scan.mailbox.pending() && scan.flip_pending(),
            "the newest frame is a pending mailbox copy"
        );
        // nothing in flight: nothing to wait for
        let mut idle = ScanState {
            started: 9,
            ..ScanState::default()
        };
        idle.on_window_latch(3, true, |_| {});
        assert_eq!((idle.barrier, idle.want), (0, false));
        // a window the console does not show: only the forgetting
        let mut other = ScanState::default();
        let dma = CtxDma {
            target: kf_disp::inst::Target::Vidmem,
            base: 0x1000_0000,
            limit: 0x1000_ffff,
            block_linear: false,
            writable: false,
        };
        other.latched.by_window[3] = Some(((1, 2, 3), dma));
        other.colors.slots[3] = Some((
            (
                1,
                3,
                1,
                kf_disp::color::Lut {
                    entries: 1025,
                    binding: kf_disp::color::Binding::Vidmem(0),
                    interpolate: false,
                    mirror: false,
                },
            ),
            kf_cuda::display::ColorLut {
                entries: 1025,
                src: 0,
                interpolate: false,
                mirror: false,
                token: 1,
            },
        ));
        other.colors.slots[32 + 3] = other.colors.slots[3];
        other.on_window_latch(3, false, |_| panic!("an un-shown window resolves nothing"));
        assert!(
            other.latched.by_window[3].is_none()
                && other.colors.slots[3].is_none()
                && other.colors.slots[35].is_none(),
            "the bindings are forgotten at every latch"
        );
        assert_eq!(
            (other.barrier, other.want, other.mailbox.latched),
            (0, false, 0),
            "nothing else moves"
        );
    }

    /// ★ review hitch-20261017 (3): the colour bindings resolved AT THE LATCH are what the copy a tick later uses. A guest
    /// that idles and unbinds the LUT context DMA (NVKMS teardown) before the tick must not make the copy fail; without the
    /// latch-time resolution (the d6516e296 behaviour) it does (`Fault::Console`, console-only). A new latch re-resolves.
    #[test]
    fn colour_bindings_resolved_at_the_latch_survive_an_unbind_before_the_copy() {
        let lut = kf_disp::color::Lut {
            entries: 1025,
            binding: kf_disp::color::Binding::Dma {
                handle: 7,
                offset: 0,
            },
            interpolate: false,
            mirror: false,
        };
        let dma = CtxDma {
            target: kf_disp::inst::Target::Vidmem,
            base: 0x1000_0000,
            limit: 0x1001_ffff,
            block_linear: false,
            writable: false,
        };
        let mut colors = ColorDmas::default();
        // the latch: resolve (what resolve_window_colors does through the same call)
        let at_latch = colors
            .resolve(0, 1, 2, 3, Some(lut), |_| Ok(dma))
            .unwrap()
            .unwrap();
        // the guest unbinds the context DMA; the copy a tick later resolves again and must find it cached
        let at_copy = colors
            .resolve(0, 1, 2, 3, Some(lut), |_| {
                Err("context DMA unbound".to_owned())
            })
            .expect("the copy uses the binding resolved at the latch")
            .unwrap();
        assert_eq!(
            at_latch.token, at_copy.token,
            "the very binding of the latch"
        );
        // without a latch-time resolution the same unbind fails the copy (the hole)
        let mut late = ColorDmas::default();
        assert!(
            late.resolve(0, 1, 2, 3, Some(lut), |_| Err(
                "context DMA unbound".to_owned()
            ))
            .is_err()
        );
        // a later latch forgets and resolves afresh (new armed state, new token)
        colors.forget_window(0);
        let again = colors
            .resolve(0, 1, 2, 3, Some(lut), |_| Ok(dma))
            .unwrap()
            .unwrap();
        assert_ne!(again.token, at_copy.token);
    }

    /// ★ review hitch-20261017 (1): the pieces of the latch that no behavioural test can see because they are wiring: the
    /// worker passes "is this window shown" from the layer lookup (`so.is_some()`), resolves the window context DMA
    /// (`scan.resolve_now`) and, on the colour path, the LUT/TMO bindings (`resolve_window_colors`) inside the call; and
    /// `ScanState::on_window_latch` itself has its four steps — forget the DMA binding, forget the colour bindings, the one
    /// flip rule, then the resolutions. (Mutations: M5 resolutions not run, M6 bindings not forgotten, M7 a call site that
    /// always says "not shown".) Known-positive: the checker finds a planted token and misses a planted absence.
    #[test]
    fn the_latch_wiring_has_its_steps_at_the_call_site_and_in_the_method() {
        let code = worker_code();
        let has_all = |text: &str, needs: &[&str]| needs.iter().all(|n| text.contains(n));
        assert!(has_all("a b c", &["a", "c"]) && !has_all("a b c", &["a", "z"]));
        let block = code
            .split("if let Effect::Latched { window } = &e")
            .nth(1)
            .unwrap()
            .split("queue.push_back")
            .next()
            .unwrap();
        for needed in [
            "scan.on_window_latch(*window, so.is_some(),",
            "scan.resolve_now(",
            "resolve_window_colors(",
            "c.layers.iter().find(|l| l.window == *window)",
        ] {
            assert!(
                block.contains(needed),
                "the latch call site lost {needed}: {block}"
            );
        }
        let body = code
            .split("fn on_window_latch(")
            .nth(1)
            .unwrap()
            .split("\n    }\n")
            .next()
            .unwrap();
        for needed in [
            "self.latched.forget(window)",
            "self.colors.forget_window(window as usize)",
            "self.on_flip_latched()",
            "resolve(self)",
        ] {
            assert!(
                body.contains(needed),
                "ScanState::on_window_latch lost {needed}: {body}"
            );
        }
        // order: forget, forget, rule, resolve
        let at = |n: &str| body.find(n).unwrap();
        assert!(
            at("self.latched.forget") < at("self.colors.forget_window")
                && at("self.colors.forget_window") < at("self.on_flip_latched()")
                && at("self.on_flip_latched()") < at("resolve(self)")
        );
    }

    /// ★ review hitch-20261017 (1): `scan.want = true` appears at the two named sites of the worker only (a console change,
    /// a core colour update) — a third one, at the latch or anywhere, is the original defect. Known-positive: the counter
    /// sees a planted line.
    #[test]
    fn the_worker_sets_want_at_the_two_named_sites_only() {
        let n = |t: &str| t.matches("scan.want = true").count();
        assert_eq!(n("a; scan.want = true; b; scan.want = true"), 2);
        assert_eq!(
            n(&worker_code()),
            2,
            "scan.want = true appears at an unnamed site of the worker"
        );
    }

    /// ★ integration (prod-candidate-5, 2026-10-18) — **hitch x bootlayer: a device reset that meets a colour-path
    /// copy in flight neither strands a flip nor shows a tainted boot layer.** GPU-free, at the pure-state level
    /// (the worker itself needs a GPU). Hitch moved the colour path onto the mailbox rule (a latch sets
    /// `barrier = flip_completion_need(..)`, never `started + 1`); bootlayer gates the boot layer on a clean
    /// store and taints it at the reset request. The pieces, in the order the reset runs them:
    /// (1) the worker ends the life only with NO copy in flight (a source check: the guard is in the code);
    /// (2) the copy in flight completes, and its flip's barrier (= that copy) is met by it;
    /// (3) `reset_life` leaves `barrier == done == started` — nothing in the new life waits for a copy that will
    ///     never be made — and the new life's first flip, latched with nothing in flight, waits for nothing;
    /// (4) the shown choice, with the range still tainted (the memory stage's scrub has not completed), is
    ///     never the boot layer, and clean again (scrub completed) it is.
    /// (Mutations expected to be caught: the life-reset guard `scan.inflight.is_none()` dropped; `request_life_reset`
    /// not tainting or tainting after the request. NOT caught: `reset_life` leaving `barrier` alone, because here the
    /// barrier already equals `started` -- the pure state cannot distinguish.)
    #[test]
    fn a_reset_during_a_colour_path_copy_strands_no_flip_and_shows_no_tainted_boot_layer() {
        let flight = |n| Inflight {
            color: true, // the colour path: the one hitch moved from `started + 1` onto the mailbox rule
            n,
            phase: Phase::Send,
            slot: 0,
            wh: (1920, 1080),
            t0: Instant::now(),
            vram: None,
            d2h: true,
            cursor: false,
            req: 0,
            summed: false,
            boot: false,
        };
        // A colour-path copy 7 is in flight; a flip latches under it.
        let mut scan = ScanState {
            started: 7,
            done: 6,
            inflight: Some(flight(7)),
            ..ScanState::default()
        };
        scan.on_window_latch(0, true, |_| {});
        assert_eq!(
            scan.barrier, 7,
            "the colour path waits for the copy in flight, as every path"
        );
        assert!(
            scan.mailbox.pending(),
            "the latched frame waits for the mailbox's copy"
        );
        // (1) the life must not end under the copy: the worker's guard is in the code
        let src = include_str!("display.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let guard = code
            .split("let owed = dp.life_reset.owed();")
            .nth(1)
            .and_then(|s| s.split("scan.reset_life();").next())
            .expect("the worker's life-reset block");
        assert!(
            guard.contains("&& scan.inflight.is_none()"),
            "the life ends only with no copy in flight: {guard}"
        );
        // (2) the copy completes: its flip is released (done >= barrier)
        scan.done = 7;
        scan.inflight = None;
        assert!(scan.done >= scan.barrier);
        // (3) the life ends
        scan.reset_life();
        assert_eq!((scan.barrier, scan.done, scan.started), (7, 7, 7));
        assert!(
            scan.barrier <= scan.done,
            "no completion of the new life waits for a copy not yet made"
        );
        assert!(
            !scan.mailbox.pending(),
            "the old life's latched frame is forgotten"
        );
        assert!(scan.want, "the new life wants its first copy at once");
        // the new life's first flip, nothing in flight: waits for nothing, and nothing is stranded
        scan.on_window_latch(0, true, |_| {});
        assert!(
            scan.barrier <= scan.done,
            "{} > {}",
            scan.barrier,
            scan.done
        );
        // (4) the boot layer: tainted at the request (`request_life_reset`), clean once the scrub completed
        let rl = code
            .split("pub fn request_life_reset(")
            .nth(1)
            .and_then(|s| s.split("\n    }\n").next())
            .expect("request_life_reset");
        assert!(
            rl.find("self.boot_clean.taint()")
                .is_some_and(|t| t < rl.find("self.life_reset.request(").unwrap()),
            "the request taints BEFORE it can be acknowledged: {rl}"
        );
        let boot = boot_scan();
        let held = held_after_life_reset(&Held {
            preserved: None,
            scanned: Some((1920, 1080)),
            dark: Dark::Unlit,
        });
        assert_ne!(
            choose_shown_gated(None, Some(&boot), false, false, &held),
            Some(Shown::Boot(boot.layer, boot.size)),
            "tainted: never the old life's bytes"
        );
        assert_eq!(
            choose_shown_gated(None, Some(&boot), false, true, &held),
            Some(Shown::Boot(boot.layer, boot.size)),
            "clean: the boot layer is back"
        );
    }

    /// CODE READ + pure state: a console colour refusal is not evidence of guest writes.
    #[test]
    fn a_console_colour_refusal_does_not_taint_the_boot_store() {
        let clean = crate::bootclean::BootClean::new();
        let counters = DispCounters::with_step_meters(true);
        let console = ConsoleShare::default();
        let mut scan = ScanState {
            barrier: 1,
            started: 1,
            ..ScanState::default()
        };
        scan.fault(
            &counters,
            &console,
            1,
            0,
            Fault::Console("SDR colour program: refused".into()),
        );
        assert_eq!(counters.console_copies_skipped.load(Ordering::Relaxed), 1);
        assert!(clean.is_clean());
        let boot = boot_scan();
        assert_eq!(
            choose_shown_gated(None, Some(&boot), false, clean.is_clean(), &Held::default()),
            Some(Shown::Boot(boot.layer, boot.size))
        );
        clean.begin_life();
        assert!(!clean.is_clean(), "an actual RM life taints the range");
        assert_eq!(
            choose_shown_gated(None, Some(&boot), false, clean.is_clean(), &Held::default()),
            None
        );
        // Pin this compositional proof to the actual worker input and the real gate.
        let code = include_str!("display.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let choice = code
            .split("let shown = choose_shown_gated(")
            .nth(1)
            .unwrap()
            .split(");")
            .next()
            .unwrap();
        assert!(choice.contains("dp.boot_clean.is_clean()"));
        assert!(!choice.contains("console_copies_skipped") && !choice.contains("scanout_refused"));
        let gate = include_str!("bootclean.rs");
        assert!(!gate.contains("console_copies_skipped") && !gate.contains("scanout_refused"));
    }

    /// Reused identifiers in a new life must resolve a fresh framebuffer binding.
    #[test]
    fn a_life_reset_drops_the_latched_scanout_binding_before_identifiers_are_reused() {
        let mut scan = ScanState::default();
        let so = scanout_6();
        let old = CtxDma {
            target: Target::Vidmem,
            base: 0,
            limit: 0x7f_ffff,
            block_linear: false,
            writable: true,
        };
        scan.on_window_latch(so.window, true, |state| {
            assert_eq!(
                state.latched.resolve(&so, || Ok(old)).unwrap().base,
                old.base
            );
        });
        assert_eq!(
            scan.latched
                .resolve(&so, || Err("unbound after latch".into()))
                .unwrap()
                .base,
            old.base,
            "the same life keeps its latch-time binding"
        );
        scan.reset_life();
        assert!(scan.latched.by_window.iter().all(Option::is_none));
        let new = CtxDma {
            base: 0x1000_0000,
            limit: 0x107f_ffff,
            ..old
        };
        let mut resolutions = 0;
        let got = scan
            .latched
            .resolve(&so, || {
                resolutions += 1;
                Ok(new)
            })
            .unwrap();
        assert_eq!(
            resolutions, 1,
            "the next life resolves the reused identifiers afresh"
        );
        assert_eq!(got.base, new.base);
        assert_ne!(got.base, old.base);
    }

    /// Pure-state integration: active bounded diagnostics do not participate in reset decisions.
    #[test]
    fn life_resets_with_event_recording_and_step_stats_remain_bounded() {
        use kf_broker::ticks::{RING_CAP, RING_DUMPS_MAX, Requests};
        let mut requests = Requests::with_diagnostics(true);
        let meters = DispCounters::with_step_meters(true);
        let mut scan = ScanState::default();
        let ack = crate::reset::Ack::default();
        let mut dumps = 0;
        for seq in 1..=4096 {
            requests.on_tick(&kf_broker::wire::TickPkt {
                tick_seq: seq,
                refresh_ns: 16_666_666,
                want: true,
                clock_only: false,
                present_ns: u64::from(seq),
            });
            requests.note_live(false);
            let before = requests.probe_state();
            ack.request(u64::from(seq));
            scan.reset_life();
            ack.done(u64::from(seq));
            assert_eq!(
                requests.probe_state(),
                before,
                "reset does not consume or wait for a broker request"
            );
            meters.lat_pass.add(Duration::from_micros(1));
            if let Some(lines) = requests.ring.take_dump("reset integration") {
                dumps += 1;
                assert!(lines.len() <= RING_CAP + 1);
            }
            assert!(requests.ring.events().len() <= RING_CAP);
            assert!(ack.owed().is_none());
        }
        assert_eq!(requests.c.ticks, 4096);
        assert_eq!(requests.ring.events().len(), RING_CAP);
        assert_eq!(dumps, RING_DUMPS_MAX);
        assert!(requests.ring.suppressed > 0);
        assert_eq!(requests.spend(4096), Some(4096));
        assert_eq!(
            requests.spend(4096),
            None,
            "the request is still spent exactly once"
        );
        assert!(meters.step_fragment().contains("pass=4096/4096/1"));
    }

    /// ★ §AK: the worker no longer carries the D1 gate or its measurement flag, parks no flip for a
    /// tick, and wires the mailbox exactly once each way: a latch of the shown window calls
    /// `on_flip_latched` (not the old `started + 1` barrier), the shown head's tick grants a copy, and
    /// a copy starts when the mailbox says so. Known-positive: the scanner finds each pattern in a
    /// planted line. (Mutations: the flag back; the old barrier back; `mailbox.tick()` dropped.)
    #[test]
    fn the_worker_is_wired_to_the_mailbox_and_has_no_tear_gate() {
        let src = include_str!("display.rs");
        let code: String = src
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .lines()
            .map(|l| l.split("//").next().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let banned = ["tear_gate", "KF3_TEAR_GATE", "tear_held", "held: p."];
        let hits = |text: &str| -> Vec<&str> {
            banned
                .iter()
                .copied()
                .filter(|b| text.contains(b))
                .collect()
        };
        assert_eq!(hits(&code), Vec::<&str>::new());
        for planted in banned {
            assert_eq!(hits(&format!("let x = {planted};")), vec![planted]);
        }
        for needed in [
            "scan.on_window_latch(*window, so.is_some(),",
            "resolve_window_colors(&mut scan.colors",
            "self.on_flip_latched()",
            "scan.on_console_tick(",
            "self.mailbox.tick()",
            "scan.mailbox.due(",
            "self.begin_copy(check)",
            "self.mailbox.copied()",
            "self.barrier = flip_completion_need(",
            "self.mailbox.pending()",
            "scan.flip_pending()",
            "pacer.take_handoffs()",
        ] {
            assert!(
                code.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .contains(
                        &needed
                            .chars()
                            .filter(|c| !c.is_whitespace())
                            .collect::<String>()
                    ),
                "the worker no longer calls {needed}"
            );
        }
        // the latch of the shown window sets no barrier of its own: the rule is one function
        let latch = code
            .split("if let Effect::Latched { window } = &e")
            .nth(1)
            .unwrap();
        let latch = latch.split("queue.push_back").next().unwrap();
        assert!(!latch.contains("barrier"), "{latch}");
    }

    /// The worker code with comments stripped (what the call-site scans look at).
    fn worker_code() -> String {
        include_str!("display.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .lines()
            .map(|l| l.split("//").next().unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// ★ review hitch-20261017 (1): the latch of a window is ONE call into `ScanState` — the property tests exercise
    /// that method, so the worker's call site must hold nothing but the call and the resolutions it passes in. A
    /// `scan.want = true`, a `started + 1`, a barrier or a mailbox touch between `Effect::Latched` and the queueing of
    /// its effect would bring the original defect back where no property test sees it. Known-positive: the scanner
    /// finds each banned token in a planted block. EXPECTED RED on the commit that adds it (the call site still sets
    /// things inline); green with the next.
    #[test]
    fn the_worker_latch_is_one_method_call_and_touches_no_copy_state() {
        let code = worker_code();
        let block = code
            .split("if let Effect::Latched { window } = &e")
            .nth(1)
            .unwrap()
            .split("queue.push_back")
            .next()
            .unwrap()
            .to_owned();
        let banned = [
            ".want",
            "started",
            "barrier",
            "mailbox",
            "inflight",
            "begin_copy",
            ".start(",
            "latched.forget",
            "colors.forget",
        ];
        let hits = |text: &str| -> Vec<&str> {
            banned
                .iter()
                .copied()
                .filter(|b| text.contains(b))
                .collect()
        };
        for planted in banned {
            assert_eq!(hits(&format!("scan{planted} = 1;")), vec![planted]);
        }
        assert_eq!(
            hits(&block),
            Vec::<&str>::new(),
            "the latch call site touches copy state: {block}"
        );
        assert!(
            block.contains("scan.on_window_latch("),
            "the worker no longer calls ScanState::on_window_latch at a latch: {block}"
        );
    }

    fn map() -> RegMap {
        let r = Regs::for_ip("580.159.04", 0x0401_0000).unwrap();
        let t = kf_disp::class::for_version("580.159.04").unwrap();
        RegMap::resolve(&r, t, &kf_chip::display::AMPERE).expect("GA10x register map")
    }

    /// ⚠⚠ H-caps probe (default off): the measured page is published only for the caps class and
    /// page base it was measured with (Ada's C773 at `0x640000`, VFIO DVI reference boot3, RTX 4070,
    /// 2026-10-08); its words are in the page, 4-byte aligned, distinct, and it presents the same
    /// engine as kf3's authored page (`SYS_CAP` heads 0-3 + SORs 0-3, `SYS_CAPB` windows 0-7) — what
    /// differs is the per-window / per-head / per-SOR capabilities only.
    #[test]
    fn the_caps_probe_page_is_the_measured_one_for_its_class_only() {
        assert!(
            caps_probe_page(0x0064_0000, 0xC673).is_none(),
            "another class"
        );
        assert!(
            caps_probe_page(0x0064_1000, 0xC773).is_none(),
            "another base"
        );
        let p = caps_probe_page(0x0064_0000, 0xC773).expect("the measured class");
        let mut seen = std::collections::BTreeSet::new();
        for (o, _) in &p.words {
            assert!(*o < 0x1000 && o % 4 == 0 && seen.insert(*o), "{o:#x}");
        }
        let t = kf_disp::class::for_version("580.65.06").unwrap();
        let r = Regs::for_ip("580.65.06", kf_chip::display::ADA.ip_version).unwrap();
        let authored = kf_disp::caps::sdr_page(t, &r, 0xC773, 0xC77D, 4, 8).expect("SDR page");
        assert_eq!(authored.base, p.base);
        assert_eq!(p.word(0), authored.word(0), "SYS_CAP");
        assert_eq!(p.word(4), authored.word(4), "SYS_CAPB");
        // the window-capability difference the record names: even windows scaler + TMO, odd not
        assert_ne!(p.word(0x780), p.word(0x7a0));
        assert_eq!(authored.word(0x780), authored.word(0x7a0));
    }

    /// ★ 2026-10-14 — THE SCAN LINE IS LIVE. The words the Windows driver reads per vblank and per
    /// `D3DKMTGetScanLine` (`[measured run 2990, BAR0 read trace]` `0x680220` = core
    /// `GET_RG_SCAN_LINE(0)`, `0x690208` = window 0 `GET_LINE`) are derived at those offsets on Ada and
    /// Ampere; a frame edge reads VBLANK at line `BLANK_START.Y`, the raster then moves and leaves the
    /// blank, a window follows its head's line, and nothing is published before a raster runs.
    #[test]
    fn the_scan_line_words_are_live_at_the_derived_offsets() {
        use kf_disp::ports::ScanGeom;
        let g = ScanGeom {
            blank_end_y: 41,
            blank_start_y: 1121,
            total_h: 1125,
            period_ns: 16_666_667,
        };
        for (ver, ip, row) in [
            ("580.65.06", 0x0404_0000, &kf_chip::display::ADA),
            ("580.159.04", 0x0401_0000, &kf_chip::display::AMPERE),
        ] {
            let r = Regs::for_ip(ver, ip).unwrap();
            let t = kf_disp::class::for_version(ver).unwrap();
            let m = RegMap::resolve(&r, t, row).expect("register map");
            let p = Ports::default();
            assert!(
                m.scan_words(&p, 10, None).is_empty(),
                "{ver}: no raster, no words"
            );
            p.set_scan_geom(0, Some(g));
            assert!(
                m.scan_words(&p, 10, None).is_empty(),
                "{ver}: no frame edge yet"
            );
            p.note_frame_edge_at(0, 1_000);
            p.set_window_head(0, Some(0));
            let at_edge = m.scan_words(&p, 1_000, None);
            assert_eq!(
                at_edge,
                vec![(0x0068_0220, 0x0001_0461), (0x0069_0208, 0x461)],
                "{ver}"
            );
            let mid = m.scan_words(&p, 1_000 + g.period_ns / 2, None);
            let core = mid.iter().find(|(o, _)| *o == 0x0068_0220).unwrap().1;
            assert_eq!(
                core & 0x0001_0000,
                0,
                "{ver}: mid-frame is not in the blank"
            );
            assert!((41..1121).contains(&(core & 0xffff)), "{ver}: {core:#x}");
            assert_eq!(
                mid.iter().find(|(o, _)| *o == 0x0069_0208).unwrap().1,
                core & 0xffff
            );
            // another head's edge leaves head 0's words out of a per-head publication
            assert!(m.scan_words(&p, 5_000, Some(1)).is_empty());
            // a window with no owner gets no word
            p.set_window_head(0, None);
            assert_eq!(
                m.scan_words(&p, 1_000, None),
                vec![(0x0068_0220, 0x0001_0461)]
            );
            assert_eq!(
                m.classify(0x0068_0220),
                DispWrite::Plain,
                "{ver}: core user area"
            );
        }
    }

    /// ⚠ H-blankstate / H-armeddefault (default off): a new core life publishes nothing extra with
    /// both off; under the experiments, the hardware's measured read-backs at the derived offsets —
    /// on Ada (the 580.88 guest's C77D core, the guest-driver row `580.65.06`) `0x680240 + 4h` reads
    /// BLANK (`0x1`) and the ARMED `0x68a218 + 0x400h` reads `0x10002`, the values the RTX 4070
    /// returned for its unlit heads / every head (VFIO DVI reference boot3). Each stored word is
    /// inside the core user area, the BLANK one in the guest-writable half (the guest overwrites it).
    #[test]
    fn the_core_birth_words_are_the_hardware_read_backs_only_under_the_experiments() {
        for (ver, ip, row) in [
            ("580.65.06", 0x0404_0000, &kf_chip::display::ADA),
            ("580.159.04", 0x0401_0000, &kf_chip::display::AMPERE),
        ] {
            let r = Regs::for_ip(ver, ip).unwrap();
            let t = kf_disp::class::for_version(ver).unwrap();
            let m = RegMap::resolve(&r, t, row).expect("register map");
            assert!(
                m.core_birth_words(false, false).is_empty(),
                "{ver}: off by default"
            );
            let blank = m.core_birth_words(true, false);
            let armed = m.core_birth_words(false, true);
            let heads = u64::from(m.heads);
            assert_eq!(blank.len() as u64, heads);
            assert_eq!(armed.len() as u64, heads);
            for h in 0..heads {
                assert_eq!(
                    blank[h as usize],
                    (0x0068_0240 + 4 * h, 0x1),
                    "{ver} head {h}"
                );
                assert_eq!(
                    armed[h as usize],
                    (0x0068_a218 + 0x400 * h, 0x0001_0002),
                    "{ver} head {h}"
                );
                assert_eq!(m.classify(blank[h as usize].0), DispWrite::Plain);
                assert_eq!(
                    m.classify(armed[h as usize].0),
                    DispWrite::ReadOnly,
                    "ARMED half"
                );
            }
            assert_eq!(m.core_birth_words(true, true).len() as u64, 2 * heads);
        }
    }

    /// ★ The vCPU decode on GA10x: each DMA channel's PUT, its read-only GET and ARMED half, the
    /// cursor PIO window and its read-only `Free`, the W1C event registers, the derived summaries.
    #[test]
    fn the_vcpu_decode_is_the_derived_register_file() {
        let m = map();
        assert!(
            m.owns(0x0068_0000)
                && m.owns(0x0061_1C30)
                && !m.owns(0x0070_0000)
                && !m.owns(0x0060_FFFC)
        );
        assert_eq!(m.classify(0x0068_0000), DispWrite::Put(0));
        assert_eq!(
            m.classify(0x0068_0004),
            DispWrite::ReadOnly,
            "GET is the engine's"
        );
        assert_eq!(
            m.classify(0x0068_8000 + 0x1000),
            DispWrite::ReadOnly,
            "the ARMED half"
        );
        assert_eq!(m.classify(0x0069_3000), DispWrite::Put(4), "window 3");
        assert_eq!(
            m.classify(0x0069_8000),
            DispWrite::Plain,
            "window 8 does not exist"
        );
        assert_eq!(
            m.classify(0x006B_7000),
            DispWrite::Put(33 + 7),
            "window-immediate 7"
        );
        assert_eq!(m.classify(0x006D_9208), DispWrite::Cursor(1, 0x208));
        assert_eq!(m.classify(0x006D_9008), DispWrite::ReadOnly, "cursor Free");
        assert_eq!(
            m.classify(0x0061_1858),
            DispWrite::Event(EventReg::AwakenWin)
        );
        assert_eq!(
            m.classify(0x0061_185C),
            DispWrite::Event(EventReg::AwakenOther)
        );
        assert_eq!(m.classify(0x0061_1868), DispWrite::Event(EventReg::SemWin));
        assert_eq!(
            m.classify(0x0061_1804),
            DispWrite::Event(EventReg::HeadTiming(1))
        );
        assert_eq!(
            m.classify(0x0061_1D8C),
            DispWrite::Event(EventReg::HeadTimingEn(3))
        );
        assert_eq!(m.classify(0x0061_1C30), DispWrite::ReadOnly);
        assert_eq!(m.classify(0x0061_1EC0), DispWrite::ReadOnly);
        assert_eq!(m.classify(0x0061_2078), DispWrite::Plain);
    }

    /// ★ 2026-10-08 (H-flip, the VFIO DVI reference): the head-timing interrupt follows the frame
    /// edges while the guest keeps LAST_DATA enabled, and only then. Replays Windows' measured order
    /// on hardware (boot3, 113/113 enables): read `0x611800`, write-1-clear LAST_DATA, enable — no
    /// interrupt at the enable; one at EVERY following frame edge while enabled (the ISR clears it
    /// each time); none once the guest disabled it, although the event keeps latching.
    #[test]
    fn the_head_timing_interrupt_is_every_frame_edge_while_enabled_and_never_at_the_enable() {
        let m = map();
        let p = Ports::default();
        let ld = m.head_last_data;
        assert_eq!(ld, 0x2, "LAST_DATA is bit 1 (dev_disp.h v03_00)");
        // frames before the guest enables anything: pending, but nothing reaches RM
        for _ in 0..3 {
            assert_eq!(frame_edge(&p, &m, 0, false), 0, "disabled: no interrupt");
        }
        assert_eq!(p.event(EventReg::HeadTiming(0)) & ld, ld, "still latched");
        // Windows' enable: clear, then enable — the stale latch must not fire at the enable
        p.guest_write(EventReg::HeadTiming(0), ld);
        p.guest_write(EventReg::HeadTimingEn(0), 0x003f_0062);
        assert_eq!(p.rm_head_timing(0), 0, "no early interrupt at the enable");
        assert_eq!(p.rm_dispatch(4), 0);
        // every frame edge while enabled raises; the ISR's W1C re-arms it for the next frame
        for frame in 0..5 {
            assert_eq!(
                frame_edge(&p, &m, 0, false),
                ld,
                "frame {frame}: LAST_DATA reaches RM"
            );
            assert_eq!(p.rm_dispatch(4), 1, "head 0 is the pending head");
            p.guest_write(EventReg::HeadTiming(0), ld);
            assert_eq!(p.rm_head_timing(0), 0, "cleared until the next edge");
        }
        // the disable is honoured at once, and at every later edge
        p.guest_write(EventReg::HeadTimingEn(0), 0x003f_0060);
        for _ in 0..3 {
            assert_eq!(frame_edge(&p, &m, 0, false), 0, "disabled: no interrupt");
        }
        // another head's edge never raises head 0
        p.guest_write(EventReg::HeadTimingEn(1), 0);
        assert_eq!(frame_edge(&p, &m, 1, false), 0);
    }

    /// ★ H-loadv (`KF3_DISPLAY_LOADV`): the guest reads `EVT_STAT_HEAD_TIMING` as the hardware
    /// showed it — `0x7` at the ISR, `0x5` after its write-1-clear of `0x2` — and LOADV, which no
    /// enable Windows writes covers, never raises an interrupt by itself.
    #[test]
    fn loadv_reads_as_the_hardware_read_it_and_raises_nothing_by_itself() {
        let m = map();
        let p = Ports::default();
        assert_eq!(frame_edge(&p, &m, 0, true), 0, "nothing enabled yet");
        assert_eq!(
            p.event(EventReg::HeadTiming(0)),
            0x7,
            "hardware: 0x611800 = 0x7"
        );
        p.guest_write(EventReg::HeadTiming(0), 0x2);
        p.guest_write(EventReg::HeadTimingEn(0), 0x003f_0062);
        assert_eq!(
            p.event(EventReg::HeadTiming(0)),
            0x5,
            "hardware: 0x5 after the clear"
        );
        assert_eq!(p.rm_head_timing(0), 0, "LOADV and VBLANK are not enabled");
        assert_eq!(
            frame_edge(&p, &m, 0, true),
            0x2,
            "only LAST_DATA reaches RM"
        );
        assert_eq!(p.event(EventReg::HeadTiming(0)), 0x7);
        // without the experiment the register reads what kf3 published before it
        let q = Ports::default();
        let _ = frame_edge(&q, &m, 0, false);
        assert_eq!(q.event(EventReg::HeadTiming(0)), 0x6);
    }

    fn frame(addr: usize, serial: u64) -> FrameView {
        FrameView {
            addr,
            width: 1920,
            height: 1080,
            stride: 7680,
            format: 1,
            serial,
            cursor: false,
        }
    }

    fn boot_scan() -> BootScan {
        BootScan {
            layer: kf_disp::scanout::boot_layer(&kf_disp::scanout::BootSurface {
                width: 1920,
                height: 1080,
                pitch: 7680,
                bytes: 0x7F_0000,
            })
            .unwrap(),
            size: (1920, 1080),
        }
    }

    /// Window 6 of head 3 scanning a 1080p pitch surface (b1f's console window).
    fn scanout_6() -> kf_disp::engine::Scanout {
        kf_disp::engine::Scanout {
            window: 6,
            head: 3,
            chn: 7,
            client: 0xc1d0_0001,
            handle: 0x1_0088,
            offset: 0,
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
            surface_width: 1920,
            surface_height: 1080,
            pitch: 120,
            block_height_log2: 0,
            format: 0xe6,
            out_x: 0,
            out_y: 0,
            out_width: 1920,
            out_height: 1080,
            iso1: 0,
            offset1: 0,
            pitch1: 0,
            swap_uv: false,
            depth: 0,
            k1: 255,
            k2: 0,
            src_factor: 0,
            dst_factor: 0,
            fmt: [0; 12],
        }
    }

    /// ★ The boot display: the boot layer until the first armed head — also on a family with no
    /// window vocabulary (no composition at all) — and never again; without it, today's choice.
    #[test]
    fn the_boot_layer_shows_until_the_first_armed_head_and_never_again() {
        let comp = Composition {
            head: 0,
            width: 1920,
            height: 1080,
            layers: Vec::new(),
        };
        let boot = BootScan {
            layer: kf_disp::scanout::boot_layer(&kf_disp::scanout::BootSurface {
                width: 1920,
                height: 1080,
                pitch: 7680,
                bytes: 0x7F_0000,
            })
            .unwrap(),
            size: (1920, 1080),
        };
        let armed = Some(Shown::Armed(comp.clone()));
        let fresh = Held::default();
        // ⊘ gop=off, before any frame: exactly today's choice
        assert_eq!(choose_shown(None, None, false, &fresh), None);
        assert_eq!(choose_shown(None, None, true, &fresh), None);
        assert_eq!(choose_shown(Some(comp.clone()), None, false, &fresh), armed);
        // gop=on, before any head: the boot layer (with or without a window vocabulary)
        assert_eq!(
            choose_shown(None, Some(&boot), false, &fresh),
            Some(Shown::Boot(boot.layer, (1920, 1080)))
        );
        // an armed composition always wins, and once a head was armed the boot layer never returns
        assert_eq!(
            choose_shown(Some(comp.clone()), Some(&boot), false, &fresh),
            armed
        );
        assert_eq!(choose_shown(Some(comp), Some(&boot), true, &fresh), armed);
        assert_eq!(choose_shown(None, Some(&boot), true, &fresh), None);
    }

    /// ★★ 2026-10-13 (owner requirement, `docs/design/V3_DEVICE_RESET.md` §6): when a guest life
    /// ends — the guest's RM unloads (fn 47 without a PM transition), or the device is reset — the
    /// console goes back to the BOOT LAYER (store `[0, G)`, what BAR1 `[0, G)` shows again), from
    /// every state the old life left: an armed scanout lost, a `PRESERVE_HW` scanout, black. Without
    /// the boot display (`gop=off`) a scanout that was shown is black, never the old life's last
    /// frame; one that never was stays QEMU's placeholder (no frame). The next life's first armed
    /// head takes over as on the first boot.
    #[test]
    fn a_guest_life_that_ends_shows_the_boot_layer_again_from_every_state() {
        let boot = boot_scan();
        let comp = Composition {
            head: 0,
            width: 1920,
            height: 1080,
            layers: Vec::new(),
        };
        let states = [
            Held::default(),
            Held {
                preserved: None,
                scanned: Some((1280, 720)),
                dark: Dark::Unlit,
            },
            Held {
                preserved: Some((vec![boot.layer], (1920, 1080))),
                scanned: Some((1920, 1080)),
                dark: Dark::Unlit,
            },
            Held {
                preserved: None,
                scanned: Some((1920, 1080)),
                dark: Dark::WindowlessBrief,
            },
        ];
        for prev in &states {
            let held = held_after_life_reset(prev);
            assert!(
                held.preserved.is_none(),
                "{prev:?}: nothing the old life preserved"
            );
            // gop=on: boot_done goes back to false with the life
            assert_eq!(
                choose_shown(None, Some(&boot), false, &held),
                Some(Shown::Boot(boot.layer, boot.size)),
                "{prev:?}"
            );
            // the next life's first armed head takes over, as on the first boot
            assert_eq!(
                choose_shown(Some(comp.clone()), Some(&boot), false, &held),
                Some(Shown::Armed(comp.clone()))
            );
            // gop=off: black if a scanout was ever shown, else no frame (the placeholder)
            assert_eq!(
                choose_shown(None, None, false, &held),
                prev.scanned.map(Shown::Blank),
                "{prev:?}"
            );
        }
    }

    /// ★★ 2026-10-18 (branch `claude/bootlayer-20261018`): the boot layer is shown only while store
    /// `[0, G)` is clean. A guest RM life writes its own vidmem there (`[measured, run 3081]` coloured
    /// rows at fixed positions on the boot screen and after an unload), so while it is tainted the
    /// console shows no boot layer: the last frame before any guest scanout, black after one was
    /// lost — never the RM's data. An armed composition is not affected, and a completed scrub
    /// brings the boot layer back from every state a life can end in.
    #[test]
    fn a_tainted_boot_range_is_never_shown_and_a_completed_scrub_brings_it_back() {
        let boot = boot_scan();
        let comp = Composition {
            head: 0,
            width: 1920,
            height: 1080,
            layers: Vec::new(),
        };
        let states = [
            Held::default(),
            Held {
                preserved: None,
                scanned: Some((1280, 720)),
                dark: Dark::Unlit,
            },
            Held {
                preserved: None,
                scanned: Some((1920, 1080)),
                dark: Dark::WindowlessBrief,
            },
        ];
        for prev in &states {
            let held = held_after_life_reset(prev);
            // tainted: not the boot layer — the last frame stays (None) or black
            let shown = choose_shown_gated(None, Some(&boot), false, false, &held);
            assert_ne!(
                shown,
                Some(Shown::Boot(boot.layer, boot.size)),
                "{prev:?}: tainted bytes must not be drawn"
            );
            assert_eq!(shown, prev.scanned.map(Shown::Blank), "{prev:?}");
            // clean again: the boot layer returns (must-keep: unload / reset go back to it)
            assert_eq!(
                choose_shown_gated(None, Some(&boot), false, true, &held),
                Some(Shown::Boot(boot.layer, boot.size)),
                "{prev:?}"
            );
            // a guest that arms its display is unaffected either way
            for clean in [false, true] {
                assert_eq!(
                    choose_shown_gated(Some(comp.clone()), Some(&boot), false, clean, &held),
                    Some(Shown::Armed(comp.clone()))
                );
            }
        }
        // the first boot: tainted before anything was ever shown is NO new frame (the last stays)
        assert_eq!(
            choose_shown_gated(None, Some(&boot), false, false, &Held::default()),
            None,
            "no black at the first handoff"
        );
        // gop=off is untouched by the gate
        assert_eq!(
            choose_shown_gated(None, None, false, false, &Held::default()),
            None
        );
    }

    /// ★ 2026-10-03 (B5, `V3_DISPLAY.md` §4.11.13): a head's scanout that WAS shown and is lost is
    /// black (box run b5 kept the last fbcon frame after `rmmod nvidia_drm`) — at once when no head
    /// is lit, after [`WINDOWLESS_HOLD`] when a lit head has no window — and a scanout freed with
    /// `PRESERVE_HW` stays until a head is armed again.
    #[test]
    fn a_lost_scanout_is_black_and_a_preserved_scanout_stays() {
        let comp = Composition {
            head: 3,
            width: 1920,
            height: 1080,
            layers: Vec::new(),
        };
        let boot = boot_scan();
        let lost = |dark| Held {
            preserved: None,
            scanned: Some((1280, 720)),
            dark,
        };
        for g in [Some(&boot), None] {
            assert_eq!(
                choose_shown(None, g, true, &lost(Dark::Unlit)),
                Some(Shown::Blank((1280, 720))),
                "no head lit: black at once, at the size last shown (gop={})",
                g.is_some()
            );
            assert_eq!(
                choose_shown(None, g, true, &lost(Dark::Windowless)),
                Some(Shown::Blank((1280, 720))),
                "a lit head with no window past the hold: black"
            );
            assert_eq!(
                choose_shown(None, g, true, &lost(Dark::WindowlessBrief)),
                None,
                "a lit head with no window inside the hold (a modeset): the last frame stays"
            );
        }
        let kept = Held {
            preserved: Some((vec![boot.layer], (1920, 1080))),
            scanned: Some((1920, 1080)),
            dark: Dark::Unlit,
        };
        assert_eq!(
            choose_shown(None, None, true, &kept),
            Some(Shown::Preserved(vec![boot.layer], (1920, 1080)))
        );
        assert_eq!(
            choose_shown(Some(comp.clone()), None, true, &kept),
            Some(Shown::Armed(comp)),
            "an armed head wins over the preserved scanout"
        );
        // the boot layer still comes first while no head was ever armed
        assert_eq!(
            choose_shown(None, Some(&boot), false, &kept),
            Some(Shown::Boot(boot.layer, (1920, 1080)))
        );
    }

    /// ⊘ The review of `v3-gop-unload` (2026-10-03): `[measured b1f]` *"+52936 ms the console shows
    /// BLACK"* 4 ms before head 3's first window — the old rule blanked whenever a frame had been
    /// shown and nothing was scanned. No scanout that was never shown is ever lost: the boot layer
    /// → first-head handoff, GB20x (no window vocabulary: never an armed composition) and `gop=off`
    /// before its first window all show no new frame, whatever the heads say.
    #[test]
    fn the_handoff_and_a_family_without_windows_never_go_black() {
        let boot = boot_scan();
        for dark in [
            Dark::No,
            Dark::Unlit,
            Dark::WindowlessBrief,
            Dark::Windowless,
        ] {
            let never = Held {
                preserved: None,
                scanned: None,
                dark,
            };
            assert_eq!(
                choose_shown(None, Some(&boot), true, &never),
                None,
                "the boot layer's last frame stays ({dark:?})"
            );
            assert_eq!(
                choose_shown(None, None, true, &never),
                None,
                "gop=off before any window ({dark:?})"
            );
        }
    }

    /// ★★ 2026-10-04 — B5 arm (a2) on box vmb, failed 5 of 5 runs with the candidate's and
    /// master's code (`traces/v3_candidates/cand1_20261003/display/`). NVKMS's teardown after X
    /// restores the console (window 6 latches context DMA `0x10088`), frees the console surface —
    /// RM clears `0x10088` from display instance memory while window 6 stays armed on it — and only
    /// then frees the window and core channels with `PRESERVE_HW` (`nvFreeDevEvo`,
    /// `ogkm-580: src/nvidia-modeset/src/nvkms-evo.c:9101-9112`). A refresh copy in that gap (a copy
    /// every 250 ms; the gap was 346 ms to 2.2 s on vmb, 153 ms on 54032077 where B5 passed) logged
    /// *"scanout REFUSED context DMA 0x10088 on channel 7: NotBound"*, and the free then showed
    /// *"BLACK … no head is lit"* instead of *"the PRESERVED scanout"*. The ordering, replayed:
    /// latch → copy → unbind → copy → the preserving frees.
    #[test]
    fn a_context_dma_unbound_before_the_preserving_free_keeps_the_console() {
        let formats =
            ScanFormats::resolve(kf_disp::class::for_version("580.159.04").unwrap(), 0xC67E);
        let so = scanout_6();
        let armed = Shown::Armed(Composition {
            head: 3,
            width: 1920,
            height: 1080,
            layers: vec![so],
        });
        // the console surface: store [0, 8 MiB), pitch
        let console = CtxDma {
            target: Target::Vidmem,
            base: 0,
            limit: 0x7F_FFFF,
            block_linear: false,
            writable: true,
        };
        let bound = |_: &kf_disp::engine::Scanout| Ok(console);
        let unbound = |s: &kf_disp::engine::Scanout| {
            Err(format!(
                "context DMA {:#x} on channel {}: NotBound",
                s.handle, s.chn
            ))
        };
        let size = (1920, 1080);
        let mut scan = ScanState::default();
        let mut held = Held::default();
        // 1. the restore: window 6 latches 0x10088, and the copy behind the flip resolves it
        scan.latched.forget(so.window);
        let latched = scan.plan(&armed, &formats, size, 1, bound);
        assert!(latched.refused.is_empty(), "{:?}", latched.refused);
        assert_eq!(latched.layers.len(), 1);
        scan.copied(&armed, &latched, size);
        held.scanned = Some(size);
        // 2. NVKMS frees the console surface: 0x10088 leaves the hash table, window 6 stays armed
        // 3. a refresh copy lands in the gap
        let gap = scan.plan(&armed, &formats, size, 2, unbound);
        assert!(
            gap.refused.is_empty(),
            "the ARMED window keeps scanning the surface it latched: {:?}",
            gap.refused
        );
        assert_eq!(gap.layers, latched.layers);
        scan.copied(&armed, &gap, size);
        // 4. the frees with PRESERVE_HW — windows 0..=7, then the core
        for _ in 0..9 {
            held.channel_freed(true, scan.last_plan.as_ref());
        }
        held.dark = Dark::Unlit;
        let boot = boot_scan();
        assert_eq!(
            choose_shown(None, Some(&boot), true, &held),
            Some(Shown::Preserved(latched.layers.clone(), size)),
            "the restored console stays on the monitor, as on bare metal"
        );
        // ⊘ the refusal stands for a window that LATCHES on a context DMA that does not resolve
        scan.latched.forget(so.window);
        let relatched = scan.plan(&armed, &formats, size, 3, unbound);
        assert!(relatched.layers.is_empty());
        assert_eq!(relatched.refused.len(), 1);
        assert!(relatched.refused[0].contains("NotBound"));
        scan.copied(&armed, &relatched, size);
        assert_eq!(
            scan.last_plan, None,
            "a copy that refused a window is not kept"
        );
        // ... and is never kept: the next copy asks again, and resolves once the handle is bound
        assert_eq!(
            scan.plan(&armed, &formats, size, 4, unbound).refused.len(),
            1
        );
        assert!(
            scan.plan(&armed, &formats, size, 5, bound)
                .refused
                .is_empty()
        );
        // ... and for a handle the armed state never resolved (a window on another context DMA)
        let other = Shown::Armed(Composition {
            head: 3,
            width: 1920,
            height: 1080,
            layers: vec![kf_disp::engine::Scanout {
                handle: 0x1_0093,
                ..so
            }],
        });
        assert_eq!(
            scan.plan(&other, &formats, size, 6, unbound).refused.len(),
            1
        );
        // a channel's new life starts unresolved
        scan.latched.forget(so.window);
        assert_eq!(
            scan.plan(&armed, &formats, size, 7, unbound).refused.len(),
            1
        );
    }

    /// ★ Windows playback (run 296, 2026-10-10): Edge's YUV video overlay (window 4, `SET_PARAMS.FORMAT`
    /// 0x38) has no console pixel format. The SDR colour program used to fail the whole copy for
    /// it (`colour frame has refused windows`), `failed` was set for the VM's life and the display
    /// thread halted EVERY guest display channel: window 0's flips froze and the guest reset the GPU
    /// (TDR). A console-only refusal now leaves the window out of the console copy, the flips behind
    /// the copy complete, nothing is halted, and the refusal is counted by name. A GPU failure still
    /// stops the display (known-positive: no forged completion for work that reached the GPU).
    #[test]
    fn a_console_only_refusal_completes_the_flip_and_never_halts_the_display() {
        let formats =
            ScanFormats::resolve(kf_disp::class::for_version("580.159.04").unwrap(), 0xC67E);
        let desktop = kf_disp::engine::Scanout {
            window: 0,
            head: 0,
            ..scanout_6()
        };
        let video = kf_disp::engine::Scanout {
            window: 4,
            head: 0,
            format: 0x38,
            iso1: 0,
            offset1: 0x10_0000,
            pitch1: 120,
            width: 1920,
            height: 1080,
            ..scanout_6()
        };
        assert!(formats.of(video.format).is_none(), "no RGB console format");
        assert!(
            formats.yuv_of(video.format).is_some(),
            "but a console YUV format"
        );
        let armed = Shown::Armed(Composition {
            head: 0,
            width: 1920,
            height: 1080,
            layers: vec![desktop, video],
        });
        let console_dma = CtxDma {
            target: Target::Vidmem,
            base: 0,
            limit: 0x7F_FFFF,
            block_linear: false,
            writable: true,
        };
        let mut scan = ScanState::default();
        let planned = scan.plan(&armed, &formats, (1920, 1080), 1, |_| Ok(console_dma));
        assert_eq!(
            planned.layers.len(),
            1,
            "the desktop window is still composed"
        );
        assert_eq!(planned.layers[0].window, 0);
        assert_eq!(
            planned.yuv.len(),
            1,
            "the YUV overlay is composed for the console"
        );
        assert_eq!(planned.yuv[0].window, 4);
        assert!(planned.refused.is_empty(), "{:?}", planned.refused);
        // a window format nothing can compose (I8) is left out by name, console-only
        let odd = kf_disp::engine::Scanout {
            format: 0x1E,
            ..video
        };
        let armed_odd = Shown::Armed(Composition {
            head: 0,
            width: 1920,
            height: 1080,
            layers: vec![desktop, odd],
        });
        let planned = scan.plan(&armed_odd, &formats, (1920, 1080), 2, |_| Ok(console_dma));
        assert_eq!(planned.layers.len(), 1);
        assert!(planned.yuv.is_empty());
        assert_eq!(planned.refused.len(), 1);
        assert!(
            planned.refused[0].contains("0x1e has no console format"),
            "{:?}",
            planned.refused
        );

        // a flip of window 0 waits for copy 1; the copy cannot be made for the console
        let (counters, console) = (DispCounters::default(), ConsoleShare::default());
        scan.barrier = 1;
        scan.started = 1;
        scan.fault(
            &counters,
            &console,
            1,
            0,
            Fault::Console("SDR colour program: x".into()),
        );
        assert!(
            !scan.failed,
            "the display thread would halt every guest channel on this"
        );
        assert!(
            scan.done >= scan.barrier,
            "the flip queued behind copy 1 is released"
        );
        assert_eq!(counters.console_copies_skipped.load(Ordering::Relaxed), 1);
        assert_eq!(counters.scanout_refused.load(Ordering::Relaxed), 1);
        // ... and the next copy is made as usual
        scan.barrier = 2;
        scan.started = 2;
        scan.finish_with(&counters, &console, 2, 0);
        assert!(scan.done >= scan.barrier && !scan.failed);

        // known-positive: a lost or failed GPU copy still stops the display and completes nothing
        let mut gpu = ScanState::default();
        gpu.barrier = 1;
        gpu.started = 1;
        gpu.fault(
            &counters,
            &console,
            1,
            0,
            Fault::Gpu("the completion signal: x".into()),
        );
        assert!(gpu.failed);
        assert!(gpu.done < gpu.barrier, "no forged completion for GPU work");
    }

    /// ★ The overlay's place is its programmed depth, not "last": a base plane in front of the video
    /// (an underlay with an alpha hole, depth smaller than the video's) composes AFTER it; a video in
    /// front of the base composes after the base; equal depths follow the window order.
    #[test]
    fn rgb_layers_and_yuv_windows_compose_in_programmed_depth_order() {
        let rgb = |window| LayerPlan {
            window,
            src: 0,
            extent: 0,
            block_linear: false,
            pitch: 0,
            block_height_log2: 0,
            x0_bytes: 0,
            y0: 0,
            width: 1,
            rows: 1,
            ox: 0,
            oy: 0,
            flags: 0,
            a_s: 255,
            b_s: 0,
            a_d: 0,
            b_d: 0,
        };
        let yuv = |window| kf_disp::scanout::YuvPlan {
            window,
            y_src: 0,
            y_extent: 0,
            c_src: 0,
            c_extent: 0,
            block_linear: false,
            y_pitch: 0,
            c_pitch: 0,
            block_height_log2: 0,
            sx0: 0,
            sy0: 0,
            sw: 1,
            sh: 1,
            ox: 0,
            oy: 0,
            dw: 1,
            dh: 1,
            sub_x_log2: 1,
            sub_y_log2: 1,
            vu_first: false,
            layout: 0,
            filter: 0,
            fmt: kf_disp::scanout::BT709_LIMITED,
        };
        let d = |m: &[(u32, u32)]| {
            let m = m.to_vec();
            move |w: u32| m.iter().find(|(x, _)| *x == w).map_or(0, |(_, d)| *d)
        };
        // Windows' MPO as measured (run 401): base window 0, overlay window 4 at depth 11
        assert_eq!(
            z_order(&[rgb(0)], &[yuv(4)], d(&[(0, 255), (4, 11)])),
            [Z::Rgb(0), Z::Yuv(0)],
            "the video in front of the base"
        );
        // the underlay: the base plane (alpha hole) in front of the video
        assert_eq!(
            z_order(&[rgb(0)], &[yuv(4)], d(&[(0, 5), (4, 11)])),
            [Z::Yuv(0), Z::Rgb(0)],
            "the base in front of the video"
        );
        // between two RGB layers
        assert_eq!(
            z_order(
                &[rgb(0), rgb(2)],
                &[yuv(4)],
                d(&[(0, 255), (2, 100), (4, 50)])
            ),
            [Z::Rgb(0), Z::Rgb(1), Z::Yuv(0)]
        );
        assert_eq!(
            z_order(
                &[rgb(0), rgb(2)],
                &[yuv(4)],
                d(&[(0, 255), (2, 20), (4, 50)])
            ),
            [Z::Rgb(0), Z::Yuv(0), Z::Rgb(1)]
        );
        // equal depth: window order
        assert_eq!(
            z_order(&[rgb(3)], &[yuv(1)], d(&[(3, 10), (1, 10)])),
            [Z::Yuv(0), Z::Rgb(0)]
        );
        // no YUV window: the planned order stands, whatever the depths
        assert_eq!(
            z_order(&[rgb(2), rgb(0)], &[], d(&[(0, 255), (2, 1)])),
            [Z::Rgb(0), Z::Rgb(1)]
        );
    }

    /// ⊘ The review of `v3-gop-unload` (2026-10-03): a page flip (a new context DMA or offset in
    /// the same window) changes no *"console shows"* key — `[measured b1f]` the old text key spent
    /// its 256 lines in ~4 s of flips — while a change of what is shown does.
    #[test]
    fn a_flip_is_not_a_console_change() {
        let layer = |handle: u32, offset: u64| kf_disp::engine::Scanout {
            handle,
            offset,
            ..scanout_6()
        };
        let comp = |l| {
            Shown::Armed(Composition {
                head: 3,
                width: 1920,
                height: 1080,
                layers: vec![l],
            })
        };
        let held = Held::default();
        let a = shown_key(Some(&comp(layer(0x1_0093, 0))), &held, true);
        let b = shown_key(Some(&comp(layer(0x1_0095, 0x80_0000))), &held, true);
        assert_eq!(a, b, "a flip");
        let wider = kf_disp::engine::Scanout {
            width: 1280,
            ..layer(0x1_0093, 0)
        };
        assert_ne!(a, shown_key(Some(&comp(wider)), &held, true));
        let lost = Held {
            preserved: None,
            scanned: Some((1920, 1080)),
            dark: Dark::WindowlessBrief,
        };
        assert_ne!(
            shown_key(None, &lost, true),
            shown_key(None, &Held::default(), true),
            "why nothing new is shown is part of the key"
        );
        assert_ne!(
            shown_key(Some(&Shown::Blank((1920, 1080))), &held, true),
            shown_key(None, &held, true)
        );
        // before any scanout was shown the heads' state changes nothing on screen, nor the line
        let unlit = Held {
            dark: Dark::Unlit,
            ..Held::default()
        };
        let brief = Held {
            dark: Dark::WindowlessBrief,
            ..Held::default()
        };
        assert_eq!(
            shown_key(None, &unlit, false),
            shown_key(None, &brief, false)
        );
    }

    /// ★ M2 triple buffering: the console only ever takes the newest READY frame; the worker's next
    /// target is never the one shown nor the one ready; an untaken frame is replaced, not queued.
    #[test]
    fn the_console_takes_the_newest_frame_and_the_gpu_never_writes_the_shown_one() {
        let c = ConsoleShare::default();
        assert_eq!(
            c.ring().slots(),
            3,
            "the broker off: three slots, as before"
        );
        assert_eq!(c.take(), None, "no frame before the first copy");
        let a = c.free_slot().unwrap();
        c.publish(a, frame(0x1000, 1), true, None);
        let shown = c.take().unwrap();
        assert_eq!((shown.addr, shown.serial), (0x1000, 1));
        assert_eq!(
            c.take().unwrap().serial,
            1,
            "nothing new: the same front again"
        );
        // two copies complete before the console asks: the second replaces the first
        let b = c.free_slot().unwrap();
        assert_ne!(b, a, "never the shown slot");
        c.publish(b, frame(0x2000, 2), true, None);
        let d = c.free_slot().unwrap();
        assert!(d != a && d != b, "neither shown nor ready");
        c.publish(d, frame(0x3000, 3), true, None);
        let e = c.free_slot().unwrap();
        assert_ne!(e, a, "the shown slot is still the console's");
        assert_ne!(e, d, "the ready slot is not a target");
        assert_eq!(
            c.take().unwrap().serial,
            3,
            "the newest, never the stale one"
        );
        for i in 0..100u64 {
            let t = c.free_slot().expect("three slots always leave a target");
            c.publish(t, frame(0x4000, 4 + i), true, None);
            if t.is_multiple_of(2) {
                c.take();
            }
        }
    }

    /// A five-slot broker ring whose slots carry a (one-page) memfd backing, as
    /// `BrokerSeat::frame` installs before the worker publishes into a slot.
    fn broker_ring() -> Arc<kf_broker::FrameRing> {
        let ring = Arc::new(kf_broker::FrameRing::new(
            kf_broker::slots::BROKER_SLOTS,
            true,
        ));
        for j in 0..ring.slots() {
            let mem =
                kf_linux_raw::SharedRam::create_named(c"kfq-test-frame", 4096).expect("memfd");
            ring.install(j, kf_broker::SlotFds::new(mem, None).expect("ids"))
                .expect("install");
        }
        ring
    }

    /// ★ With the broker on, the console and the relay read ONE ring: a frame the relay holds is
    /// never a fill target, and the console still gets every newest frame.
    #[test]
    fn the_console_and_the_broker_share_one_ring() {
        let ring = broker_ring();
        let c = ConsoleShare::over(ring.clone());
        let a = c.free_slot().unwrap();
        c.publish(a, frame(0x1000, 1), true, None);
        assert_eq!(ring.take_broker(), kf_broker::Take::Taken(a));
        assert_eq!(
            c.take().unwrap().serial,
            1,
            "the console sees the same frame"
        );
        let g = ring.geometry(a);
        assert_eq!(
            (g.width, g.stride, g.fourcc),
            (1920, 7680, kf_broker::wire::FOURCC_XR24)
        );
        for i in 0..50u64 {
            let t = c.free_slot().expect("five slots always leave a target");
            assert_eq!(
                ring.held_mask() & (1 << t),
                0,
                "never a frame the broker holds"
            );
            c.publish(t, frame(0x2000, 2 + i), true, None);
        }
        assert!(ring.release_held(a));
    }

    /// ★ §8.13 (the review, 2026-10-04): the console's cursor follows the frame the console SHOWS —
    /// the bit the worker publishes with a frame is what taking it reports, and a frame the console
    /// does not take (VRAM only) changes nothing. Known-positive: before any take, `Nothing`.
    #[test]
    fn the_console_share_carries_each_frames_cursor_bit_to_what_the_console_shows() {
        let c = ConsoleShare::default();
        assert_eq!(c.shown_frame(), ShownFrame::Nothing);
        let a = c.free_slot().unwrap();
        c.publish(
            a,
            FrameView {
                cursor: true,
                ..frame(0x1000, 1)
            },
            true,
            None,
        );
        assert!(c.take().unwrap().cursor);
        assert_eq!(c.shown_frame(), ShownFrame::CursorComposed);
        let b = c.free_slot().unwrap();
        c.publish(b, frame(0x2000, 2), true, None);
        assert!(!c.take().unwrap().cursor);
        assert_eq!(c.shown_frame(), ShownFrame::CursorFree);
        let v = c.free_slot().unwrap();
        c.publish(
            v,
            FrameView {
                cursor: true,
                ..frame(0, 3)
            },
            false,
            None,
        );
        assert_eq!(c.take().unwrap().serial, 2, "the console keeps b");
        assert_eq!(c.shown_frame(), ShownFrame::CursorFree);
        // and the cursor point the console reads is the one the worker noted, (-1, -1) included
        c.note_cursor_point(Some((-1, -1)));
        assert_eq!(c.cursor_point(), Some((-1, -1)));
        c.note_cursor_point(None);
        assert_eq!(c.cursor_point(), None);
    }

    /// ★ §8.11: a frame only in VRAM (the pack ran, the D2H did not) is offered to the broker and
    /// NOT to the console, which keeps its last host frame; and the two demand signals: broker
    /// activity wants frames (the refresh rate) without being the console's demand.
    #[test]
    fn a_vram_only_frame_goes_to_the_broker_and_the_console_keeps_its_host_frame() {
        let ring = broker_ring();
        for j in 0..ring.slots() {
            let fd = kf_linux_raw::SharedRam::create_named(c"kfq-test-vram", 4096)
                .expect("memfd")
                .dup_for_export()
                .expect("dup");
            ring.install_vram(j, kf_broker::VramFds::new(fd, 10 << 20).expect("id"))
                .expect("install");
        }
        let c = ConsoleShare::over(ring.clone());
        let a = c.free_slot().unwrap();
        c.publish(a, frame(0x1000, 1), true, None);
        assert_eq!(c.take().unwrap().serial, 1);
        let b = c.free_slot().unwrap();
        let vg = kf_broker::VramGeom {
            stride: 7680,
            extent: 8_847_360,
        };
        c.publish(b, frame(0, 2), false, Some(vg));
        assert_eq!(ring.broker_ready(), Some(b), "the broker is offered it");
        assert!(ring.backed(b, kf_broker::Kind::Vram) && !ring.backed(b, kf_broker::Kind::Host));
        assert_eq!(ring.vram_geometry(b), vg);
        let shown = c.take().unwrap();
        assert_eq!(
            (shown.addr, shown.serial),
            (0x1000, 1),
            "the console keeps its host frame"
        );
        // the demand split
        let d = ConsoleShare::default();
        assert!(!d.wanted_within(2000));
        d.note_broker_demand();
        assert!(d.wanted_within(2000), "the broker keeps the refresh rate");
        assert!(d.broker_wanted_within(2000));
        assert!(
            !d.console_wanted_within(2000),
            "broker activity is not the console's demand"
        );
        let _ = d.take();
        assert!(d.console_wanted_within(2000));
    }

    /// ★ The worker's refusal branch, DRIVEN (the second review of `v3-broker`, 2026-10-03: the
    /// first fix's tests called the ring directly, so deleting the call site passed them all):
    /// the first refused broker backing withdraws EVERY slot from the broker — the frame that
    /// was ready, and a slot the worker never reallocates (it keeps its broker memfd and the GPU
    /// goes on writing it) — while the console gets every frame; the seat is never asked again.
    #[test]
    fn a_refused_broker_backing_withdraws_every_slot_from_the_broker() {
        let ring = broker_ring();
        let c = ConsoleShare::over(ring.clone());
        let mut refused = None;
        assert_eq!(
            broker_backing(&ring, &mut refused, Some(|| Ok::<u32, String>(7))),
            Some(7),
            "before a refusal the seat's frame is used"
        );
        assert!(refused.is_none() && !ring.withdrawn(kf_broker::Kind::Host));
        let a = c.free_slot().unwrap();
        c.publish(a, frame(0x1000, 1), true, None);
        assert_eq!(
            ring.take_broker(),
            kf_broker::Take::Taken(a),
            "the broker holds a"
        );
        let b = c.free_slot().unwrap();
        c.publish(b, frame(0x2000, 2), true, None);
        assert_eq!(ring.broker_ready(), Some(b), "b waits for the broker");
        // a growing slot's broker backing is refused
        let got = broker_backing(
            &ring,
            &mut refused,
            Some(|| Err::<u32, String>("cuMemHostRegister of the frame memfd: refused".into())),
        );
        assert_eq!(got, None, "the console's own memory");
        assert!(
            refused
                .as_deref()
                .is_some_and(|e| e.contains("cuMemHostRegister"))
        );
        assert!(ring.withdrawn(kf_broker::Kind::Host));
        assert!(
            !ring.withdrawn(kf_broker::Kind::Vram),
            "a host refusal leaves the GPU-copy rung (§8.11)"
        );
        assert_eq!(ring.broker_ready(), None, "the ready frame b is dropped");
        // slots the worker never reallocates still carry their broker memfd: none is offered
        for i in 0..20u64 {
            let t = c.free_slot().expect("five slots always leave a target");
            assert!(ring.fds(t).is_some(), "slot {t} keeps its broker memfd");
            c.publish(t, frame(0x3000, 3 + i), true, None);
            assert_eq!(
                ring.take_broker(),
                kf_broker::Take::Empty,
                "slot {t} was offered to the broker after the refusal"
            );
            assert_eq!(c.take().unwrap().serial, 3 + i, "the console shows it");
        }
        // the seat is never asked again
        let again = broker_backing(
            &ring,
            &mut refused,
            Some(|| -> Result<u32, String> { panic!("the seat was asked after a refusal") }),
        );
        assert_eq!(again, None);
        assert!(
            ring.release_held(a),
            "the frame the broker held stays its own"
        );
        // without a broker nothing is withdrawn and no seat exists
        let plain = ConsoleShare::default();
        let mut none = None;
        assert_eq!(
            broker_backing(plain.ring(), &mut none, None::<fn() -> Result<u32, String>>),
            None
        );
        assert!(none.is_none() && !plain.ring().withdrawn(kf_broker::Kind::Host));
    }
}

#[cfg(test)]
mod diagnostic_gate_tests {
    use super::*;

    // The green commit changes only this fixture's explicit diagnostic selection.
    fn enabled_meter() -> LatStat {
        LatStat::with_diagnostics(true)
    }

    #[test]
    fn diagnostic_default_meter_does_not_record_samples() {
        let m = LatStat::default();
        m.add(Duration::from_micros(12));
        assert_eq!(
            m.fragment("edge"),
            "edge=0/0/0",
            "no diagnostic flag: no latency samples"
        );
    }

    #[test]
    fn diagnostic_default_counters_do_not_record_any_step() {
        let c = DispCounters::default();
        for m in [&c.lat_edge, &c.lat_resolve, &c.lat_start, &c.lat_pass] {
            m.add(Duration::from_micros(7));
            assert_eq!(
                m.fragment("step"),
                "step=0/0/0",
                "all four steps must be disabled by default"
            );
        }
    }

    #[test]
    fn diagnostic_worker_never_takes_an_unguarded_meter_timestamp() {
        let code = worker_code_for_diagnostics();
        for raw in [
            "let pass_t0 = Instant::now()",
            "let t_res = Instant::now()",
            "let t_st = Instant::now()",
        ] {
            assert!(
                !code.contains(raw),
                "diagnostics off still takes a timestamp: {raw}"
            );
        }
        for guarded in [
            "lat_pass.stamp()",
            "lat_resolve.stamp()",
            "lat_start.stamp()",
        ] {
            assert!(
                code.contains(guarded),
                "each measurement must use the gated clock: {guarded}"
            );
        }
    }

    fn worker_code_for_diagnostics() -> &'static str {
        include_str!("display.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap()
    }

    #[test]
    fn diagnostic_enabled_meter_keeps_the_legacy_fragment() {
        let m = enabled_meter();
        m.add(Duration::from_micros(5));
        m.add(Duration::from_micros(12));
        assert_eq!(m.fragment("edge"), "edge=2/17/12");
    }

    #[test]
    fn diagnostic_enabled_meter_keeps_nanosecond_sums_and_maximum() {
        let m = enabled_meter();
        for ns in [1500, 2500, 500] {
            m.add(Duration::from_nanos(ns));
        }
        assert_eq!(m.n.load(Ordering::Relaxed), 3);
        assert_eq!(m.sum_ns.load(Ordering::Relaxed), 4500);
        assert_eq!(m.max_ns.load(Ordering::Relaxed), 2500);
        assert_eq!(m.fragment("pass"), "pass=3/4/2");
    }

    #[test]
    fn diagnostic_disabled_meter_never_calls_either_clock() {
        let m = LatStat::with_diagnostics(false);
        assert!(
            m.start_with(|| panic!("disabled start read the clock"))
                .is_none()
        );
        m.finish_with(Some(Instant::now()), || {
            panic!("disabled finish read the clock")
        });
        assert_eq!(m.fragment("pass"), "pass=0/0/0");
        assert!(
            DispCounters::with_step_meters(false)
                .step_fragment()
                .is_empty()
        );
    }

    #[test]
    fn diagnostic_enabled_meter_keeps_the_legacy_start_to_finish_interval() {
        let m = enabled_meter();
        let now = Instant::now();
        let start = m.start_with(|| now);
        assert_eq!(start, Some(now));
        m.finish_with(start, || now + Duration::from_micros(17));
        assert_eq!(m.fragment("resolve"), "resolve=1/17/17");
        assert_eq!(
            DispCounters::with_step_meters(true).step_fragment(),
            " lat[edge=0/0/0 resolve=0/0/0 start=0/0/0 pass=0/0/0]"
        );
    }
}

#[cfg(test)]
mod lut_life_tests {
    use super::*;

    fn lut() -> kf_disp::color::Lut {
        kf_disp::color::Lut {
            entries: 1025,
            binding: kf_disp::color::Binding::Dma {
                handle: 7,
                offset: 0,
            },
            interpolate: false,
            mirror: false,
        }
    }

    fn dma() -> CtxDma {
        CtxDma {
            target: kf_disp::inst::Target::Vidmem,
            base: 0x1000_0000,
            limit: 0x1001_ffff,
            block_linear: false,
            writable: false,
        }
    }

    fn snapshot_key(lut: kf_cuda::display::ColorLut) -> (u64, bool, u64, u32) {
        // Pin this GPU-free cache model to the actual retained GPU snapshot key.
        let source: String = include_str!("../../kf-cuda/src/display.rs")
            .split("fn color_snapshot(")
            .nth(1)
            .unwrap()
            .split("pub fn color_layer(")
            .next()
            .unwrap()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(source.contains("letkey=(lut.src,lut.interpolate,lut.token,lut.entries);"));
        assert!(source.contains("(old.src,old.interpolate,old.token,old.entries)!=key"));
        (lut.src, lut.interpolate, lut.token, lut.entries)
    }

    #[test]
    fn a_life_reset_with_the_same_lut_location_and_shape_requires_a_new_gpu_snapshot() {
        let mut scan = ScanState::default();
        let old = scan
            .colors
            .resolve(0, 1, 2, 3, Some(lut()), |_| Ok(dma()))
            .unwrap()
            .unwrap();
        scan.reset_life();
        assert!(
            scan.colors.slots.iter().all(Option::is_none),
            "all per-life bindings must be cleared"
        );
        let mut resolved = 0;
        let new = scan
            .colors
            .resolve(0, 1, 2, 3, Some(lut()), |_| {
                resolved += 1;
                Ok(dma())
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            resolved, 1,
            "reused channel ids must resolve their binding afresh"
        );
        assert_eq!(
            (old.src, old.entries, old.interpolate),
            (new.src, new.entries, new.interpolate)
        );
        assert_ne!(
            snapshot_key(old),
            snapshot_key(new),
            "the old GPU snapshot key must not suppress the new life's LUT copy"
        );
    }

    #[test]
    fn repeated_life_resets_keep_tokens_newer_than_every_prior_snapshot() {
        let mut scan = ScanState::default();
        let mut highest = 0;
        for _ in 0..3 {
            for slot in [0, 32, 64] {
                let resolved = scan
                    .colors
                    .resolve(slot, 1, 2, 3, Some(lut()), |_| Ok(dma()))
                    .unwrap()
                    .unwrap();
                assert!(
                    resolved.token > highest,
                    "a new life reused snapshot token {} after {}",
                    resolved.token,
                    highest
                );
                highest = resolved.token;
            }
            scan.reset_life();
            assert!(
                scan.colors.slots.iter().all(Option::is_none),
                "no binding survives the life reset"
            );
        }
    }
}
