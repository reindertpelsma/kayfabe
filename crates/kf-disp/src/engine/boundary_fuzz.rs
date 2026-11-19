//! ★ 2026-11-18 (boundary fuzzing, `docs/design/V3_BOUNDARY_FUZZING.md`): the display [`Engine`] against
//! hostile pushbuffers, GPU-free.
//!
//! Every word a display channel's pushbuffer holds is the guest kernel's to choose, and so are `PUT`,
//! the channel's allocation and the cursor PIO writes. The generator is structure-aware: method headers
//! of every opcode (METHOD, JUMP, NONINC, SET_SUBDEVICE_MASK) with counts from 0 to the field's maximum,
//! method offsets drawn from the class table's own rows (every plain method and every indexed method at
//! indices 0..9, so one past the last head and window too), data words that are small, boundary or
//! random, and `PUT` anywhere (unaligned, past the ring, behind `GET`). Interleaved: vblanks on any head,
//! acquire polls answered either way, cursor writes, frees and re-allocations of any instance.
//!
//! Properties: nothing panics; every call returns within a bound; a step's effects and GETs are bounded
//! ([`EFFECT_CAP`]); and a replay of the same script is identical (the effects, step by step).
//!
//! Campaign: `KF_FUZZ_DISP_SEEDS=4000 cargo test --release -p kf-disp --lib boundary_fuzz -- --ignored --nocapture`.

use super::tests::{CLIENT, CORE, WIN, all_ok, engine, lit, pb};
use super::*;
use crate::class::ClassTable;

/// Effects (plus GETs) one call may produce.
const EFFECT_CAP: usize = 4096;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize]
    }
}

/// Every method offset the class table names for `class` (indexed methods at 0..9).
fn methods(t: &ClassTable, class: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for (_, row) in t.rows_under(class, "") {
        match row {
            crate::class::Row::V(v) if v < 0x1_0000 && v % 4 == 0 => out.push(v as u32),
            crate::class::Row::A(b, s) if b < 0x1_0000 => {
                for i in 0..9 {
                    out.push((b + i * s) as u32);
                }
            }
            crate::class::Row::A2(b, s1, s2) if b < 0x1_0000 => {
                for i in 0..3 {
                    for j in 0..3 {
                        out.push((b + i * s1 + j * s2) as u32);
                    }
                }
            }
            _ => {}
        }
    }
    out.retain(|m| m % 4 == 0 && *m < 0x1_0000);
    out.sort_unstable();
    out.dedup();
    out
}

fn data(r: &mut Rng) -> u32 {
    match r.below(6) {
        0 => 0,
        1 => r.below(16) as u32,
        2 => r.pick(&[
            1u32 << 31,
            u32::MAX,
            0x7fff_ffff,
            0xffff,
            0x1_0000,
            4096,
            8192,
            2200,
            1125,
        ]),
        3 => 1 << r.below(32),
        _ => r.next() as u32,
    }
}

/// A 4 KiB-or-64 KiB ring of hostile words.
fn ring(r: &mut Rng, meth: &[u32], len: usize) -> (Vec<u8>, u32) {
    let mut w: Vec<u32> = Vec::new();
    let n = r.below(64) as usize + 1;
    for _ in 0..n {
        let method = if r.below(8) == 0 {
            (r.below(0x4000) as u32) << 2
        } else {
            r.pick(meth)
        };
        match r.below(12) {
            0 => w.push(0),                                               // NOP
            1 => w.push((1 << 29) | ((r.below(len as u64) as u32) & !3)), // JUMP anywhere
            2 => w.push((3 << 29) | data(r)),                             // SET_SUBDEVICE_MASK
            3 => w.push(r.next() as u32),                                 // any word
            4 => {
                // NONINC with a count
                let c = r.below(8) as u32;
                w.push((2 << 29) | (c << 18) | (method >> 2 << 2));
                for _ in 0..c {
                    w.push(data(r));
                }
            }
            5 => {
                // a count past the ring's end
                w.push((0x3ff << 18) | method);
            }
            _ => {
                let c = r.pick(&[1u32, 1, 1, 2, 3, 4, 8]);
                w.push((c << 18) | method);
                for _ in 0..c {
                    w.push(data(r));
                }
            }
        }
    }
    let mut bytes: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
    bytes.resize(len.max(bytes.len()), 0);
    bytes.truncate(len);
    let end = (w.len() * 4).min(len) as u32;
    let put = match r.below(8) {
        0 => r.below(len as u64 + 64) as u32,
        1 => r.next() as u32,
        2 => end.saturating_sub(4 * r.below(4) as u32),
        _ => end,
    };
    (bytes, put)
}

