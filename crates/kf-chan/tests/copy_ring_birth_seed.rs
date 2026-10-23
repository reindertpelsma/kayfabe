//! CE birth regression. FALSIFIER: a dedicated-copy ring still needs SET_OBJECT,
//! or the graphics/video controls, class pin, or existing address perimeter widen.
//! Public headers from Q3 runs 4304/4310; every payload and GP 0..33 is synthetic.
use kf_abi::submit::{ce, gp_entry, method_header_decode, method_header_inc};
use kf_chan::ring::{GuestMemory, Next, RingRefusal, TranslatedRing};
use kf_chan::tmode::{GrConfig, Ir, Rows, Span, bind};
use kf_chan::translated::{Refusal, Target, Window};
use kf_chan::tspace_unsafe::TWindows;
use kf_chan::ttables::Tier;
use kf_host::MapPerm;
use std::collections::BTreeMap;

const GPFIFO: u64 = 0x10_0000;
const PB: u64 = 0x20_0000;
const SEMA: u64 = 0x1_2050_f060;
const HOST_CE: u32 = 0xc7b5;
const FB: u64 = 0x10_0000_0000;

fn birth_config(engine: u32, video: u32, host_ce: u32, tier: bool) -> GrConfig {
    kf_chan::tmode::birth_decoder_config(engine, video, host_ce, tier)
}

fn packet(method: u32, values: &[u32]) -> Vec<u32> {
    let mut words = vec![method_header_inc(4, method, values.len() as u32).unwrap()];
    words.extend(values);
    words
}

fn semaphore() -> Vec<u32> {
    let mut words = packet(0x240, &[1, 0x2050_f060, 1]);
    words.extend(packet(0x300, &[(1 << 2) | (1 << 3) | (2 << 5)]));
    assert_eq!(words[0], 0x2003_8090);
    assert_eq!(words[4], 0x2001_80c0);
    words
}

#[derive(Default)]
struct Memory(BTreeMap<u64, u8>);
impl Memory {
    fn put(&mut self, va: u64, bytes: &[u8]) {
        for (i, byte) in bytes.iter().enumerate() {
            self.0.insert(va + i as u64, *byte);
        }
    }
}
impl GuestMemory for Memory {
    fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String> {
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = *self.0.get(&(va + i as u64)).ok_or("unmapped test VA")?;
        }
        Ok(())
    }
}
struct NoLegacyWindow;
impl Window for NoLegacyWindow {
    fn translate(&self, _: Target, _: u64, _: u64) -> Option<u64> {
        panic!("T-mode must hand out unbound IR")
    }
}

fn replay(config: GrConfig, words: &[u32], gp: u32) -> Result<Next, RingRefusal> {
    let mut ring = TranslatedRing::new_tmode(GPFIFO, 64, 0);
    ring.set_gr(config);
    let mut mem = Memory::default();
    for slot in 0..gp {
        mem.put(GPFIFO + 8 * u64::from(slot), &0u64.to_le_bytes());
    }
    let bytes: Vec<_> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    mem.put(PB, &bytes);
    mem.put(
        GPFIFO + 8 * u64::from(gp),
        &gp_entry(PB, bytes.len() as u64).unwrap().to_le_bytes(),
    );
    for retired in 1..=gp {
        assert_eq!(
            ring.next(gp + 1, &mut mem, is_ce, &NoLegacyWindow).unwrap(),
            Next::Submit {
                words: vec![],
                retires: Some(retired)
            }
        );
    }
    let next = ring.next(gp + 1, &mut mem, is_ce, &NoLegacyWindow);
    assert_eq!(ring.entries_fetched(), u64::from(gp + 1));
    next
}
fn is_ce(class: u32) -> bool {
    kf_chip::is_any_dma_copy_class(class)
}
fn items(config: GrConfig, words: &[u32], gp: u32) -> Vec<Ir> {
    match replay(config, words, gp).expect("host CE object exists at birth") {
        Next::Bind { ir, retires } => {
            assert_eq!(retires, Some(gp + 1));
            ir
        }
        other => panic!("expected unbound work: {other:?}"),
    }
}
fn launch(ir: &[Ir]) -> &kf_chan::tmode::CeLaunch {
    match ir {
        [Ir::Launch(l)] => l,
        other => panic!("unexpected IR: {other:?}"),
    }
}
fn copy_config() -> GrConfig {
    birth_config(0xb, 0, HOST_CE, false)
}

