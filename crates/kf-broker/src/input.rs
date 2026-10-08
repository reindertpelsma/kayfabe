// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The broker's input policy behind a VMM-neutral trait** (`OWNER_RULINGS.md` §V, 2026-10-08:
//! "full vmm neutral input traits for a broker to hook on"; `docs/design/V3_DISPLAY.md` §8.20).
//!
//! The relay ([`crate::Relay`]) turns broker packets into bounded [`Input`]: evdev ranges, the
//! ABS clamp into the broker's range, REL summing (saturating), the grab mirrored from every
//! packet, the routing of buttons and the wheel to the RELATIVE pointer while grabbed, no ABS
//! while grabbed, and the re-sync of the absolute pointer when the grab ends. This module is the
//! rest of the policy, which until 2026-10-08 lived in QEMU's device (`kf3.c`):
//!
//! - **which keys exist**: `KEY_RESERVED`, codes above `KEY_MAX` and the evdev BUTTON blocks are
//!   refused here; the VMM still answers whether it can map the rest ([`InputSink::key`]);
//! - **which buttons are forwarded**: left, right, middle, side and extra ([`Button`]); any other
//!   button code is dropped and counted;
//! - **the pointing device on a grab and on its end**: a relative one while grabbed, the absolute
//!   one otherwise, a paravirtual one preferred, else the first the VMM lists;
//! - **the missing-device warning**: once per kind, the absolute one checked at the first
//!   connection;
//! - **where the sync points fall**: one per button, wheel detent, absolute position and summed
//!   relative motion — the per-packet mapping kf3.c had (`V3_DISPLAY.md` §8.20 tabulates it);
//! - **close**: force off, or an ACPI-style powerdown the guest decides on.
//!
//! The VMM implements [`InputSink`] — eleven small verbs that name no VMM type — and nothing
//! else. For QEMU that is `qemu/hw/misc/kf3/kf3.c` (through `kf-qemu`'s `raw_unsafe.rs`); another
//! VMM maps the same verbs to its own input devices (`V3_DISPLAY.md` §8.20 says what
//! cloud-hypervisor, crosvm or Firecracker would write). Every [`Input`] costs at most
//! [`MAX_SINK_CALLS_PER_INPUT`] sink calls, and the relay emits at most two inputs per packet, so
//! a packet costs at most four — whatever the broker sends.

use crate::conn::{Input, Pointer, log_line};

/// The largest Linux input code (`KEY_MAX`).
pub const KEY_MAX: u16 = 0x2ff;

/// ★ The most sink calls one [`Input`] costs (a grab: the device list, then the selection or the
/// missing-device notice; a pointer event: the event, then the sync).
pub const MAX_SINK_CALLS_PER_INPUT: usize = 2;

/// The most pointing devices the policy looks at (the rest of a longer list is ignored).
pub const MAX_POINTER_DEVICES: usize = 16;

/// The longest device name kept for the log, in bytes.
pub const POINTER_NAME_MAX: usize = 55;

/// ★ A pointer button the policy forwards — the five a mouse or tablet has (nvkvm-pv
/// `relay.c:1101-1111`, the set kf3.c forwarded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// `BTN_LEFT`.
    Left,
    /// `BTN_RIGHT`.
    Right,
    /// `BTN_MIDDLE`.
    Middle,
    /// `BTN_SIDE` (back).
    Side,
    /// `BTN_EXTRA` (forward).
    Extra,
}

impl Button {
    /// The forwarded button for a Linux evdev button code, or `None` (dropped).
    #[must_use]
    pub fn from_evdev(code: u16) -> Option<Button> {
        match code {
            0x110 => Some(Button::Left),
            0x111 => Some(Button::Right),
            0x112 => Some(Button::Middle),
            0x113 => Some(Button::Side),
            0x114 => Some(Button::Extra),
            _ => None,
        }
    }

    /// The Linux evdev code.
    #[must_use]
    pub fn evdev(self) -> u16 {
        match self {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
            Button::Side => 0x113,
            Button::Extra => 0x114,
        }
    }
}

