//! The engine against the method streams NVKMS pushes (`ogkm-580: nvkms-evo3.c`), built from the
//! DERIVED class tables — every method offset below is looked up by name.

use super::*;
use crate::class::{self, put};

const CORE: u32 = 0xC67D;
const WIN: u32 = 0xC67E;
const IMM: u32 = 0xC67B;
const CLIENT: u32 = 0xc1d0_0015;

fn t() -> &'static ClassTable {
    class::for_version("580.159.04").unwrap()
}

fn classes() -> Classes {
    Classes::of(&kf_chip::display::AMPERE)
}

fn engine() -> Engine {
    let r = Regs::for_ip("580.159.04", 0x0401_0000).unwrap();
    Engine::new(
        Vocab::resolve(t(), &classes(), &r).expect("GA10x vocabulary"),
        4,
        8,
    )
}

fn m(cl: u32, n: &str) -> u32 {
    t().v(cl, n).unwrap_or_else(|| panic!("NV{cl:04X}_{n}"))
}
fn ma(cl: u32, n: &str, i: u32) -> u32 {
    t().a(cl, n, i)
        .unwrap_or_else(|| panic!("NV{cl:04X}_{n}({i})"))
}
fn fl(cl: u32, n: &str) -> (u8, u8) {
    t().f(cl, n).unwrap_or_else(|| panic!("NV{cl:04X}_{n}"))
}

/// A NVKMS-style stream: every method with its own one-word header.
struct Ring {
    words: Vec<u32>,
}
impl Ring {
    fn new() -> Ring {
        Ring { words: Vec::new() }
    }
    fn m(&mut self, method: u32, data: u32) -> &mut Ring {
        self.words.push((1 << 18) | method);
        self.words.push(data);
        self
    }
    fn put(&self) -> u32 {
        (self.words.len() * 4) as u32
    }
    fn bytes(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self.words.iter().flat_map(|w| w.to_le_bytes()).collect();
        v.resize(4096, 0);
        v
    }
}

fn all_ok(_: &Acquire) -> bool {
    true
}

fn pb() -> Option<PbLoc> {
    Some(PbLoc {
        sysmem: true,
        addr: 0x1000_0000,
        bytes: 4096,
    })
}

#[test]
fn method_diagnostic_is_off_by_default_and_cannot_be_replenished() {
    for budget in [0, 2] {
        let mut e = engine();
        e.trace_methods(budget);
        let n = e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0).unwrap();
        let mut ring = Ring::new();
        ring.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0)
            .m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0)
            .m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0);
        let s = e.step(n, &ring.bytes(), ring.put(), &mut all_ok);
        assert_eq!(
            s.effects
                .iter()
                .filter(|x| matches!(x, Effect::Trace(_)))
                .count(),
            budget as usize
        );
        assert_eq!(e.methods, 3);
        assert_eq!(e.method_trace_remaining, 0);
        e.free(ChannelKind::Window, 0);
        e.trace_methods(MAX_METHOD_TRACE);
        assert_eq!(e.method_trace_remaining, 0);
    }
    let mut e = engine();
    e.trace_methods(u32::MAX);
    assert_eq!(e.method_trace_remaining, MAX_METHOD_TRACE);
}

#[test]
fn constructor_probe_never_decodes_arms_or_completes_dma_or_pio() {
    let r = Regs::for_ip("580.159.04", 0x0401_0000).unwrap();
    let mut e = Engine::new_constructor_probe(Vocab::resolve(t(), &classes(), &r).unwrap(), 4, 8);
    for kind in [
        ChannelKind::Core,
        ChannelKind::Window,
        ChannelKind::WindowImm,
        ChannelKind::Cursor,
    ] {
        let n = e.alloc(kind, 0, CLIENT, 1, pb(), 0).unwrap();
        let mut ring = Ring::new();
        ring.m(e.vocab.update_of(kind), 0);
        let s = if kind == ChannelKind::Cursor {
            e.cursor_write(0, e.vocab.update_of(kind), 0, &mut |_| panic!("no acquire"))
        } else {
            e.step(n, &ring.bytes(), ring.put(), &mut |_| panic!("no acquire"))
        };
        assert!(
            matches!(s.effects.as_slice(), [Effect::Exception { chn, at: 0, .. }] if *chn == n)
        );
        assert_eq!(s.gets, vec![(n, 1, 0)]);
        let c = e.chans[n as usize].as_ref().unwrap();
        assert!(
            c.queue.is_empty() && c.assy.iter().all(|v| *v == 0) && c.armed.iter().all(|v| *v == 0)
        );
        assert_eq!(c.decoded, 0);
        e.free(kind, 0);
        e.alloc(kind, 0, CLIENT, 2, pb(), 0).unwrap();
        let s = e.step(n, &ring.bytes(), ring.put(), &mut |_| panic!("no acquire"));
        assert!(matches!(s.effects.as_slice(), [Effect::Exception { .. }]));
        assert_eq!(s.gets, vec![(n, 2, 0)]);
    }
    for _ in 0..10000 {
        assert!(
            e.cursor_write(0, 0, 123, &mut |_| panic!("no acquire"))
                .effects
                .is_empty()
        );
    }
    let c = e.chans[ChannelKind::Cursor.channel_number(0) as usize]
        .as_ref()
        .unwrap();
    assert!(c.queue.is_empty());
    assert_eq!((e.methods, e.updates), (0, 0));
    assert!(
        e.vblank(0, &mut |_| panic!("no acquire"))
            .effects
            .is_empty()
    );
    assert!(
        e.poll_acquires(&mut |_| panic!("no acquire"))
            .effects
            .is_empty()
    );
}

#[test]
fn halted_cursor_does_not_accumulate_more_pio() {
    let mut e = engine();
    let n = e.alloc(ChannelKind::Cursor, 0, CLIENT, 1, None, 0).unwrap();
    e.chans[n as usize].as_mut().unwrap().halted = true;
    for _ in 0..10000 {
        assert!(
            e.cursor_write(0, 0, 123, &mut |_| panic!("no acquire"))
                .effects
                .is_empty()
        );
    }
    assert!(e.chans[n as usize].as_ref().unwrap().queue.is_empty());
}

/// Core notifier: SET_CONTEXT_DMA_NOTIFIER, SET_NOTIFIER_CONTROL(offset idx, WRITE_AWAKEN, NOTIFY).
fn core_notifier(r: &mut Ring, handle: u32, idx: u32) {
    r.m(m(CORE, "SET_CONTEXT_DMA_NOTIFIER"), handle);
    let mut ctl = put(0, fl(CORE, "SET_NOTIFIER_CONTROL_OFFSET"), idx);
    ctl = put(ctl, fl(CORE, "SET_NOTIFIER_CONTROL_MODE"), 1);
    ctl = put(ctl, fl(CORE, "SET_NOTIFIER_CONTROL_NOTIFY"), 1);
    r.m(m(CORE, "SET_NOTIFIER_CONTROL"), ctl);
}

/// A head raster + pixel clock (1080p60: 2200×1125 at 148.5 MHz) and window `w` owned by `head`.
fn modeset(r: &mut Ring, head: u32, w: u32) {
    let rs = put(
        put(0, fl(CORE, "HEAD_SET_RASTER_SIZE_WIDTH"), 2200),
        fl(CORE, "HEAD_SET_RASTER_SIZE_HEIGHT"),
        1125,
    );
    r.m(ma(CORE, "HEAD_SET_RASTER_SIZE", head), rs);
    r.m(
        ma(CORE, "HEAD_SET_PIXEL_CLOCK_FREQUENCY", head),
        148_500_000,
    );
    r.m(ma(CORE, "WINDOW_SET_CONTROL", w), head);
}

