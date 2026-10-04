// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The GPU-copy rung's decisions, GPU-free** (`docs/design/V3_DISPLAY.md` §8.11,
//! `OWNER_RULINGS.md` §L) — what the display worker asks once per frame, kept here so a test
//! drives every branch without a GPU, a render node or a VMM:
//!
//! - [`VramMode`] — the `display-broker-vram=auto|on|off` property;
//! - [`plan`] — which copies a frame gets: the PACK into a VRAM slot (rung 0), the D2H copy into
//!   host memory (the console, and the host rungs), both, or neither;
//! - [`Provisioning`] — when to ask for VRAM slots: the five class-0 slots at the first explicit
//!   yes (with `auto`; at realize with `on`), and the one growth per slot to the largest size
//!   (`kf_disp::vramslot::SLOT_MAX`) when a frame needs more — each asked at most once, never on
//!   the frame path.
//!
//! ⊘ **Two demand signals, not one** (the adversarial review of the design, 2026-10-03): frames are
//! WANTED (the refresh clock's 33 ms instead of 250 ms) when the console or an active broker asks;
//! the HOST copy is wanted only when the console asks or a broker must be fed through host memory.
//! Merging them either loses the refresh rate on the broker (a front-buffer-rendering guest drops
//! to 4 Hz) or keeps a GPU→CPU copy running while the broker alone shows the VM.

/// ★ `display-broker-vram`: whether kf3 allocates VRAM frame slots for the GPU-copy rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VramMode {
    /// Probe at realize (render node, `GET_DEV_INFO`, the ABI gate); allocate the slots on a
    /// provisioning thread at the first explicit yes for the block-linear pair. A host whose
    /// compositor never says yes spends no VRAM. The default.
    #[default]
    Auto,
    /// Allocate (and self-test) the class-0 slots at realize; a refusal fails realize.
    On,
    /// Never allocate: the host-memory rungs only.
    Off,
}

impl VramMode {
    /// The property's spelling.
    ///
    /// # Errors
    /// Anything but `auto`, `on`, `off`, by name.
    pub fn parse(s: &str) -> Result<VramMode, String> {
        match s {
            "auto" => Ok(VramMode::Auto),
            "on" => Ok(VramMode::On),
            "off" => Ok(VramMode::Off),
            other => Err(format!(
                "display-broker-vram={other:?}: the values are auto, on and off"
            )),
        }
    }

    /// ★ KF3 ABI 12 (the broker's one bump above master's 11; the branch called it 13 before the
    /// merge): `kf3_realize`'s `display_broker` word carries the broker in bit 0 and this
    /// mode in bits 1-2 (0 auto, 1 on, 2 off). `None` for the broker off; an error for a mode
    /// value 3 or any bit above 2.
    ///
    /// # Errors
    /// An undefined encoding, by name.
    pub fn from_abi(word: u32) -> Result<Option<VramMode>, String> {
        if word & !0b111 != 0 {
            return Err(format!(
                "display_broker word {word:#x}: bits above 2 are undefined"
            ));
        }
        if word & 1 == 0 {
            return if word == 0 {
                Ok(None)
            } else {
                Err(format!(
                    "display_broker word {word:#x}: display-broker-vram without display-broker"
                ))
            };
        }
        match (word >> 1) & 0b11 {
            0 => Ok(Some(VramMode::Auto)),
            1 => Ok(Some(VramMode::On)),
            2 => Ok(Some(VramMode::Off)),
            _ => Err(format!(
                "display_broker word {word:#x}: mode 3 is undefined"
            )),
        }
    }

    /// The ABI word for a broker in this mode (the C device's encoding, for tests).
    #[must_use]
    pub fn abi_word(self) -> u32 {
        1 | (match self {
            VramMode::Auto => 0,
            VramMode::On => 1,
            VramMode::Off => 2,
        } << 1)
    }
}

/// What the worker knows when a frame is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Choice {
    /// A broker seat exists and was active recently.
    pub broker: bool,
    /// The relay would take a VRAM frame now ([`crate::FrameRing::want_vram`]).
    pub want_vram: bool,
    /// A free VRAM slot can take THIS frame (provisioned, large enough, its fence idle).
    pub vram_slot: bool,
    /// The console asked for a frame recently.
    pub console: bool,
    /// The host kind is withdrawn from the broker.
    pub host_withdrawn: bool,
    /// The VRAM kind is withdrawn from the broker.
    pub vram_withdrawn: bool,
}