/// Whether `code` is an evdev KEY a keyboard reports — not `KEY_RESERVED`, not above `KEY_MAX`,
/// and not in a BUTTON block (`BTN_MISC`..`BTN_GEAR_UP`, `BTN_DPAD_*`, `BTN_TRIGGER_HAPPY*`:
/// buttons arrive as `EV_BTN`). The VMM may still have no mapping for a code that passes.
#[must_use]
pub fn is_evdev_key(code: u16) -> bool {
    code != 0
        && code <= KEY_MAX
        && !(0x100..=0x151).contains(&code)
        && !(0x220..=0x223).contains(&code)
        && !(0x2c0..=0x2e7).contains(&code)
}

/// ★ The range an absolute position is in: `0..width` x `0..height`, each in `1..=2^20` (the
/// relay's bound).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbsRange {
    /// Columns.
    pub width: u32,
    /// Rows.
    pub height: u32,
}

impl AbsRange {
    /// `(x, y)` scaled onto a device axis `0..=axis_max` the way QEMU's `qemu_input_scale_axis`
    /// does (`value * axis_max / range`, the range not less one — measured on the guest as
    /// `x * 32767 / 1920`, `V3_DISPLAY.md` §8.17). For a sink whose device has its own axis.
    #[must_use]
    pub fn scale(self, x: u32, y: u32, axis_max: u32) -> (u32, u32) {
        let s = |v: u32, r: u32| {
            let r = u64::from(r.max(1));
            let v = u64::from(v).min(r - 1);
            u32::try_from(v * u64::from(axis_max) / r).unwrap_or(axis_max)
        };
        (s(x, self.width), s(y, self.height))
    }
}

/// ★ One pointing device the VMM offers the guest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PointerDevice {
    /// The VMM's handle for [`InputSink::select_pointer`].
    pub id: u32,
    /// Absolute (a tablet) or relative (a mouse).
    pub kind: Pointer,
    /// A paravirtual device (virtio-input): preferred when there is a choice.
    pub paravirtual: bool,
    /// For the log only (at most [`POINTER_NAME_MAX`] printable bytes are kept).
    pub name: String,
}

/// What the user asked for by closing the broker's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerRequest {
    /// Stop the VM now.
    ForceOff,
    /// An ACPI-style powerdown request the guest decides on.
    Powerdown,
}

/// ★★ **What a VMM implements so the broker's input reaches its guest** — all policy is on this
/// side of the trait; each verb is one thing the VMM's input layer does.
///
/// Calls come from one thread (the VMM's event loop, inside the relay's entries); none may block.
/// Pointer verbs (`button`, `wheel`, `abs`, `rel`) are QUEUED until [`InputSink::sync`]; `key` is a
/// complete report by itself (the VMM syncs it as it needs).
pub trait InputSink {
    /// A key edge, by Linux evdev code ([`is_evdev_key`] holds). Returns whether the VMM could
    /// map it (a `false` is counted, never retried).
    fn key(&mut self, code: u16, down: bool) -> bool;
    /// A button edge on the `to` pointing device.
    fn button(&mut self, button: Button, down: bool, to: Pointer);
    /// One wheel detent on the `to` device: `dy` +1 up (away from the user) / -1 down, `dx` +1
    /// right / -1 left; one of them is non-zero. A VMM without a horizontal wheel ignores `dx`.
    fn wheel(&mut self, dx: i32, dy: i32, to: Pointer);
    /// The absolute pointer at `(x, y)` inside `range` (`x < width`, `y < height`).
    fn abs(&mut self, x: u32, y: u32, range: AbsRange);
    /// Relative motion on the relative pointer.
    fn rel(&mut self, dx: i32, dy: i32);
    /// End of one pointer report (evdev's `SYN_REPORT`).
    fn sync(&mut self);
    /// The pointing devices the guest has, written to `out`; returns how many (at most
    /// `out.len()`).
    fn pointer_devices(&mut self, out: &mut [PointerDevice]) -> usize;
    /// Make `id` (of `kind`) the device pointer events go to.
    fn select_pointer(&mut self, id: u32, kind: Pointer);
    /// The guest has no `kind` device; the policy has logged what is lost. The VMM may say how
    /// to add one (called at most once per kind per VM).
    fn missing_pointer(&mut self, kind: Pointer);
    /// The user closed the broker's window.
    fn close(&mut self, request: PowerRequest);
    /// The broker's window is `width` x `height` (64..=8192) at `refresh_mhz` (0: unknown).
    fn resize_hint(&mut self, width: u32, height: u32, refresh_mhz: u32);
}