/// ★ NVKMS's core InitChannel (≈6 KiB of scaler coefficients) through a 4 KiB ring: the engine
/// consumes it all, follows the wrap JUMP, and GET reaches every PUT — the wall of the 2026-09-29
/// display smoke (`GET 0 : PUT 4040`) is gone.
#[test]
fn init_methods_are_consumed_through_the_wrap() {
    let mut e = engine();
    let chn = e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0).unwrap();
    assert_eq!(chn, 0);
    let coeff = ma(CORE, "HEAD_SET_VIEWPORT_POINT_IN", 0);
    let mut ring = vec![0u32; 1024];
    let (mut put_off, mut total) = (0usize, 0u32);
    for pass in 0..3 {
        // fill up to 4040 bytes, then JUMP to 0 (what nvEvoMakeRoom writes at the end)
        let mut n = 0;
        while put_off + 8 <= 4040 && n < 300 {
            ring[put_off / 4] = (1 << 18) | coeff;
            ring[put_off / 4 + 1] = pass * 1000 + n;
            put_off += 8;
            n += 1;
        }
        let bytes: Vec<u8> = ring.iter().flat_map(|w| w.to_le_bytes()).collect();
        let s = e.step(0, &bytes, put_off as u32, &mut all_ok);
        assert!(
            s.effects.is_empty(),
            "no UPDATE, no effect: {:?}",
            s.effects
        );
        assert_eq!(s.gets, vec![(0, 1, put_off as u32)]);
        total += n;
        if put_off + 8 > 4040 {
            ring[put_off / 4] = 1 << 29; // JUMP 0
            put_off = 0;
        }
    }
    assert_eq!(e.methods, u64::from(total));
    assert_eq!(e.exceptions, 0);
}

/// ★ A synchronous core UPDATE: the state is ARMED first (the words NVKMS later reads back from the
/// ARMED half), then the notifier it asked for is stated — never the other way round.
#[test]
fn a_core_update_arms_then_notifies() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    let mut r = Ring::new();
    r.m(ma(CORE, "WINDOW_SET_CONTROL", 1), 0);
    core_notifier(&mut r, 0xcafe_0001, 2);
    r.m(m(CORE, "SET_INTERLOCK_FLAGS"), 0)
        .m(m(CORE, "SET_WINDOW_INTERLOCK_FLAGS"), 0)
        .m(m(CORE, "UPDATE"), 0);
    let s = e.step(0, &r.bytes(), r.put(), &mut all_ok);
    match s.effects.as_slice() {
        [
            Effect::CoreArmed(w),
            Effect::Notify {
                chn: 0,
                client: CLIENT,
                handle: 0xcafe_0001,
                offset: 32,
                awaken: true,
                finished: true,
            },
        ] => {
            assert!(
                w.contains(&(ma(CORE, "WINDOW_SET_CONTROL", 1), 0))
                    || !w
                        .iter()
                        .any(|(o, _)| *o == ma(CORE, "WINDOW_SET_CONTROL", 1))
            );
            assert!(
                w.iter()
                    .any(|(o, v)| *o == m(CORE, "SET_CONTEXT_DMA_NOTIFIER") && *v == 0xcafe_0001)
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.gets, vec![(0, 1, r.put())]);
    assert_eq!(
        e.armed(ChannelKind::Core, 0, m(CORE, "SET_CONTEXT_DMA_NOTIFIER")),
        Some(0xcafe_0001)
    );
    assert_eq!(e.updates, 1);
}

/// ★ nvEvoUpdateC3's modeset group: the core UPDATE is interlocked with window 0 (and window 0 with
/// its immediate channel). The core STOPS at its UPDATE — busy, GET before the method — until the
/// window's and the immediate channel's UPDATEs arrive; then all three arm together and the core's
/// notifier follows.
#[test]
fn an_interlocked_group_waits_for_every_member() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::WindowImm, 0, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    modeset(&mut c, 0, 0);
    core_notifier(&mut c, 0xcafe_0001, 0);
    c.m(m(CORE, "SET_INTERLOCK_FLAGS"), 0);
    c.m(m(CORE, "SET_WINDOW_INTERLOCK_FLAGS"), 1 << 0);
    let at_update = c.put();
    c.m(m(CORE, "UPDATE"), 0);
    let s = e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert!(s.effects.is_empty(), "{:?}", s.effects);
    assert!(
        s.gets.contains(&(0, 1, at_update)),
        "GET stands before the UPDATE: {:?}",
        s.gets
    );
    assert!(e.waiting(0));
    // the immediate channel (interlocked with its window) arrives first — still waiting
    let mut i = Ring::new();
    i.m(
        m(IMM, "UPDATE"),
        put(0, fl(IMM, "UPDATE_INTERLOCK_WITH_WINDOW"), 1),
    );
    let s = e.step(33, &i.bytes(), i.put(), &mut all_ok);
    assert!(s.effects.is_empty());
    // the window: interlocked with core and its immediate channel
    let mut w = Ring::new();
    w.m(
        m(WIN, "SET_INTERLOCK_FLAGS"),
        put(0, fl(WIN, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CORE"), 1),
    );
    w.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 0);
    w.m(
        m(WIN, "UPDATE"),
        put(0, fl(WIN, "UPDATE_INTERLOCK_WITH_WIN_IMM"), 1),
    );
    let s = e.step(1, &w.bytes(), w.put(), &mut all_ok);
    let kinds: Vec<&str> = s
        .effects
        .iter()
        .map(|x| match x {
            Effect::CoreArmed(_) => "armed",
            Effect::Notify { .. } => "notify",
            Effect::Latched { .. } => "latched",
            Effect::Heads => "heads",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["armed", "notify", "latched", "heads"],
        "{:?}",
        s.effects
    );
    assert!(
        s.gets.contains(&(0, 1, c.put()))
            && s.gets.contains(&(1, 1, w.put()))
            && s.gets.contains(&(33, 1, i.put()))
    );
    assert!(!e.waiting(0) && !e.waiting(1) && !e.waiting(33));
    let h = e.heads_armed();
    assert_eq!(h[0].raster, (2200, 1125));
    assert_eq!(h[0].period_ns, 2200 * 1125 * 1_000_000_000 / 148_500_000);
    assert_eq!(h[1].period_ns, 0);
}

/// ★ 2026-10-10 (run 293, measured): a ONE-SIDED interlock. The overlay window 4 interlocks with window 0 and with
/// its immediate channel; window 0's own UPDATE names nothing. Window 0's update must latch TOGETHER with the waiting
/// overlay update (and its immediate channel), never alone ahead of it — alone, the overlay starves.
#[test]
fn a_pending_update_that_waits_for_a_channel_latches_with_that_channels_next_update() {
    let mut e = engine();
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 4, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::WindowImm, 4, CLIENT, 1, pb(), 0);
    let w4 = ChannelKind::Window.channel_number(4);
    let i4 = ChannelKind::WindowImm.channel_number(4);
    let mut w = Ring::new();
    w.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 1 << 0);
    w.m(
        m(WIN, "UPDATE"),
        put(0, fl(WIN, "UPDATE_INTERLOCK_WITH_WIN_IMM"), 1),
    );
    assert!(
        e.step(w4, &w.bytes(), w.put(), &mut all_ok)
            .effects
            .is_empty()
    );
    let mut i = Ring::new();
    i.m(
        m(IMM, "UPDATE"),
        put(0, fl(IMM, "UPDATE_INTERLOCK_WITH_WINDOW"), 1),
    );
    assert!(
        e.step(i4, &i.bytes(), i.put(), &mut all_ok)
            .effects
            .is_empty()
    );
    assert!(
        e.waiting(w4) && e.waiting(i4),
        "the overlay waits for window 0"
    );
    // window 0's update, interlocked with nothing
    let mut z = Ring::new();
    z.m(m(WIN, "UPDATE"), 0);
    let s = e.step(1, &z.bytes(), z.put(), &mut all_ok);
    let latched: Vec<u32> = s
        .effects
        .iter()
        .filter_map(|x| match x {
            Effect::Latched { window } => Some(*window),
            _ => None,
        })
        .collect();
    assert_eq!(
        latched,
        vec![0, 4],
        "window 0 latches WITH the overlay: {:?}",
        s.effects
    );
    assert!(!e.waiting(1) && !e.waiting(w4) && !e.waiting(i4));
}