#[test]
fn a_copy_runlist_kernel_ring_admits_the_engine_semaphore_without_set_object() {
    let ir = items(copy_config(), &semaphore(), 34);
    let l = launch(&ir);
    assert_eq!(l.tier, Tier::C7b5);
    assert!(l.src.is_none() && l.dst.is_none());
    assert_eq!((l.sema.unwrap().va, l.sema.unwrap().bytes), (SEMA, 4));
}

#[test]
fn another_host_ce_tier_is_seeded_at_the_first_entry() {
    let ir = items(birth_config(0x9, 0, 0xc8b5, false), &semaphore(), 0);
    assert_eq!(launch(&ir).tier, Tier::C8b5);
}

fn unseeded_graphics(tier: bool) {
    assert_eq!(
        replay(birth_config(1, 0, HOST_CE, tier), &semaphore(), 0),
        Err(RingRefusal::Rewrite {
            gp: 0,
            why: Refusal::NoCeObject {
                subch: 4,
                method: 0x240
            }
        })
    );
}
#[test]
fn graphics_without_gr_tier_stays_unseeded() {
    unseeded_graphics(false);
}
#[test]
fn graphics_with_gr_tier_stays_unseeded() {
    unseeded_graphics(true);
}

fn video_wins(class: u32) {
    let config = birth_config(0xb, class, HOST_CE, false);
    assert_eq!(
        config,
        GrConfig {
            video: class,
            ..Default::default()
        },
        "video excludes a CE seed"
    );
    let mut words = packet(0x240, &[1, 0x2050_f060, 1]);
    words.extend(packet(0x304, &[0]));
    let ir = items(config, &words, 0);
    assert!(
        ir.iter().any(|i| matches!(i, Ir::VideoSem(_))),
        "video semaphore retained"
    );
    assert!(!ir.iter().any(|i| matches!(i, Ir::Launch(_))));
}
#[test]
fn nvdec_birth_wins_over_the_host_ce_hint() {
    video_wins(0xc9b0);
}
#[test]
fn nvenc_birth_wins_over_the_host_ce_hint() {
    video_wins(0xc9b7);
}

fn foreign(class: u32, sub: u32) {
    let words = [method_header_inc(sub, 0, 1).unwrap(), class];
    assert_eq!(
        replay(copy_config(), &words, 0),
        Err(RingRefusal::Rewrite {
            gp: 0,
            why: Refusal::ForeignClass { subch: sub, class }
        })
    );
}
#[test]
fn a_guest_cannot_rebind_to_a_wider_ce_class() {
    foreign(0xc8b5, 4);
}
#[test]
fn a_guest_cannot_rebind_another_hardware_subchannel() {
    foreign(0xc9b5, 0);
}
#[test]
fn a_guest_cannot_rebind_ce_on_a_software_subchannel() {
    foreign(0xc8b5, 6);
}
#[test]
fn a_guest_cannot_rebind_to_graphics() {
    foreign(0xc797, 4);
}
#[test]
fn the_own_host_class_can_still_be_bound_explicitly() {
    let mut words = packet(0, &[HOST_CE]);
    words.extend(semaphore());
    let ir = items(copy_config(), &words, 0);
    assert!(
        ir.iter()
            .any(|i| matches!(i, Ir::Launch(l) if l.tier == Tier::C7b5))
    );
}

struct Placement {
    mapped: bool,
}
impl Rows for Placement {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<Span>, u64> {
        if self.mapped && va == SEMA && len == 4 {
            Ok(vec![Span {
                ram: false,
                off: 0x1000,
                len,
                perm: MapPerm::READ_WRITE,
            }])
        } else {
            Err(va)
        }
    }
    fn dma_to_file_range(&self, _: u64, _: u64) -> Option<u64> {
        None
    }
}
fn windows() -> TWindows {
    TWindows::new((FB, 0x1000_0000), (0x20_0000_0000, 0x1000_0000), 1 << 40).unwrap()
}
fn output_writes(words: &[u32]) -> Vec<(u32, u32)> {
    words
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| (method_header_decode(p[0]).unwrap().method, p[1]))
        .collect()
}
#[test]
fn a_seeded_semaphore_is_bound_to_authored_window_words() {
    let ir = items(copy_config(), &semaphore(), 0);
    let mut out = vec![];
    bind(&ir[0], &Placement { mapped: true }, &windows(), &mut out).unwrap();
    let wr = output_writes(&out);
    assert!(wr.contains(&(0x240, (FB >> 32) as u32)));
    assert!(wr.contains(&(0x244, 0x1000)));
    assert!(!out.contains(&0x2050_f060), "guest VA is never emitted");
}
#[test]
fn a_seeded_semaphore_outside_the_vm_rows_is_refused() {
    let ir = items(copy_config(), &semaphore(), 0);
    let mut out = vec![];
    assert!(bind(&ir[0], &Placement { mapped: false }, &windows(), &mut out).is_err());
    assert!(out.is_empty());
}