/// One script against a lit engine; the transcript of every call's output.
fn script(seed: u64, rounds: u32) -> Vec<String> {
    let t = crate::class::for_version("580.159.04").expect("table");
    let core = methods(t, CORE);
    let win = methods(t, WIN);
    let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut e = engine();
    lit(&mut e);
    let mut out = Vec::new();
    let mut life = 2u32;
    for _ in 0..rounds {
        let t0 = std::time::Instant::now();
        let mut answer = r.below(2) == 0;
        let mut acq = |_: &Acquire| answer;
        let st = match r.below(12) {
            0..=5 => {
                let core_ch = r.below(3) == 0;
                let (chn, meth) = if core_ch {
                    (0, &core)
                } else {
                    let inst = r.below(9) as u32;
                    (
                        e.channel_number(ChannelKind::Window, inst)
                            .unwrap_or(inst + 1),
                        &win,
                    )
                };
                let chn = if r.below(16) == 0 {
                    r.next() as u32 % 80
                } else {
                    chn
                };
                let len = if r.below(8) == 0 {
                    r.pick(&[0usize, 4, 100, 65536])
                } else {
                    4096
                };
                let (bytes, put) = ring(&mut r, meth, len);
                Some(e.step(chn, &bytes, put, &mut acq))
            }
            6 => Some(e.vblank(r.below(10) as u32, &mut acq)),
            7 => {
                answer = !answer;
                let mut acq = |_: &Acquire| answer;
                Some(e.poll_acquires(&mut acq))
            }
            8 => {
                let head = r.below(10) as u32;
                let off = (r.below(0x2000) as u32) & !3;
                Some(e.cursor_write(head, off, data(&mut r), &mut acq))
            }
            9 => {
                let kind = r.pick(&[
                    ChannelKind::Core,
                    ChannelKind::Window,
                    ChannelKind::WindowImm,
                    ChannelKind::Cursor,
                ]);
                e.free(kind, r.below(40) as u32);
                None
            }
            10 => {
                let kind = r.pick(&[
                    ChannelKind::Core,
                    ChannelKind::Window,
                    ChannelKind::WindowImm,
                    ChannelKind::Cursor,
                ]);
                life += 1;
                let wild = r.next() as u32;
                let off = r.pick(&[0u32, 4, 4092, 4096, u32::MAX, wild]);
                let loc = if r.below(4) == 0 { None } else { pb() };
                e.alloc(kind, r.below(40) as u32, CLIENT, life, loc, off);
                None
            }
            _ => {
                e.halt_scanout();
                e.reset_life();
                lit(&mut e);
                None
            }
        };
        let dt = t0.elapsed();
        assert!(dt.as_secs() < 2, "seed {seed}: one call took {dt:?}");
        if let Some(st) = st {
            assert!(
                st.effects.len() + st.gets.len() <= EFFECT_CAP,
                "seed {seed}: {} effects, {} gets",
                st.effects.len(),
                st.gets.len()
            );
            out.push(format!("{:?} {:?}", st.effects, st.gets));
        }
    }
    let _ = all_ok;
    out
}

/// `(calls with output, calls)`: a coverage sanity figure for the design note.
fn campaign(seeds: std::ops::Range<u64>, rounds: u32) -> (usize, usize) {
    let (mut busy, mut all) = (0, 0);
    for seed in seeds {
        let got = std::panic::catch_unwind(|| script(seed, rounds));
        let t = match got {
            Ok(t) => t,
            Err(e) => panic!(
                "seed {seed}: PANIC {:?}",
                e.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            ),
        };
        if seed % 8 == 0 {
            assert_eq!(script(seed, rounds), t, "seed {seed}: replay differs");
        }
        all += t.len();
        busy += t.iter().filter(|l| l.as_str() != "[] []").count();
    }
    (busy, all)
}

#[test]
fn a_hostile_display_pushbuffer_cannot_break_the_engine() {
    campaign(0..32, 150);
}

#[test]
#[ignore = "campaign: KF_FUZZ_DISP_SEEDS=<n> KF_FUZZ_DISP_ROUNDS=<n> [KF_FUZZ_SECONDS=<s>] cargo test --release -p kf-disp --lib boundary_fuzz -- --ignored --nocapture"]
fn campaign_display_engine() {
    let env = |k: &str, d: u64| {
        std::env::var(k)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(d)
    };
    let seeds = env("KF_FUZZ_DISP_SEEDS", 4000);
    let rounds = env("KF_FUZZ_DISP_ROUNDS", 400) as u32;
    let base = env("KF_FUZZ_BASE", 1000);
    // ★ Time mode: KF_FUZZ_SECONDS=<s> keeps running batches of `seeds` until the deadline.
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(env("KF_FUZZ_SECONDS", 0));
    let (mut busy, mut all, mut next) = (0, 0, base);
    loop {
        let (b, a) = campaign(next..next + seeds, rounds);
        busy += b;
        all += a;
        next += seeds;
        if std::time::Instant::now() >= deadline {
            break;
        }
    }
    eprintln!(
        "campaign: {} scripts x {rounds} calls, no finding; {busy} of {all} calls had effects",
        next - base
    );
}