/// ★ 2026-10-10 (runs 400/403, measured: the guest's flip queue declared a TDR 2 s after a present
/// whose plane 0 completed and whose plane 1 never did; the STALL report named chn 5 waiting for chn 1
/// and chn 37 while chn 1 was not at an UPDATE). The overlay window 4 re-enabled after being off:
/// window 0's UPDATE (names nothing) arrived FIRST and was parked for its head's vblank; the overlay's
/// UPDATE (names window 0 and its immediate channel) arrived 21 us later. Window 0's update is still
/// PENDING in hardware until that vblank, so the overlay's update joins it and all latch together at
/// the vblank — it must not wait for a window-0 UPDATE that the driver will never send (it waits for the
/// overlay's completion).
#[test]
fn an_update_naming_a_channel_already_parked_for_the_vblank_joins_its_latch() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 4, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::WindowImm, 4, CLIENT, 1, pb(), 0);
    let (w0, w4) = (1, ChannelKind::Window.channel_number(4));
    let i4 = ChannelKind::WindowImm.channel_number(4);
    // head 0 lit, windows 0 and 4 owned by it
    let mut c = Ring::new();
    modeset(&mut c, 0, 0);
    c.m(ma(CORE, "WINDOW_SET_CONTROL", 4), 0);
    c.m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    // both windows scan a surface (each channel's ring keeps growing: GET follows PUT)
    let (mut r0, mut r4, mut ri) = (Ring::new(), Ring::new(), Ring::new());
    for (w, n, r) in [(0u32, w0, &mut r0), (4, w4, &mut r4)] {
        r.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x1_0001 + w);
        r.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 0);
        r.m(m(WIN, "UPDATE"), 0);
        e.step(n, &r.bytes(), r.put(), &mut all_ok);
        e.vblank(0, &mut all_ok);
    }
    // window 0's flip first: parked for head 0's vblank
    r0.m(m(WIN, "UPDATE"), 0);
    let s = e.step(w0, &r0.bytes(), r0.put(), &mut all_ok);
    assert!(
        s.effects.is_empty(),
        "parked for the vblank: {:?}",
        s.effects
    );
    assert!(e.waiting(w0));
    // 21 us later: the overlay's flip, naming window 0 and its immediate channel
    r4.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 1 << 0);
    r4.m(
        m(WIN, "UPDATE"),
        put(0, fl(WIN, "UPDATE_INTERLOCK_WITH_WIN_IMM"), 1),
    );
    e.step(w4, &r4.bytes(), r4.put(), &mut all_ok);
    ri.m(
        m(IMM, "UPDATE"),
        put(0, fl(IMM, "UPDATE_INTERLOCK_WITH_WINDOW"), 1),
    );
    e.step(i4, &ri.bytes(), ri.put(), &mut all_ok);
    assert!(
        e.waiting(w0) && e.waiting(w4) && e.waiting(i4),
        "all three wait for the vblank together"
    );
    let s = e.vblank(0, &mut all_ok);
    let mut latched: Vec<u32> = s
        .effects
        .iter()
        .filter_map(|x| match x {
            Effect::Latched { window } => Some(*window),
            _ => None,
        })
        .collect();
    latched.sort_unstable();
    assert_eq!(latched, vec![0, 4], "{:?}", s.effects);
    assert!(!e.waiting(w0) && !e.waiting(w4) && !e.waiting(i4));
}

/// ★ A plain non-tearing flip on an active head latches at the head's VBLANK: until then the window
/// is busy with GET before its UPDATE, and the flip's notifier is NOT written.
#[test]
fn a_non_tearing_flip_waits_for_vblank_and_its_acquire() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    modeset(&mut c, 0, 0);
    c.m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    let mut w = Ring::new();
    w.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x1_0001);
    w.m(m(WIN, "SET_CONTEXT_DMA_NOTIFIER"), 0xcafe_00f0);
    w.m(
        m(WIN, "SET_NOTIFIER_CONTROL"),
        put(
            put(0, fl(WIN, "SET_NOTIFIER_CONTROL_OFFSET"), 1),
            fl(WIN, "SET_NOTIFIER_CONTROL_MODE"),
            1,
        ),
    );
    // acquire: semaphore at 16-byte slot 3 must EQUAL 0xf473f473
    w.m(m(WIN, "SET_CONTEXT_DMA_ACQ_SEMAPHORE"), 0xcafe_0a00);
    w.m(
        m(WIN, "SET_ACQ_SEMAPHORE_CONTROL"),
        put(0, fl(WIN, "SET_ACQ_SEMAPHORE_CONTROL_OFFSET"), 3),
    );
    w.m(m(WIN, "SET_ACQ_SEMAPHORE_VALUE"), 0xf473_f473);
    // release: 0xd00dd00d at slot 4
    w.m(m(WIN, "SET_CONTEXT_DMA_SEMAPHORE"), 0xcafe_0b00);
    w.m(
        m(WIN, "SET_SEMAPHORE_CONTROL"),
        put(0, fl(WIN, "SET_SEMAPHORE_CONTROL_OFFSET"), 4),
    );
    w.m(m(WIN, "SET_SEMAPHORE_RELEASE"), 0xd00d_d00d);
    let at = w.put();
    w.m(m(WIN, "UPDATE"), 0);
    let s = e.step(1, &w.bytes(), w.put(), &mut all_ok);
    assert!(s.effects.is_empty(), "parked for vblank: {:?}", s.effects);
    assert!(s.gets.contains(&(1, 1, at)));
    // a vblank whose acquire does not hold: still parked
    let mut sem = 0u64;
    let s = e.vblank(0, &mut |a: &Acquire| {
        assert_eq!((a.handle, a.offset, a.mode), (0xcafe_0a00, 48, 0));
        a.satisfied_by(sem)
    });
    assert!(s.effects.is_empty());
    assert!(e.waiting(1));
    sem = 0xf473_f473;
    let s = e.vblank(1, &mut |a: &Acquire| a.satisfied_by(sem));
    assert!(
        s.effects.is_empty(),
        "another head's vblank latches nothing"
    );
    let s = e.vblank(0, &mut |a: &Acquire| a.satisfied_by(sem));
    // the window had no surface before this flip: the notifier is written, the flip EVENT is not; and
    // its RELEASE is NOT written now — only when this entry is flipped away (the next latch)
    match s.effects.as_slice() {
        [
            Effect::Latched { window: 0 },
            Effect::Notify {
                chn: 1,
                handle: 0xcafe_00f0,
                offset: 16,
                awaken: false,
                ..
            },
        ] => {}
        other => panic!("{other:?}"),
    }
    assert!(s.gets.contains(&(1, 1, w.put())));
    // the next flip (slot 5, value 0xbeef) flips the first away: ITS release (slot 4, 0xd00dd00d) is written now
    let w2 = &mut w;
    w2.m(
        m(WIN, "SET_SEMAPHORE_CONTROL"),
        put(0, fl(WIN, "SET_SEMAPHORE_CONTROL_OFFSET"), 5),
    );
    w2.m(m(WIN, "SET_SEMAPHORE_RELEASE"), 0xbeef);
    w2.m(m(WIN, "UPDATE"), 0);
    let s = e.step(1, &w2.bytes(), w2.put(), &mut all_ok);
    assert!(s.effects.is_empty(), "parked for vblank: {:?}", s.effects);
    let s = e.vblank(0, &mut |a: &Acquire| a.satisfied_by(sem));
    match s.effects.as_slice() {
        [
            Effect::Latched { window: 0 },
            Effect::Release {
                handle: 0xcafe_0b00,
                offset: 64,
                value: 0xd00d_d00d,
                wide: false,
                ..
            },
            // the outgoing entry's notifier FINISHED (its flip-away), then the incoming one's (BEGUN)
            Effect::Notify {
                chn: 1,
                offset: 16,
                finished: true,
                ..
            },
            Effect::Notify {
                chn: 1,
                offset: 16,
                finished: false,
                ..
            },
        ] => {}
        other => panic!("the outgoing entry's release, not the incoming one's: {other:?}"),
    }
}

