//! Independent bit-per-vector oracle. Provenance: NVIDIA open-gpu-kernel-modules
//! 580.65.06 dev_vm.h (TU102/GA102/GH100/GB100), intr_tu102.c:648-744.
//! No production decode, shadow, subtree helper or asserted predicate is reused.
use kf_chip::Family;
use kf_trap::cpuintr::{CpuIntr, Raise, Reg};

const FAMILIES: [(Family, usize); 5] = [
    (Family::Turing, 256),
    (Family::Ampere, 256),
    (Family::Ada, 256),
    (Family::Hopper, 512),
    (Family::Blackwell, 512),
];
const UM: u64 = 0xbb0000;

struct Reference {
    pending: [bool; 512],
    enabled: [bool; 512],
    top: [bool; 32],
    vectors: usize,
}

impl Reference {
    fn new(vectors: usize) -> Self {
        Self {
            pending: [false; 512],
            enabled: [false; 512],
            top: [false; 32],
            vectors,
        }
    }
    fn leaf_asserted(&self, leaf: usize) -> bool {
        (leaf * 32..leaf * 32 + 32).any(|v| self.pending[v] && self.enabled[v] && self.top[v / 64])
    }
    fn latch(&mut self, v: u32) -> Raise {
        let v = v as usize;
        if v >= self.vectors {
            return Raise::OutOfRange;
        }
        self.pending[v] = true;
        if self.leaf_asserted(v / 32) {
            Raise::Message
        } else {
            Raise::None
        }
    }
    fn write(&mut self, r: Reg, value: u32) -> Raise {
        let mut send = false;
        match r {
            Reg::Leaf(i) => {
                for b in 0..32 {
                    if value & (1 << b) != 0 {
                        self.pending[i * 32 + b] = false;
                    }
                }
            }
            Reg::LeafEnSet(i) => {
                for b in 0..32 {
                    let v = i * 32 + b;
                    if value & (1 << b) != 0 {
                        send |= !self.enabled[v] && self.pending[v] && self.top[v / 64];
                        self.enabled[v] = true;
                    }
                }
            }
            Reg::LeafEnClear(i) => {
                for b in 0..32 {
                    if value & (1 << b) != 0 {
                        self.enabled[i * 32 + b] = false;
                    }
                }
            }
            Reg::TopEnSet => {
                for t in 0..32 {
                    if value & (1 << t) != 0 {
                        send |= !self.top[t]
                            && (0..self.vectors)
                                .any(|v| v / 64 == t && self.pending[v] && self.enabled[v]);
                        self.top[t] = true;
                    }
                }
            }
            Reg::TopEnClear => {
                for t in 0..32 {
                    if value & (1 << t) != 0 {
                        self.top[t] = false;
                    }
                }
            }
            Reg::Trigger => return self.latch(value & 4095),
            Reg::Top => {}
        }
        if send { Raise::Message } else { Raise::None }
    }
    fn read(&self, r: Reg) -> u32 {
        let pack = |xs: &[bool]| {
            xs.iter()
                .enumerate()
                .fold(0, |w, (b, x)| w | (u32::from(*x) << b))
        };
        match r {
            Reg::Leaf(i) => pack(&self.pending[i * 32..i * 32 + 32]),
            Reg::LeafEnSet(i) | Reg::LeafEnClear(i) => pack(&self.enabled[i * 32..i * 32 + 32]),
            Reg::Top => {
                (0..self.vectors).fold(0, |w, v| w | (u32::from(self.pending[v]) << (v / 64)))
            }
            Reg::TopEnSet | Reg::TopEnClear => pack(&self.top),
            Reg::Trigger => 0,
        }
    }
    fn check(&self, actual: &CpuIntr) {
        for i in 0..self.vectors / 32 {
            for (r, off) in [
                (Reg::Leaf(i), 0xb81000),
                (Reg::LeafEnSet(i), 0xb81200),
                (Reg::LeafEnClear(i), 0xb81400),
            ] {
                assert_eq!(actual.decode(off + i as u64 * 4), Some(r));
                assert_eq!(actual.read(r), self.read(r), "{r:?}");
                let mut shadow = Vec::new();
                actual.shadow(r, |o, v| shadow.push((o, v)));
                for (o, v) in shadow {
                    assert_eq!(v, self.read(decode(o)), "shadow {o:x}");
                }
            }
        }
        for r in [Reg::Top, Reg::TopEnSet, Reg::TopEnClear, Reg::Trigger] {
            assert_eq!(actual.read(r), self.read(r), "{r:?}");
        }
        actual.shadow_all(|o, v| assert_eq!(v, self.read(decode(o)), "all {o:x}"));
    }
}