/// Counters, for the status line and the tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputCounters {
    /// Key edges delivered and mapped.
    pub keys: u64,
    /// Key codes refused here (not an evdev key).
    pub keys_refused: u64,
    /// Key codes the VMM could not map.
    pub keys_unmapped: u64,
    /// Button edges delivered.
    pub buttons: u64,
    /// Button codes not forwarded.
    pub buttons_dropped: u64,
    /// Wheel detents delivered.
    pub wheel: u64,
    /// Absolute positions delivered.
    pub abs: u64,
    /// Relative reports delivered.
    pub rel: u64,
    /// Pointing-device switches made.
    pub switches: u64,
    /// Sink calls made.
    pub sink_calls: u64,
}

/// ★ The VM-lifetime input policy (it outlives broker connections). Main loop only.
#[derive(Debug, Default)]
pub struct InputPolicy {
    checked: bool,
    warned: [bool; 2],
    logged_refusal: [bool; 3],
    counters: InputCounters,
}

fn kind_name(k: Pointer) -> &'static str {
    match k {
        Pointer::Absolute => "absolute",
        Pointer::Relative => "relative",
    }
}

fn kind_index(k: Pointer) -> usize {
    match k {
        Pointer::Absolute => 0,
        Pointer::Relative => 1,
    }
}

/// A sink wrapper that counts calls (the bound is a test's claim; the count is the status's).
struct Counted<'a> {
    sink: &'a mut dyn InputSink,
    calls: u64,
}

impl InputPolicy {
    /// A fresh policy.
    #[must_use]
    pub fn new() -> InputPolicy {
        InputPolicy::default()
    }

    /// The counters.
    #[must_use]
    pub fn counters(&self) -> InputCounters {
        self.counters
    }

    /// One status fragment.
    #[must_use]
    pub fn status(&self) -> String {
        let c = &self.counters;
        format!(
            "input[keys={} keys_refused={} keys_unmapped={} buttons={} buttons_dropped={} wheel={} abs={} rel={} switches={}]",
            c.keys,
            c.keys_refused,
            c.keys_unmapped,
            c.buttons,
            c.buttons_dropped,
            c.wheel,
            c.abs,
            c.rel,
            c.switches
        )
    }

    /// ★ A broker connection passed the peer check: at the FIRST one per VM, check that an
    /// absolute pointing device exists (the broker's pointer position arrives as absolute
    /// reports, which go nowhere without one). On the VMM's loop, so every device of the VM's
    /// configuration exists by then. At most [`MAX_SINK_CALLS_PER_INPUT`] sink calls.
    pub fn connected(&mut self, sink: &mut dyn InputSink) {
        if self.checked {
            return;
        }
        self.checked = true;
        let mut s = Counted { sink, calls: 0 };
        if self.pick(&mut s, Pointer::Absolute).is_none() {
            self.missing(
                &mut s,
                Pointer::Absolute,
                "the broker's pointer position goes nowhere (keys and buttons still work)",
            );
        }
        self.counters.sink_calls += s.calls;
    }

    /// ★ Deliver bounded inputs from the relay. At most [`MAX_SINK_CALLS_PER_INPUT`] sink calls
    /// per input.
    pub fn deliver(&mut self, inputs: &[Input], sink: &mut dyn InputSink) {
        let mut s = Counted { sink, calls: 0 };
        for &i in inputs {
            self.one(&mut s, i);
        }
        self.counters.sink_calls += s.calls;
    }