/// ★ A window's second latch writes the FIRST entry's notifier slot
/// FINISHED (its flip-away) before the new entry's own notifier; the first latch writes only its own.
#[test]
fn a_flip_away_writes_the_outgoing_notifier_finished_and_the_new_one_begun() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    modeset(&mut c, 0, 0);
    c.m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    let mut w = Ring::new();
    w.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x1_0001);
    w.m(m(WIN, "SET_CONTEXT_DMA_NOTIFIER"), 0xcafe_00f0);
    w.m(
        m(WIN, "SET_NOTIFIER_CONTROL"),
        put(0, fl(WIN, "SET_NOTIFIER_CONTROL_OFFSET"), 1),
    );
    w.m(m(WIN, "UPDATE"), 0);
    e.step(1, &w.bytes(), w.put(), &mut all_ok);
    match e.vblank(0, &mut all_ok).effects.as_slice() {
        [
            Effect::Latched { window: 0 },
            Effect::Notify {
                offset: 16,
                finished: false,
                ..
            },
        ] => {}
        other => panic!("{other:?}"),
    }
    w.m(
        m(WIN, "SET_NOTIFIER_CONTROL"),
        put(0, fl(WIN, "SET_NOTIFIER_CONTROL_OFFSET"), 2),
    );
    w.m(m(WIN, "UPDATE"), 0);
    e.step(1, &w.bytes(), w.put(), &mut all_ok);
    match e.vblank(0, &mut all_ok).effects.as_slice() {
        [
            Effect::Latched { window: 0 },
            Effect::Notify {
                offset: 16,
                finished: true,
                ..
            },
            Effect::Notify {
                offset: 32,
                finished: false,
                ..
            },
        ] => {}
        other => panic!("{other:?}"),
    }
}

/// A window on an INACTIVE head (no raster armed) latches at once; so does an immediate flip.
#[test]
fn flips_without_an_active_head_latch_at_once() {
    let mut e = engine();
    e.alloc(ChannelKind::Window, 2, CLIENT, 1, pb(), 0);
    let mut w = Ring::new();
    w.m(m(WIN, "SET_CONTEXT_DMA_NOTIFIER"), 0x77);
    w.m(m(WIN, "UPDATE"), 0);
    let s = e.step(3, &w.bytes(), w.put(), &mut all_ok);
    assert!(
        matches!(
            s.effects.as_slice(),
            [
                Effect::Latched { window: 2 },
                Effect::Notify {
                    chn: 3,
                    handle: 0x77,
                    ..
                }
            ]
        ),
        "{:?}",
        s.effects
    );
}

/// ⊘ Hostile guest: a method outside the method space, a bad opcode and a JUMP cycle each stop
/// the channel by name with GET at the offending header; nothing after it runs; a later PUT does
/// not revive it; a PUT past the ring is refused.
#[test]
fn hostile_streams_stop_the_channel_by_name() {
    let mut e = engine();
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    let mut w = Ring::new();
    w.m(m(WIN, "SET_PRESENT_CONTROL"), 0)
        .m(0x800, 1)
        .m(m(WIN, "UPDATE"), 0);
    let s = e.step(1, &w.bytes(), w.put(), &mut all_ok);
    assert!(
        matches!(
            s.effects.as_slice(),
            [Effect::Exception { chn: 1, at: 8, .. }]
        ),
        "{:?}",
        s.effects
    );
    assert!(s.gets.contains(&(1, 1, 8)));
    let s = e.step(1, &w.bytes(), w.put() + 8, &mut all_ok);
    assert!(s.effects.is_empty(), "halted: nothing runs");
    // bad opcode / runaway / bad pointers on fresh channels
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    let mut bad = vec![0u8; 4096];
    bad[0..4].copy_from_slice(&(7u32 << 29).to_le_bytes());
    assert!(matches!(
        e.step(0, &bad, 4, &mut all_ok).effects.as_slice(),
        [Effect::Exception { .. }]
    ));
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    let mut cyc = vec![0u8; 4096];
    cyc[0..4].copy_from_slice(&(1u32 << 29).to_le_bytes());
    assert!(matches!(
        e.step(0, &cyc, 8, &mut all_ok).effects.as_slice(),
        [Effect::Exception { .. }]
    ));
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    assert!(matches!(
        e.step(0, &[0u8; 4096], 8192, &mut all_ok)
            .effects
            .as_slice(),
        [Effect::Exception { .. }]
    ));
    assert_eq!(e.exceptions, 1);
    // a channel number the display does not have is not allocated
    assert!(
        e.alloc(ChannelKind::Window, 8, CLIENT, 1, pb(), 0)
            .is_none()
    );
    assert!(
        e.alloc(ChannelKind::Cursor, 4, CLIENT, 1, None, 0)
            .is_none()
    );
}

/// A group never waits on a channel that is not allocated (the core interlocks with windows NVKMS
/// has not allocated yet during its first window-mapping update).
#[test]
fn an_unallocated_interlock_target_does_not_block() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    core_notifier(&mut c, 0x1, 0);
    c.m(m(CORE, "SET_WINDOW_INTERLOCK_FLAGS"), 0xFF)
        .m(m(CORE, "UPDATE"), 0);
    let s = e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert!(
        s.effects.iter().any(|x| matches!(x, Effect::Notify { .. })),
        "{:?}",
        s.effects
    );
}

/// Cursor PIO: an Update arms the cursor channel at once; a write outside its space is refused.
#[test]
fn cursor_pio_updates_arm() {
    let mut e = engine();
    e.alloc(ChannelKind::Cursor, 1, CLIENT, 1, None, 0);
    let k = 0xC67A;
    let hot = t().a(k, "SET_CURSOR_HOT_SPOT_POINT_OUT", 0).unwrap();
    e.cursor_write(1, hot, 0x0010_0020, &mut all_ok);
    let s = e.cursor_write(1, m(k, "UPDATE"), 0, &mut all_ok);
    assert!(s.effects.is_empty());
    assert_eq!(e.armed(ChannelKind::Cursor, 1, hot), Some(0x0010_0020));
    let s = e.cursor_write(1, 0x800, 0, &mut all_ok);
    assert!(matches!(s.effects.as_slice(), [Effect::Exception { .. }]));
}

/// The acquire modes: EQ, CGEQ (wrapping), STRICT_GEQ; 32-bit compares ignore the high word.
#[test]
fn acquire_modes() {
    let a = |mode, value, wide| Acquire {
        chn: 1,
        client: 0,
        handle: 1,
        offset: 0,
        value,
        wide,
        mode,
    };
    assert!(a(0, 5, false).satisfied_by(0xFFFF_FFFF_0000_0005));
    assert!(!a(0, 5, false).satisfied_by(6));
    assert!(a(1, 0xFFFF_FFF0, false).satisfied_by(2), "CGEQ wraps");
    assert!(
        !a(2, 0xFFFF_FFF0, false).satisfied_by(2),
        "STRICT_GEQ does not"
    );
    assert!(a(2, 5, true).satisfied_by(5));
    assert!(
        !a(9, 5, true).satisfied_by(5),
        "an unknown mode never holds"
    );
}

/// ★ `[measured m1a, 2026-09-30, GA106 / 580.159.04]` NVKMS kicks PUT to the end of the ring,
/// then writes the wrap JUMP there and kicks PUT = 0: that pass decodes no method at all, and GET
/// must still follow the JUMP to 0 — otherwise NVKMS, filling the ring from 0 up to just below the
/// stuck GET, waits forever (`Error while waiting for GPU progress: 0x0000c67d:0 2:0:4040:4032`,
/// `traces/v3_display/m1a_20260930/`).
#[test]
fn a_jump_only_pass_moves_get_to_its_target() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    let m0 = ma(CORE, "HEAD_SET_VIEWPORT_POINT_IN", 0);
    let mut ring = vec![0u32; 1024];
    let mut at = 0usize;
    while at + 8 <= 4040 {
        ring[at / 4] = (1 << 18) | m0;
        ring[at / 4 + 1] = at as u32;
        at += 8;
    }
    let bytes = |r: &Vec<u32>| -> Vec<u8> { r.iter().flat_map(|w| w.to_le_bytes()).collect() };
    let s = e.step(0, &bytes(&ring), at as u32, &mut all_ok);
    assert_eq!(s.gets, vec![(0, 1, at as u32)]);
    ring[at / 4] = 1 << 29; // JUMP 0
    let s = e.step(0, &bytes(&ring), 0, &mut all_ok);
    assert_eq!(s.gets, vec![(0, 1, 0)], "GET follows the JUMP");
    // and the next pass runs from the ring's start
    ring[0] = (1 << 18) | m0;
    ring[1] = 7;
    let s = e.step(0, &bytes(&ring), 8, &mut all_ok);
    assert_eq!(s.gets, vec![(0, 1, 8)]);
    assert_eq!(e.exceptions, 0);
}