fn decode(o: u64) -> Reg {
    match o {
        0xb81600 => Reg::Top,
        0xb81608 => Reg::TopEnSet,
        0xb81610 => Reg::TopEnClear,
        0xb81640 => Reg::Trigger,
        0xb81000..=0xb8103c => Reg::Leaf(((o - 0xb81000) / 4) as usize),
        0xb81200..=0xb8123c => Reg::LeafEnSet(((o - 0xb81200) / 4) as usize),
        0xb81400..=0xb8143c => Reg::LeafEnClear(((o - 0xb81400) / 4) as usize),
        _ => panic!("unexpected shadow offset {o:x}"),
    }
}

fn random_run(mut seed: u64) {
    for (family, vectors) in FAMILIES {
        let actual = CpuIntr::new(family, UM).unwrap();
        let mut reference = Reference::new(vectors);
        for step in 0..12_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let i = (seed as usize / 16) % (vectors / 32);
            let value = if step % 3 == 0 {
                1 << ((seed >> 32) % 32)
            } else {
                (seed >> 32) as u32
            };
            match seed % 10 {
                0 => assert_eq!(
                    actual.latch((seed >> 24) as u32 % 530),
                    reference.latch((seed >> 24) as u32 % 530)
                ),
                1 => {
                    actual.reset();
                    reference = Reference::new(vectors);
                }
                n => {
                    let r = match n {
                        2 => Reg::LeafEnSet(i),
                        3 => Reg::LeafEnClear(i),
                        4 => Reg::TopEnSet,
                        5 => Reg::TopEnClear,
                        6 => Reg::Leaf(i),
                        7 => Reg::Trigger,
                        8 => Reg::Top,
                        _ => Reg::LeafEnSet(i),
                    };
                    assert_eq!(
                        actual.write(r, value),
                        reference.write(r, value),
                        "{family:?} step {step} {r:?}={value:x}"
                    );
                }
            }
            reference.check(&actual);
        }
    }
}

#[test]
fn differential_random_sparse_and_dense_seed_one() {
    random_run(0x99817abcfe);
}
#[test]
fn differential_random_sparse_and_dense_seed_two() {
    random_run(0x927461239abc);
}

#[test]
fn exhaustive_pending_leaf_top_and_all_six_orders_every_vector() {
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for (family, vectors) in FAMILIES {
        for v in 0..vectors {
            for initial in 0..8 {
                for order in orders {
                    let t = CpuIntr::new(family, UM).unwrap();
                    let mut p = initial & 1 != 0;
                    let mut l = initial & 2 != 0;
                    let mut top = initial & 4 != 0;
                    let mut count = 0;
                    t.write(Reg::LeafEnSet(v / 32), if l { 1 << (v % 32) } else { 0 });
                    t.write(Reg::TopEnSet, if top { 1 << (v / 64) } else { 0 });
                    if p {
                        count += usize::from(t.latch(v as u32) == Raise::Message);
                    }
                    for action in order {
                        let before = p && l && top;
                        let got = match action {
                            0 if !p => {
                                p = true;
                                t.latch(v as u32)
                            }
                            0 => Raise::None,
                            1 => {
                                l = true;
                                t.write(Reg::LeafEnSet(v / 32), 1 << (v % 32))
                            }
                            _ => {
                                top = true;
                                t.write(Reg::TopEnSet, 1 << (v / 64))
                            }
                        };
                        let expected = if !before && p && l && top {
                            Raise::Message
                        } else {
                            Raise::None
                        };
                        assert_eq!(
                            got, expected,
                            "{family:?} v{v} initial{initial} order{order:?}"
                        );
                        count += usize::from(got == Raise::Message);
                    }
                    assert_eq!(count, 1);
                    assert_eq!(t.write(Reg::Leaf(v / 32), 1 << (v % 32)), Raise::None);
                    assert_eq!(t.read(Reg::Leaf(v / 32)), 0);
                    assert_eq!(t.read(Reg::Top), 0);
                }
            }
        }
    }
}

#[test]
fn reset_discards_pending_and_both_enable_aliases_every_family() {
    for (f, n) in FAMILIES {
        let t = CpuIntr::new(f, UM).unwrap();
        for v in 0..n {
            t.latch(v as u32);
        }
        for i in 0..n / 32 {
            t.write(Reg::LeafEnSet(i), u32::MAX);
        }
        t.write(Reg::TopEnSet, u32::MAX);
        t.reset();
        Reference::new(n).check(&t);
        assert_eq!(t.write(Reg::TopEnSet, u32::MAX), Raise::None);
        for i in 0..n / 32 {
            assert_eq!(t.write(Reg::LeafEnSet(i), u32::MAX), Raise::None);
        }
    }
}