fn physical_copy(dst: u32) {
    let mut words = packet(ce::OFFSET_IN_UPPER, &[0, 0x2000, 0, dst]);
    words.extend(packet(ce::LINE_LENGTH_IN, &[16]));
    words.extend(packet(ce::SET_SRC_PHYS_MODE, &[0]));
    words.extend(packet(ce::SET_DST_PHYS_MODE, &[0]));
    words.extend(packet(
        ce::LAUNCH_DMA,
        &[2 | (1 << 7) | (1 << 8) | (1 << 12) | (1 << 13)],
    ));
    let ir = items(copy_config(), &words, 0);
    let mut out = vec![];
    bind(&ir[0], &Placement { mapped: false }, &windows(), &mut out).unwrap();
    let wr = output_writes(&out);
    assert!(wr.contains(&(ce::OFFSET_IN_UPPER, (FB >> 32) as u32)));
    assert!(wr.contains(&(ce::OFFSET_OUT_UPPER, (FB >> 32) as u32)));
    let emitted = wr.iter().find(|p| p.0 == ce::LAUNCH_DMA).unwrap().1;
    assert_eq!(
        emitted & ((1 << 12) | (1 << 13)),
        0,
        "host operands are virtual"
    );
}
#[test]
fn physical_operands_still_translate_into_the_window() {
    physical_copy(0x4000);
}
#[test]
fn another_physical_destination_still_translates() {
    physical_copy(0x8000);
}

// Observe the existing public Debug surface until the typed counter is introduced.
fn seed_count(st: &kf_chan::tmode::TState, expected: u64) {
    let shown = format!("{st:?}");
    assert!(
        shown.contains(&format!("seeded_launches: {expected},")),
        "{shown}"
    );
}
fn state() -> kf_chan::tmode::TState {
    let mut st = kf_chan::tmode::TState::default();
    st.gr = copy_config();
    st.tier = Tier::of_ce_class(st.gr.ce);
    st
}
#[test]
fn seeded_launch_counter_observes_the_first_unbound_launch() {
    let mut st = state();
    let ir = kf_chan::tmode::decode(&semaphore(), is_ce, &mut st, None).unwrap();
    assert_eq!(launch(&ir).tier, Tier::C7b5);
    seed_count(&st, 1);
}
#[test]
fn seeded_launch_counter_observes_each_unbound_launch() {
    let mut st = state();
    for _ in 0..2 {
        kf_chan::tmode::decode(&semaphore(), is_ce, &mut st, None).unwrap();
    }
    seed_count(&st, 2);
}
#[test]
fn an_explicit_host_bind_does_not_count_as_a_seeded_launch() {
    let mut st = state();
    let mut words = packet(0, &[HOST_CE]);
    words.extend(semaphore());
    kf_chan::tmode::decode(&words, is_ce, &mut st, None).unwrap();
    seed_count(&st, 0);
}
#[test]
fn an_explicit_bind_in_a_later_segment_preserves_the_prior_seed_count() {
    let mut st = state();
    kf_chan::tmode::decode(&semaphore(), is_ce, &mut st, None).unwrap();
    kf_chan::tmode::decode(&packet(0, &[HOST_CE]), is_ce, &mut st, None).unwrap();
    kf_chan::tmode::decode(&semaphore(), is_ce, &mut st, None).unwrap();
    seed_count(&st, 1);
}
#[test]
fn a_refused_launch_does_not_count_as_seeded_work() {
    let mut st = state();
    let mut words = packet(0x240, &[1, 0x2050_f061, 1]);
    words.extend(packet(0x300, &[0x4c]));
    assert!(kf_chan::tmode::decode(&words, is_ce, &mut st, None).is_err());
    seed_count(&st, 0);
}