    fn one(&mut self, s: &mut Counted<'_>, i: Input) {
        match i {
            Input::Key { code, down } => {
                if !is_evdev_key(code) {
                    self.counters.keys_refused += 1;
                    self.refusal(0, || {
                        format!("input: {code:#x} is not an evdev key code — dropped (counted)")
                    });
                } else if s.key(code, down) {
                    self.counters.keys += 1;
                } else {
                    self.counters.keys_unmapped += 1;
                    self.refusal(1, || {
                        format!("input: the VMM has no mapping for evdev key {code:#x} — dropped (counted)")
                    });
                }
            }
            Input::Btn { code, down, to } => match Button::from_evdev(code) {
                Some(b) => {
                    s.button(b, down, to);
                    s.sync();
                    self.counters.buttons += 1;
                }
                None => {
                    self.counters.buttons_dropped += 1;
                    self.refusal(2, || {
                        format!("input: button {code:#x} is not forwarded (left, right, middle, side and extra are) — dropped (counted)")
                    });
                }
            },
            Input::Wheel { dx, dy, to } => {
                if dx != 0 || dy != 0 {
                    s.wheel(dx.signum(), dy.signum(), to);
                    s.sync();
                    self.counters.wheel += 1;
                }
            }
            Input::Abs { x, y, w, h } => {
                let (Ok(width), Ok(height)) = (u32::try_from(w), u32::try_from(h)) else {
                    return;
                };
                if width == 0 || height == 0 {
                    return;
                }
                let range = AbsRange { width, height };
                let x = u32::try_from(x).unwrap_or(0).min(width - 1);
                let y = u32::try_from(y).unwrap_or(0).min(height - 1);
                s.abs(x, y, range);
                s.sync();
                self.counters.abs += 1;
            }
            Input::Rel { dx, dy } => {
                s.rel(dx, dy);
                s.sync();
                self.counters.rel += 1;
            }
            Input::Grab(on) => {
                let kind = if on {
                    Pointer::Relative
                } else {
                    Pointer::Absolute
                };
                match self.pick(s, kind) {
                    Some(d) => {
                        s.select_pointer(d.id, kind);
                        self.counters.switches += 1;
                        log_line(format!(
                            "pointing device -> #{} {} ({})",
                            d.id,
                            d.name,
                            kind_name(kind)
                        ));
                    }
                    None => self.missing(
                        s,
                        kind,
                        if on {
                            "pointer motion goes nowhere while grabbed"
                        } else {
                            "the pointer cannot be put back on ungrab"
                        },
                    ),
                }
            }
            Input::Close { force } => s.close(if force {
                PowerRequest::ForceOff
            } else {
                PowerRequest::Powerdown
            }),
            Input::Surface { w, h, mhz } => {
                let w = u32::try_from(w.clamp(64, 8192)).unwrap_or(64);
                let h = u32::try_from(h.clamp(64, 8192)).unwrap_or(64);
                s.resize_hint(w, h, mhz);
            }
        }
    }

    /// The device of `kind` to use: the first paravirtual one, else the first one (one sink call).
    fn pick(&mut self, s: &mut Counted<'_>, kind: Pointer) -> Option<PointerDevice> {
        let mut devs: [PointerDevice; MAX_POINTER_DEVICES] = Default::default();
        let n = s.pointer_devices(&mut devs).min(MAX_POINTER_DEVICES);
        let of_kind = || devs[..n].iter().filter(|d| d.kind == kind);
        let pick = of_kind()
            .find(|d| d.paravirtual)
            .or_else(|| of_kind().next())
            .cloned();
        pick.map(|mut d| {
            d.name = printable(&d.name);
            d
        })
    }