/// ★ `[measured m1b, 2026-09-30, GA106 / 580.159.04]` nvidia-drm queues a flip event only for
/// planes that were active before the commit ("Hardware generates flip event for only those planes
/// which were active previously", `nvidia-drm-modeset.c:93-135`). So: the first flip of a window
/// (no surface before) writes its notifier but raises no AWAKEN; the next flip, with the window
/// scanning on an active head, does.
#[test]
fn the_flip_event_is_raised_only_for_a_window_that_was_active() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    modeset(&mut c, 0, 0);
    c.m(m(CORE, "SET_WINDOW_INTERLOCK_FLAGS"), 1)
        .m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    let flip = |w: &mut Ring, surface: u32| {
        w.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), surface);
        w.m(m(WIN, "SET_CONTEXT_DMA_NOTIFIER"), 0x99);
        w.m(
            m(WIN, "SET_NOTIFIER_CONTROL"),
            put(0, fl(WIN, "SET_NOTIFIER_CONTROL_MODE"), 1),
        );
        w.m(
            m(WIN, "SET_INTERLOCK_FLAGS"),
            put(0, fl(WIN, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CORE"), 1),
        );
        w.m(m(WIN, "UPDATE"), 0);
    };
    // the modeset's own window update (interlocked with the core): no surface before -> no event
    let mut w = Ring::new();
    flip(&mut w, 0x5000);
    let s = e.step(1, &w.bytes(), w.put(), &mut all_ok);
    assert!(
        s.effects.iter().any(|x| matches!(
            x,
            Effect::Notify {
                chn: 1,
                awaken: false,
                ..
            }
        )),
        "{:?}",
        s.effects
    );
    // a plain flip now: previously active -> the event
    let at = w.put();
    w.m(m(WIN, "SET_INTERLOCK_FLAGS"), 0);
    w.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x6000);
    w.m(m(WIN, "UPDATE"), 0);
    let s = e.step(1, &w.bytes(), w.put(), &mut all_ok);
    assert!(s.effects.is_empty(), "parked for vblank");
    let _ = at;
    let s = e.vblank(0, &mut all_ok);
    assert!(
        s.effects.iter().any(|x| matches!(
            x,
            Effect::Notify {
                chn: 1,
                awaken: true,
                ..
            }
        )),
        "{:?}",
        s.effects
    );
}

/// ★ M2: what a head scans out is its lowest enabled window's ARMED surface — the fields NVKMS's
/// `EvoFlipC3Common` programs (`nvkms-evo3.c:3990-4085`), in their raw units.
#[test]
fn a_head_scans_out_its_lowest_enabled_window() {
    let mut e = engine();
    let sv =
        ScanVocab::resolve(t(), WIN, IMM, CORE).expect("GA10x window surfaces are context DMAs");
    assert!(
        ScanVocab::resolve(t(), 0xCA7E, 0xCA7B, 0xCA7D).is_none(),
        "GB20x names surfaces by address (M5)"
    );
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 2, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 3, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    modeset(&mut c, 1, 2);
    c.m(ma(CORE, "WINDOW_SET_CONTROL", 3), 1)
        .m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert_eq!(e.scanout(&sv, 1), None, "no window scans yet");
    let mut w = Ring::new();
    w.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0xabc);
    w.m(ma(WIN, "SET_OFFSET", 0), 0x100);
    w.m(m(WIN, "SET_SIZE"), (1080 << 16) | 1920);
    w.m(m(WIN, "SET_SIZE_IN"), (1080 << 16) | 1920);
    w.m(ma(WIN, "SET_PLANAR_STORAGE", 0), 7680 >> 6);
    w.m(m(WIN, "SET_PARAMS"), m(WIN, "SET_PARAMS_FORMAT_X8R8G8B8"));
    w.m(
        m(WIN, "SET_PRESENT_CONTROL"),
        put(0, fl(WIN, "SET_PRESENT_CONTROL_BEGIN_MODE"), 1),
    );
    w.m(m(WIN, "UPDATE"), 0);
    let s = e.step(4, &w.bytes(), w.put(), &mut all_ok);
    assert!(
        s.effects
            .iter()
            .any(|x| matches!(x, Effect::Latched { window: 3 })),
        "an immediate flip latches at once"
    );
    let so = e.scanout(&sv, 1).expect("head 1 scans window 3");
    assert_eq!(
        (so.window, so.chn, so.handle, so.offset),
        (3, 4, 0xabc, 0x10000)
    );
    assert_eq!(
        (so.width, so.height, so.surface_width, so.pitch),
        (1920, 1080, 1920, 120)
    );
    assert_eq!(so.format, 0xE6, "X8R8G8B8");
    assert_eq!(e.scanout(&sv, 0), None);
}

/// ★ `SYSTEM_GET_ACTIVE`'s source: a head lights the SOR whose ARMED `OWNER_MASK` names it, only
/// while its raster runs; an SOR owned by an idle head lights nothing, and detaching it (owner none)
/// goes dark again.
#[test]
fn a_running_head_lights_the_sor_that_names_it() {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    assert_eq!(e.lit_sors(), vec![None; 4], "nothing at boot");
    let owner = |head_mask: u32| put(0, fl(CORE, "SOR_SET_CONTROL_OWNER_MASK"), head_mask);
    let mut c = Ring::new();
    // SOR 1 names head 2, but head 2 has no raster yet
    c.m(ma(CORE, "SOR_SET_CONTROL", 1), owner(1 << 2))
        .m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert_eq!(
        e.lit_sors(),
        vec![None; 4],
        "an owner without a raster lights nothing"
    );
    modeset(&mut c, 2, 4);
    c.m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert_eq!(e.lit_sors(), vec![None, None, Some(1), None]);
    c.m(ma(CORE, "SOR_SET_CONTROL", 1), owner(0))
        .m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert_eq!(e.lit_sors(), vec![None; 4], "detached");
}