/// The copies one frame gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plan {
    /// The pack into a VRAM slot (rung 0).
    pub pack: bool,
    /// The D2H copy into the slot's host frame.
    pub d2h: bool,
}

impl Plan {
    /// Neither copy: the frame is not made (its flips still complete).
    #[must_use]
    pub fn none(&self) -> bool {
        !self.pack && !self.d2h
    }
}

/// ★ Which copies a frame gets:
/// - **pack** when a broker is active, wants VRAM, the VRAM kind is not withdrawn, and a VRAM
///   slot can take the frame;
/// - **D2H** when the console asked, when an active broker must be fed through host memory
///   (no pack, host kind not withdrawn), or when NOBODY asked (the 4 Hz refresh keeps a
///   screendump recent, as before the broker) — and never only because the broker is active
///   while it takes the GPU copy: then no byte crosses to the CPU.
#[must_use]
pub fn plan(c: &Choice) -> Plan {
    let pack = c.broker && c.want_vram && !c.vram_withdrawn && c.vram_slot;
    let d2h = c.console || (c.broker && !pack && !c.host_withdrawn) || (!c.broker && !c.console);
    Plan { pack, d2h }
}

/// One provisioning request: allocate a VRAM object of `bytes` for each slot in `slots`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The ring slots to back.
    pub slots: Vec<usize>,
    /// Each object's size.
    pub bytes: u64,
}

/// ★ When to ask the provisioning thread for slots — at most once per size class and slot, and
/// never again after a refusal (the VRAM kind is then withdrawn).
#[derive(Debug, Clone)]
pub struct Provisioning {
    mode: VramMode,
    slots: usize,
    class0: u64,
    max: u64,
    /// Per slot: the largest object asked for (0 = none).
    asked: Vec<u64>,
    refused: bool,
}

impl Provisioning {
    /// A plan over `slots` ring slots, first at `class0` bytes, growing once to `max`.
    #[must_use]
    pub fn new(mode: VramMode, slots: usize, class0: u64, max: u64) -> Provisioning {
        Provisioning {
            mode,
            slots,
            class0,
            max,
            asked: vec![0; slots],
            refused: false,
        }
    }

    /// ★ The class-0 slots — asked at realize with `on` (`realize` = true), at the first
    /// `want_vram` with `auto`, never with `off` or after a refusal.
    pub fn first(&mut self, realize: bool, want_vram: bool) -> Option<Request> {
        let due = match self.mode {
            VramMode::Off => false,
            VramMode::On => realize,
            VramMode::Auto => want_vram && !realize,
        };
        if !due || self.refused || self.asked.iter().any(|a| *a != 0) {
            return None;
        }
        self.ask(self.class0)
    }

    /// ★ A frame needs `extent` bytes and no slot was asked that large: one growth to `max` for
    /// every slot (a mode change moves every frame), once. `None` when no size can hold it.
    pub fn grow(&mut self, extent: u64) -> Option<Request> {
        if self.mode == VramMode::Off
            || self.refused
            || extent > self.max
            || self.asked.iter().all(|a| *a == 0)
            || self.asked.iter().all(|a| *a >= extent)
        {
            return None;
        }
        self.ask(self.max)
    }

    fn ask(&mut self, bytes: u64) -> Option<Request> {
        let slots: Vec<usize> = (0..self.slots).filter(|&j| self.asked[j] < bytes).collect();
        if slots.is_empty() {
            return None;
        }
        for &j in &slots {
            self.asked[j] = bytes;
        }
        Some(Request { slots, bytes })
    }

    /// A provisioning step was refused: no request is made again.
    pub fn refuse(&mut self) {
        self.refused = true;
    }