    fn missing(&mut self, s: &mut Counted<'_>, kind: Pointer, lost: &str) {
        let k = kind_index(kind);
        if self.warned[k] {
            return;
        }
        self.warned[k] = true;
        log_line(format!(
            "NO {} pointing device exists, so {lost}. Give the VM one (a virtio-input {}).",
            kind_name(kind),
            if kind == Pointer::Absolute {
                "tablet bound to this display"
            } else {
                "mouse"
            }
        ));
        s.missing_pointer(kind);
    }

    fn refusal(&mut self, which: usize, line: impl FnOnce() -> String) {
        if !self.logged_refusal[which] {
            self.logged_refusal[which] = true;
            log_line(line());
        }
    }
}

/// `name` cut to [`POINTER_NAME_MAX`] bytes of printable ASCII (anything else becomes `?`).
fn printable(name: &str) -> String {
    name.bytes()
        .take(POINTER_NAME_MAX)
        .map(|b| {
            if b.is_ascii_graphic() || b == b' ' {
                char::from(b)
            } else {
                '?'
            }
        })
        .collect()
}

impl Counted<'_> {
    fn key(&mut self, code: u16, down: bool) -> bool {
        self.calls += 1;
        self.sink.key(code, down)
    }
    fn button(&mut self, b: Button, down: bool, to: Pointer) {
        self.calls += 1;
        self.sink.button(b, down, to);
    }
    fn wheel(&mut self, dx: i32, dy: i32, to: Pointer) {
        self.calls += 1;
        self.sink.wheel(dx, dy, to);
    }
    fn abs(&mut self, x: u32, y: u32, r: AbsRange) {
        self.calls += 1;
        self.sink.abs(x, y, r);
    }
    fn rel(&mut self, dx: i32, dy: i32) {
        self.calls += 1;
        self.sink.rel(dx, dy);
    }
    fn sync(&mut self) {
        self.calls += 1;
        self.sink.sync();
    }
    fn pointer_devices(&mut self, out: &mut [PointerDevice]) -> usize {
        self.calls += 1;
        self.sink.pointer_devices(out)
    }
    fn select_pointer(&mut self, id: u32, kind: Pointer) {
        self.calls += 1;
        self.sink.select_pointer(id, kind);
    }
    fn missing_pointer(&mut self, kind: Pointer) {
        self.calls += 1;
        self.sink.missing_pointer(kind);
    }
    fn close(&mut self, r: PowerRequest) {
        self.calls += 1;
        self.sink.close(r);
    }
    fn resize_hint(&mut self, w: u32, h: u32, mhz: u32) {
        self.calls += 1;
        self.sink.resize_hint(w, h, mhz);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_is_qemus_value_times_axis_over_range() {
        let r = AbsRange {
            width: 1920,
            height: 1080,
        };
        assert_eq!(r.scale(100, 100, 32767), (1706, 3033));
        assert_eq!(r.scale(0, 0, 32767), (0, 0));
        // clamped into the range, never past the axis
        assert_eq!(r.scale(u32::MAX, u32::MAX, 32767), (32749, 32736));
    }

    #[test]
    fn buttons_and_keys_are_disjoint() {
        for c in 0..=KEY_MAX {
            assert!(
                !(is_evdev_key(c) && Button::from_evdev(c).is_some()),
                "{c:#x}"
            );
        }
        assert!(is_evdev_key(30) && is_evdev_key(0x2a) && is_evdev_key(KEY_MAX));
        assert!(!is_evdev_key(0) && !is_evdev_key(0x110) && !is_evdev_key(0x300));
        for b in [
            Button::Left,
            Button::Right,
            Button::Middle,
            Button::Side,
            Button::Extra,
        ] {
            assert_eq!(Button::from_evdev(b.evdev()), Some(b));
        }
    }

    #[test]
    fn a_device_name_is_cut_and_made_printable() {
        let n = printable(&format!("QEMU\u{1b}[31m{}", "x".repeat(100)));
        assert!(
            n.len() == POINTER_NAME_MAX && n.starts_with("QEMU?[31m"),
            "{n}"
        );
    }
}