/// ★ M3: a head shows EVERY enabled window it owns, back to front by `DEPTH` (smaller is closer to
/// the front), each where its window-immediate `SET_POINT_OUT` puts it, inside the head's
/// `VIEWPORT_SIZE_IN` — weston puts its clients on the overlay window
/// (`[measured m3g, 2026-09-30, GA106 / 580.159.04]`).
#[test]
fn a_head_composes_its_windows_back_to_front() {
    let mut e = engine();
    let sv = ScanVocab::resolve(t(), WIN, IMM, CORE).unwrap();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 2, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 3, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::WindowImm, 3, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    modeset(&mut c, 1, 2);
    c.m(ma(CORE, "WINDOW_SET_CONTROL", 3), 1)
        .m(
            ma(CORE, "HEAD_SET_VIEWPORT_SIZE_IN", 1),
            (1080 << 16) | 1920,
        )
        .m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    let window = |iso: u32, depth: u32, w: u32, h: u32| {
        let mut r = Ring::new();
        r.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), iso);
        r.m(m(WIN, "SET_SIZE"), (h << 16) | w);
        r.m(m(WIN, "SET_SIZE_IN"), (h << 16) | w);
        r.m(m(WIN, "SET_SIZE_OUT"), (h << 16) | w);
        r.m(
            m(WIN, "SET_COMPOSITION_CONTROL"),
            put(0, fl(WIN, "SET_COMPOSITION_CONTROL_DEPTH"), depth),
        );
        r.m(
            m(WIN, "SET_PRESENT_CONTROL"),
            put(0, fl(WIN, "SET_PRESENT_CONTROL_BEGIN_MODE"), 1),
        );
        r.m(m(WIN, "UPDATE"), 0);
        r
    };
    let back = window(0xa0, 255, 1920, 1080);
    let front = window(0xb0, 0, 250, 250);
    e.step(3, &back.bytes(), back.put(), &mut all_ok);
    e.step(4, &front.bytes(), front.put(), &mut all_ok);
    // ★ D1 (`display-max-fps`): the second tearing flip on head 1 waits for the head's tick
    assert!(e.waiting(4));
    e.vblank(1, &mut all_ok);
    assert!(!e.waiting(4));
    let mut imm = Ring::new();
    imm.m(ma(IMM, "SET_POINT_OUT", 0), (50 << 16) | 100)
        .m(m(IMM, "UPDATE"), 0);
    e.step(36, &imm.bytes(), imm.put(), &mut all_ok);
    let comp = e.composition(&sv, 1).expect("head 1 shows two windows");
    assert_eq!((comp.width, comp.height), (1920, 1080), "the viewport");
    let order: Vec<u32> = comp.layers.iter().map(|l| l.window).collect();
    assert_eq!(order, vec![2, 3], "deepest first");
    let f = &comp.layers[1];
    assert_eq!(
        (f.out_x, f.out_y, f.out_width, f.out_height),
        (100, 50, 250, 250)
    );
    assert_eq!(f.handle, 0xb0);
    assert_eq!(e.composition(&sv, 0), None, "head 0 shows nothing");
}

/// ★ Display step 3d: a head's cursor is read from the ARMED core state (context DMA, offset in
/// 256-byte units, control, composition) and the cursor channel's last `Update` (a signed 16-bit
/// hot-spot point); a head with no enabled cursor has none.
#[test]
fn a_head_cursor_is_scanned_from_the_core_and_its_pio_point() {
    let k = 0xC67A;
    let cv = CursorVocab::resolve(t(), CORE, k).expect("GA10x cursor vocabulary");
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Cursor, 1, CLIENT, 1, None, 0);
    assert_eq!(e.cursor_scan(&cv, 1), None, "nothing programmed");
    let mut ctl = put(0, fl(CORE, "HEAD_SET_CONTROL_CURSOR_ENABLE"), 1);
    ctl = put(
        ctl,
        fl(CORE, "HEAD_SET_CONTROL_CURSOR_FORMAT"),
        m(CORE, "HEAD_SET_CONTROL_CURSOR_FORMAT_A8R8G8B8"),
    );
    ctl = put(
        ctl,
        fl(CORE, "HEAD_SET_CONTROL_CURSOR_SIZE"),
        m(CORE, "HEAD_SET_CONTROL_CURSOR_SIZE_W64_H64"),
    );
    let mut comp = put(0, fl(CORE, "HEAD_SET_CONTROL_CURSOR_COMPOSITION_K1"), 255);
    comp = put(
        comp,
        fl(
            CORE,
            "HEAD_SET_CONTROL_CURSOR_COMPOSITION_CURSOR_COLOR_FACTOR_SELECT",
        ),
        m(
            CORE,
            "HEAD_SET_CONTROL_CURSOR_COMPOSITION_CURSOR_COLOR_FACTOR_SELECT_K1",
        ),
    );
    comp = put(
        comp,
        fl(
            CORE,
            "HEAD_SET_CONTROL_CURSOR_COMPOSITION_VIEWPORT_COLOR_FACTOR_SELECT",
        ),
        m(
            CORE,
            "HEAD_SET_CONTROL_CURSOR_COMPOSITION_VIEWPORT_COLOR_FACTOR_SELECT_NEG_K1_TIMES_SRC",
        ),
    );
    let a2 = |n: &str| t().a2(CORE, n, 1, 0).unwrap_or_else(|| panic!("{n}(1, 0)"));
    let mut c = Ring::new();
    modeset(&mut c, 1, 2);
    c.m(a2("HEAD_SET_CONTEXT_DMA_CURSOR"), 0xcafe)
        .m(a2("HEAD_SET_OFFSET_CURSOR"), 0x20)
        .m(ma(CORE, "HEAD_SET_CONTROL_CURSOR", 1), ctl)
        .m(ma(CORE, "HEAD_SET_CONTROL_CURSOR_COMPOSITION", 1), comp)
        .m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    let hot = t().a(k, "SET_CURSOR_HOT_SPOT_POINT_OUT", 0).unwrap();
    e.cursor_write(1, hot, (40 << 16) | 0xFFFB, &mut all_ok);
    e.cursor_write(1, m(k, "UPDATE"), 0, &mut all_ok);
    let cs = e.cursor_scan(&cv, 1).expect("head 1's cursor is enabled");
    assert_eq!(
        (cs.handle, cs.offset, cs.size, cs.argb8888),
        (0xcafe, 0x2000, 64, true)
    );
    assert_eq!((cs.x, cs.y), (-5, 40), "the point is signed");
    assert_eq!(
        (cs.k1, cs.cursor_factor, cs.viewport_factor, cs.mode),
        (255, 2, 7, 0),
        "NVKMS's premultiplied cursor"
    );
    assert_eq!((cs.client, cs.head), (CLIENT, 1));
    assert_eq!(e.cursor_scan(&cv, 0), None, "head 0 has no cursor");
    assert_eq!(e.cursor_scan(&cv, 9), None, "no such head");
}

/// A window flip: `SET_PRESENT_CONTROL.BEGIN_MODE` = `mode` (0 non-tearing, 1 immediate — what
/// nvidia-drm's async flips program), interlocked with the windows in `with`, then `UPDATE`.
fn flip(r: &mut Ring, mode: u32, with: u32) {
    // a flip shows a surface (an UPDATE with none anywhere is not a flip: `group_ready`)
    r.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x1_0001);
    r.m(
        m(WIN, "SET_PRESENT_CONTROL"),
        put(0, fl(WIN, "SET_PRESENT_CONTROL_BEGIN_MODE"), mode),
    );
    r.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), with);
    r.m(m(WIN, "UPDATE"), 0);
}

/// Head 0 lit (1080p60) with windows 0 and 1 on it; the core's ring for later updates.
fn lit(e: &mut Engine) -> Ring {
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 1, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    modeset(&mut c, 0, 0);
    c.m(ma(CORE, "WINDOW_SET_CONTROL", 1), 0);
    c.m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert!(e.heads_armed()[0].period_ns > 0);
    c
}

fn latched(s: &Step) -> Vec<u32> {
    s.effects
        .iter()
        .filter_map(|x| match x {
            Effect::Latched { window } => Some(*window),
            _ => None,
        })
        .collect()
}

