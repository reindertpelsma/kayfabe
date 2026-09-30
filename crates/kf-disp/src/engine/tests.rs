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
    // the window had no surface before this flip: the notifier is written, the flip EVENT is not
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

/// ★ `[measured m1a]` NVKMS kicks PUT to the end of the ring, then writes the wrap JUMP there and
/// kicks PUT = 0: that pass decodes no method at all, and GET must still follow the JUMP to 0 —
/// otherwise NVKMS, filling the ring from 0 up to just below the stuck GET, waits forever
/// (`Error while waiting for GPU progress: 0x0000c67d:0 2:0:4040:4032`).
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

/// ★ `[measured m1b]` nvidia-drm queues a flip event only for planes that were active before the
/// commit ("Hardware generates flip event for only those planes which were active previously",
/// `nvidia-drm-modeset.c:93-135`). So: the first flip of a window (no surface before) writes its
/// notifier but raises no AWAKEN; the next flip, with the window scanning on an active head, does.
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