    /// Whether a refusal ended provisioning.
    #[must_use]
    pub fn refused(&self) -> bool {
        self.refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_property_and_the_abi_word_say_the_same_three_things() {
        for (s, m) in [
            ("auto", VramMode::Auto),
            ("on", VramMode::On),
            ("off", VramMode::Off),
        ] {
            assert_eq!(VramMode::parse(s), Ok(m));
            assert_eq!(VramMode::from_abi(m.abi_word()), Ok(Some(m)));
        }
        assert_eq!(VramMode::default(), VramMode::Auto);
        assert!(VramMode::parse("yes").is_err());
        assert!(VramMode::parse("").is_err());
        assert_eq!(
            VramMode::from_abi(0),
            Ok(None),
            "`0`: the broker off (as before the vram bits)"
        );
        assert_eq!(
            VramMode::from_abi(1),
            Ok(Some(VramMode::Auto)),
            "`1`: auto (as before the vram bits)"
        );
        for bad in [0b111, 0b1000, 0b100, 0b10, u32::MAX] {
            assert!(VramMode::from_abi(bad).is_err(), "{bad:#b}");
        }
    }

    /// ★ Every branch of the plan, including the two the review named: an active broker on the
    /// GPU copy with the console idle makes NO D2H copy; and with nobody asking the 4 Hz
    /// screendump copy goes on.
    #[test]
    fn the_plan_packs_for_the_broker_and_copies_to_the_host_only_when_someone_needs_it() {
        let native = Choice {
            broker: true,
            want_vram: true,
            vram_slot: true,
            ..Choice::default()
        };
        assert_eq!(
            plan(&native),
            Plan {
                pack: true,
                d2h: false
            },
            "no byte to the CPU"
        );
        assert_eq!(
            plan(&Choice {
                console: true,
                ..native
            }),
            Plan {
                pack: true,
                d2h: true
            }
        );
        for off in [
            Choice {
                want_vram: false,
                ..native
            },
            Choice {
                vram_slot: false,
                ..native
            },
            Choice {
                vram_withdrawn: true,
                ..native
            },
        ] {
            assert_eq!(
                plan(&off),
                Plan {
                    pack: false,
                    d2h: true
                },
                "{off:?}: the host rungs"
            );
        }
        assert!(
            plan(&Choice {
                want_vram: false,
                host_withdrawn: true,
                ..native
            })
            .none(),
            "both kinds gone and no console: no copy"
        );
        assert_eq!(
            plan(&Choice::default()),
            Plan {
                pack: false,
                d2h: true
            },
            "nobody asks: the 4 Hz screendump copy"
        );
        assert_eq!(
            plan(&Choice {
                want_vram: true,
                vram_slot: true,
                ..Choice::default()
            }),
            Plan {
                pack: false,
                d2h: true
            },
            "no active broker: never a pack"
        );
    }

    /// ★ Provisioning asks once per class and slot: `auto` at the first want (not at realize),
    /// `on` at realize, `off` never; one growth to the max for every slot; nothing after a
    /// refusal; nothing for a frame no size holds.
    #[test]
    fn slots_are_asked_once_per_class_and_never_after_a_refusal() {
        let (c0, max) = (10 << 20, 36 << 20);
        let mut p = Provisioning::new(VramMode::Auto, 5, c0, max);
        assert_eq!(p.first(true, true), None, "auto never at realize");
        assert_eq!(p.first(false, false), None, "auto waits for a yes");
        assert_eq!(p.grow(1), None, "no growth before the first");
        let r = p.first(false, true).unwrap();
        assert_eq!(
            r,
            Request {
                slots: vec![0, 1, 2, 3, 4],
                bytes: c0
            }
        );
        assert_eq!(p.first(false, true), None, "once");
        assert_eq!(p.grow(c0), None, "class 0 holds it");
        assert_eq!(p.grow(max + 1), None, "no size holds it");
        let g = p.grow(c0 + 1).unwrap();
        assert_eq!((g.slots.len(), g.bytes), (5, max));
        assert_eq!(p.grow(max), None, "one growth");
        let mut on = Provisioning::new(VramMode::On, 5, c0, max);
        assert_eq!(on.first(false, true), None, "on: at realize only");
        assert!(on.first(true, false).is_some());
        let mut off = Provisioning::new(VramMode::Off, 5, c0, max);
        assert_eq!(off.first(true, true), None);
        assert_eq!(off.first(false, true), None);
        let mut r = Provisioning::new(VramMode::Auto, 5, c0, max);
        r.refuse();
        assert!(r.refused());
        assert_eq!(r.first(false, true), None);
    }
}