/// ★ D1 (`display-max-fps`, owner decision of 2026-10-04): a TEARING flip on an active head that
/// already presented since the head's tick waits for the next tick — so async flips are bounded by
/// the cap too; the first one after a tick still latches at once (it tears as asked). Another
/// head's tick releases nothing. Known-positive: with the gate off (the mutation) the second flip
/// latches at once, as before the gate.
#[test]
fn a_second_tearing_flip_waits_for_the_vblank() {
    assert!(engine().tear_gate, "on by default (D1)");
    for gate in [true, false] {
        let mut e = engine();
        e.tear_gate = gate;
        lit(&mut e);
        let mut w = Ring::new();
        flip(&mut w, 1, 0);
        let s = e.step(1, &w.bytes(), w.put(), &mut all_ok);
        assert_eq!(
            latched(&s),
            vec![0],
            "gate {gate}: the first latches at once"
        );
        flip(&mut w, 1, 0);
        let s = e.step(1, &w.bytes(), w.put(), &mut all_ok);
        if !gate {
            assert_eq!(latched(&s), vec![0], "without the gate it latches at once");
            assert_eq!(e.pace[0].tear_held, 0);
            assert_eq!(e.pace[0].presents, 2);
            continue;
        }
        assert!(s.effects.is_empty(), "{:?}", s.effects);
        assert!(e.waiting(1));
        assert_eq!(e.pace[0].tear_held, 1);
        assert!(e.vblank(1, &mut all_ok).effects.is_empty(), "head 1's tick");
        assert_eq!(latched(&e.vblank(0, &mut all_ok)), vec![0], "head 0's tick");
        assert!(!e.waiting(1));
        // presented again at that tick: the next waits again
        flip(&mut w, 1, 0);
        assert!(
            e.step(1, &w.bytes(), w.put(), &mut all_ok)
                .effects
                .is_empty()
        );
        assert_eq!(e.pace[0].tear_held, 2);
        // a tick with nothing presented since: the parked one latches, then one at once again
        e.vblank(0, &mut all_ok);
        e.vblank(0, &mut all_ok);
        flip(&mut w, 1, 0);
        assert_eq!(
            latched(&e.step(1, &w.bytes(), w.put(), &mut all_ok)),
            vec![0]
        );
        assert_eq!(e.pace[0].presents, 4);
        assert_eq!(e.pace[0].tearing, 4);
        assert_eq!(e.pace[1], PaceCounts::default(), "nothing on head 1");
    }
}

/// ★ `presents` counts LATCHES, once per head per latch: two windows of one head latched together
/// are one present; a non-tearing flip at its tick is one; a group holding the core latches at
/// once and is counted as `core_imm` too (the tick does not bound it, so the meter must see it).
/// (Mutations: counting per window; a counter blind to core groups.)
#[test]
fn presents_count_one_per_latch_including_core_groups() {
    let mut e = engine();
    let mut c = lit(&mut e);
    assert_eq!(e.pace[0].presents, 0, "a modeset alone presents no window");
    // windows 0 and 1, interlocked, non-tearing: parked, then ONE present at the tick
    let (mut w0, mut w1) = (Ring::new(), Ring::new());
    flip(&mut w0, 0, 1 << 1);
    flip(&mut w1, 0, 1 << 0);
    e.step(1, &w0.bytes(), w0.put(), &mut all_ok);
    assert!(
        e.step(2, &w1.bytes(), w1.put(), &mut all_ok)
            .effects
            .is_empty()
    );
    let s = e.vblank(0, &mut all_ok);
    assert_eq!(latched(&s), vec![0, 1]);
    assert_eq!(e.pace[0].presents, 1, "one latch, two windows");
    assert_eq!(e.pace[0].tearing, 0);
    // window 0 interlocked with the core: latches at once, even right after a present
    w0.m(
        m(WIN, "SET_INTERLOCK_FLAGS"),
        put(0, fl(WIN, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CORE"), 1),
    );
    flip(&mut w0, 0, 0);
    e.step(1, &w0.bytes(), w0.put(), &mut all_ok);
    c.m(m(CORE, "SET_WINDOW_INTERLOCK_FLAGS"), 1 << 0);
    c.m(m(CORE, "UPDATE"), 0);
    let s = e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert_eq!(latched(&s), vec![0], "{:?}", s.effects);
    assert_eq!(e.pace[0].presents, 2);
    assert_eq!(e.pace[0].core_imm, 1);
}

/// ★ F7: a flip parked for a head's tick latches when the head goes IDLE (a core update that stops
/// its raster) — its tick will never come. (Mutation: without the unpark the window stays busy
/// forever, GET before its UPDATE.)
#[test]
fn a_group_parked_on_a_head_that_goes_idle_latches() {
    let mut e = engine();
    let mut c = lit(&mut e);
    let mut w = Ring::new();
    flip(&mut w, 0, 0);
    assert!(
        e.step(1, &w.bytes(), w.put(), &mut all_ok)
            .effects
            .is_empty()
    );
    assert!(e.waiting(1), "parked for head 0's tick");
    // the core stops head 0 (pixel clock 0): the parked flip latches in the same pass
    c.m(ma(CORE, "HEAD_SET_PIXEL_CLOCK_FREQUENCY", 0), 0);
    c.m(m(CORE, "UPDATE"), 0);
    let s = e.step(0, &c.bytes(), c.put(), &mut all_ok);
    assert_eq!(e.heads_armed()[0].period_ns, 0);
    assert_eq!(latched(&s), vec![0], "{:?}", s.effects);
    assert!(!e.waiting(1));
    // one whose acquire does not hold becomes an acquire-only wait (the poll re-checks it)
    let mut e = engine();
    let mut c = lit(&mut e);
    let mut w = Ring::new();
    w.m(m(WIN, "SET_CONTEXT_DMA_ACQ_SEMAPHORE"), 0xcafe_0a00);
    w.m(m(WIN, "SET_ACQ_SEMAPHORE_VALUE"), 7);
    flip(&mut w, 0, 0);
    e.step(1, &w.bytes(), w.put(), &mut all_ok);
    c.m(ma(CORE, "HEAD_SET_PIXEL_CLOCK_FREQUENCY", 0), 0);
    c.m(m(CORE, "UPDATE"), 0);
    let mut no = |_: &Acquire| false;
    e.step(0, &c.bytes(), c.put(), &mut no);
    assert!(e.waiting(1) && e.acquire_pending());
    assert_eq!(latched(&e.poll_acquires(&mut all_ok)), vec![0]);
}

#[test]
fn indexed_color_tables_latch_all_entries_and_reset_with_channel_lifetime() {
    let mut e = engine();
    let n = e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0).unwrap();
    let entry = m(WIN, "SET_CSC0LUT_ENTRY");
    let idx = fl(WIN, "SET_CSC0LUT_ENTRY_IDX");
    let value = fl(WIN, "SET_CSC0LUT_ENTRY_VALUE");
    let mut ring = Ring::new();
    ring.m(entry, put(put(0, idx, 0), value, 12))
        .m(entry, put(put(0, idx, 1), value, 20));
    e.step(n, &ring.bytes(), ring.put(), &mut all_ok);
    assert_eq!(e.armed_inline(0).unwrap()[0].entries[0], None);
    ring.m(m(WIN, "UPDATE"), 0);
    e.step(n, &ring.bytes(), ring.put(), &mut all_ok);
    assert_eq!(
        e.armed_inline(0).unwrap()[0].entries[..2],
        [Some(12), Some(20)]
    );
    ring.m(entry, put(put(0, idx, 0), value, 30));
    e.step(n, &ring.bytes(), ring.put(), &mut all_ok);
    assert_eq!(e.armed_inline(0).unwrap()[0].entries[0], Some(12));
    e.free(ChannelKind::Window, 0);
    e.alloc(ChannelKind::Window, 0, CLIENT, 2, pb(), 0).unwrap();
    assert_eq!(e.armed_inline(0).unwrap()[0].entries[0], None);
}

#[test]
fn indexed_color_table_overflow_stops_before_update() {
    let mut e = engine();
    let n = e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0).unwrap();
    let mut ring = Ring::new();
    ring.m(
        m(WIN, "SET_CSC1LUT_ENTRY"),
        put(0, fl(WIN, "SET_CSC1LUT_ENTRY_IDX"), 1025),
    )
    .m(m(WIN, "UPDATE"), 0);
    let step = e.step(n, &ring.bytes(), ring.put(), &mut all_ok);
    assert!(
        step.effects
            .iter()
            .any(|e| matches!(e, Effect::Exception { .. }))
    );
    assert_eq!(e.updates, 0);
}

/// ⚠ H-corelatch (`Engine::core_latch_at_vblank`, default off): once a head is active, a core
/// UPDATE latches — and states its notifier — at that head's next vblank, not at once; with no
/// active head (the first modeset) it still latches at once. Off, every core update is immediate.
#[test]
fn a_core_update_on_an_active_head_waits_for_the_vblank_only_under_the_experiment() {
    for on in [false, true] {
        let mut e = engine();
        e.core_latch_at_vblank = on;
        e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
        let mut r = Ring::new();
        modeset(&mut r, 0, 0);
        r.m(m(CORE, "UPDATE"), 0);
        let s = e.step(0, &r.bytes(), r.put(), &mut all_ok);
        assert!(
            s.effects.iter().any(|x| matches!(x, Effect::CoreArmed(_))),
            "no active head yet: the modeset latches at once (on={on})"
        );
        assert!(
            e.heads_armed()
                .iter()
                .any(|h| h.head == 0 && h.period_ns > 0)
        );
        core_notifier(&mut r, 0xcafe_0001, 2);
        r.m(m(CORE, "UPDATE"), 0);
        let s = e.step(0, &r.bytes(), r.put(), &mut all_ok);
        let notified = |s: &Step| s.effects.iter().any(|x| matches!(x, Effect::Notify { .. }));
        if on {
            assert!(!notified(&s), "parked for head 0's vblank");
            assert!(
                e.vblank(1, &mut all_ok).effects.is_empty(),
                "another head's tick"
            );
            assert!(
                notified(&e.vblank(0, &mut all_ok)),
                "head 0's tick latches it"
            );
        } else {
            assert!(notified(&s), "default: at once");
        }
    }
}

/// ★ 2026-10-11 (overlay stall H1, runs 408-416; `traces/windows_playback_tdr_20261010/README.md`): the Windows
/// driver's enable sequence for the overlay plane (window 4), as kicked in REAL TIME (PUT writes of run 416, µs
/// apart): core UPDATE; core UPDATE naming window 4 and window 4's ownership; window 4 UPDATE naming the core (A);
/// window 4 UPDATE naming nothing (B, no surface yet); window 0's UPDATE naming window 4; window 0's immediate
/// channel; window 4's immediate channel naming window 4; window 4's flip C naming window 0 and its immediate
/// channel. Each channel's fetch is independent and the hardware sees the writes in the order they were made, so
/// A and B are gone (latched) before window 0's UPDATE arrives, and window 0's and the immediate channel's UPDATEs
/// wait for C and latch with it at the vblank.
struct Overlay {
    e: Engine,
    c: Ring,
    r0: Ring,
    r4: Ring,
    i0: Ring,
    i4: Ring,
    marks: Vec<(u32, u32)>,
}

const W0: u32 = 1;
const I0: u32 = 33;

fn overlay_sequence() -> Overlay {
    let mut e = engine();
    e.alloc(ChannelKind::Core, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::Window, 4, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::WindowImm, 0, CLIENT, 1, pb(), 0);
    e.alloc(ChannelKind::WindowImm, 4, CLIENT, 1, pb(), 0);
    let mut c = Ring::new();
    modeset(&mut c, 0, 0);
    c.m(m(CORE, "UPDATE"), 0);
    e.step(0, &c.bytes(), c.put(), &mut all_ok);
    // window 0 scans a surface (its ring keeps growing)
    let mut r0 = Ring::new();
    r0.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x1_0001);
    r0.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 0);
    r0.m(m(WIN, "UPDATE"), 0);
    e.step(W0, &r0.bytes(), r0.put(), &mut all_ok);
    e.vblank(0, &mut all_ok);
    Overlay {
        e,
        c,
        r0,
        r4: Ring::new(),
        i0: Ring::new(),
        i4: Ring::new(),
        marks: Vec::new(),
    }
}

impl Overlay {
    /// Record the ring writes in the order the guest made them: `(channel, PUT)` after each batch.
    fn kick(&mut self, chn: u32) {
        let put = match chn {
            0 => self.c.put(),
            W0 => self.r0.put(),
            I0 => self.i0.put(),
            5 => self.r4.put(),
            _ => self.i4.put(),
        };
        self.marks.push((chn, put));
    }

    fn build(&mut self) {
        let w4 = ChannelKind::Window.channel_number(4);
        // core UPDATE (alone), then the core naming window 4, owning it for head 0
        self.c.m(m(CORE, "UPDATE"), 0);
        self.kick(0);
        self.c.m(ma(CORE, "WINDOW_SET_CONTROL", 4), 0);
        self.c.m(m(CORE, "SET_WINDOW_INTERLOCK_FLAGS"), 1 << 4);
        self.c.m(m(CORE, "UPDATE"), 0);
        self.kick(0);
        // A: window 4 UPDATE naming the core
        self.r4.m(
            m(WIN, "SET_INTERLOCK_FLAGS"),
            put(0, fl(WIN, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CORE"), 1),
        );
        self.r4.m(m(WIN, "UPDATE"), 0);
        self.kick(w4);
        // B: window 4 UPDATE naming nothing (no surface yet)
        self.r4.m(m(WIN, "SET_INTERLOCK_FLAGS"), 0);
        self.r4.m(m(WIN, "UPDATE"), 0);
        self.kick(w4);
        // window 0's UPDATE naming window 4, and its immediate channel
        self.r0.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 1 << 4);
        self.r0.m(m(WIN, "UPDATE"), 0);
        self.i0.m(m(IMM, "UPDATE"), 0);
        self.kick(I0);
        self.kick(W0);
        // window 4's immediate channel, naming window 4
        self.i4.m(
            m(IMM, "UPDATE"),
            put(0, fl(IMM, "UPDATE_INTERLOCK_WITH_WINDOW"), 1),
        );
        self.kick(ChannelKind::WindowImm.channel_number(4));
        // C: window 4's first flip, naming window 0 and its immediate channel
        self.r4.m(ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x1_0002);
        self.r4.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 1 << 0);
        self.r4.m(
            m(WIN, "UPDATE"),
            put(0, fl(WIN, "UPDATE_INTERLOCK_WITH_WIN_IMM"), 1),
        );
        self.r4.m(m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 0);
        self.kick(w4);
    }

    fn ring_of(&self, chn: u32) -> &Ring {
        match chn {
            0 => &self.c,
            W0 => &self.r0,
            I0 => &self.i0,
            5 => &self.r4,
            _ => &self.i4,
        }
    }
}

/// In ARRIVAL order the overlay's first flip finds its partners: window 4's flip waits for the vblank WITH window 0's
/// UPDATE and its immediate channel's, and all three latch together there.
#[test]
fn the_overlay_enable_sequence_pairs_in_arrival_order() {
    let mut o = overlay_sequence();
    o.build();
    let w4 = ChannelKind::Window.channel_number(4);
    let marks = o.marks.clone();
    for (chn, put) in marks {
        let bytes = o.ring_of(chn).bytes();
        o.e.step(chn, &bytes, put, &mut all_ok);
    }
    assert!(
        o.e.waiting(w4) && o.e.waiting(W0),
        "window 4's flip and window 0's UPDATE wait for the vblank together: {:?}",
        o.e.parked()
    );
    let s = o.e.vblank(0, &mut all_ok);
    let mut l = latched(&s);
    l.sort_unstable();
    assert_eq!(l, vec![0, 4], "{:?}", s.effects);
    assert!(!o.e.waiting(w4), "window 4's flip latched with window 0's: {:?}", o.e.parked());
}

/// The same writes applied the way the display worker did before 2026-10-11 — every channel whose PUT moved, in CHANNEL
/// NUMBER order, each to its LATEST PUT — pair differently: window 0's UPDATE joins window 4's earlier A, the immediate
/// channel's joins B, and flip C is left naming two channels that will not UPDATE again until the next present, which
/// the guest never submits (it is waiting for this one). This is the stall H1 of runs 408-416.
#[test]
fn batched_in_channel_number_order_the_same_writes_leave_the_overlay_flip_unpaired() {
    let mut o = overlay_sequence();
    o.build();
    let w4 = ChannelKind::Window.channel_number(4);
    for chn in [0, W0, w4, I0, ChannelKind::WindowImm.channel_number(4)] {
        let bytes = o.ring_of(chn).bytes();
        let put = o.ring_of(chn).put();
        o.e.step(chn, &bytes, put, &mut all_ok);
    }
    let s = o.e.vblank(0, &mut all_ok);
    assert!(
        o.e.waiting(w4),
        "window 4's flip C is parked with no partner: {:?} / {:?}",
        o.e.parked(),
        s.effects
    );
}
