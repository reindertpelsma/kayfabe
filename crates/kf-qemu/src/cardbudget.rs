//! ★ The per-HOST-CARD budget, summed across every kf3 device in this process that sits on the same
//! host GPU — checked at realize, refused by name up front (V3_MULTI_GPU_AUDIT §2 "Shared host
//! BAR1"; `THE_CONSTRAINTS.md` §w727 *"refuse at startup, loudly, never silently clamp"*).
//!
//! The relation (`V3_P4_PORT_MAP.md` Q5, summed over devices):
//!
//! ```text
//!   Σ_devices_on_card (guest_bar1 + guest_bar2 + PRAMIN window + OUR_HEADROOM)  ≤  host_bar1
//! ```
//!
//! ⚠ Scope, stated rather than implied: this sees the kf3 devices of **this QEMU process** only. Two
//! VMs on one host card share the same host BAR1 pool and neither can see the other's demand — that
//! half stays design work (`THE_CONSTRAINTS.md:2166`, *"whether a second GPU, a second VM … share the
//! pool"*). ⊘ `OUR_HEADROOM` is the one measured member (w726: 16 MiB, the walker context + our
//! channels) per device, because each device brings its own walker context and channels.
//!
//! The store is budgeted by RM itself: a reservation the card cannot hold is refused at realize
//! (`store of N MiB refused: NoMemory`, measured in the audit). This module adds what RM cannot say —
//! how much of the card the OTHER devices of this process already hold — to that refusal.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// Per device, beyond its advertised apertures: the walker's CUDA context + our channels (w726).
pub const OUR_HEADROOM: u64 = 16 << 20;
/// The PRAMIN window's views (1 MiB, `kf_trap::pramin`).
pub const PRAMIN_BYTES: u64 = kf_trap::pramin::GRANULE * kf_trap::pramin::SLOTS as u64;

/// What one device asks of its host card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Demand {
    /// Host BAR1 bytes (apertures + PRAMIN + headroom).
    pub bar1: u64,
    /// Store (framebuffer) bytes.
    pub store: u64,
}

impl Demand {
    /// A device advertising `guest_bar1` / `guest_bar2` over a `store`-byte framebuffer.
    #[must_use]
    pub fn of(guest_bar1: u64, guest_bar2: u64, store: u64) -> Demand {
        Demand {
            bar1: guest_bar1.saturating_add(guest_bar2).saturating_add(PRAMIN_BYTES).saturating_add(OUR_HEADROOM),
            store,
        }
    }
}

/// The pure rule: may `want` join `held` (the sum of this card's devices so far) on a card whose
/// BAR1 is `host_bar1`?
///
/// # Errors
/// The refusal, naming every term.
pub fn check(bdf: &str, host_bar1: u64, held: Demand, n_held: usize, want: Demand) -> Result<(), String> {
    let total = held.bar1.saturating_add(want.bar1);
    if total > host_bar1 {
        return Err(format!(
            "host card {bdf}: BAR1 budget exceeded — this device needs {} MiB (guest BAR1 + BAR2 + \
             {} MiB PRAMIN + {} MiB headroom) and the {n_held} kf3 device(s) already on this card \
             hold {} MiB; {} MiB > the host's {} MiB BAR1. Refused at realize, by name: shrink the \
             guest BAR1 (bar1-size) or put this device on another host GPU",
            want.bar1 >> 20,
            PRAMIN_BYTES >> 20,
            OUR_HEADROOM >> 20,
            held.bar1 >> 20,
            total >> 20,
            host_bar1 >> 20
        ));
    }
    Ok(())
}

static CARDS: Mutex<BTreeMap<String, (Demand, usize)>> = Mutex::new(BTreeMap::new());

/// ★ Admit one device's demand on host card `bdf`, or refuse it by name. Taken at realize only
/// (never on a vCPU, never under another lock). A device is realized once and never unplugged
/// (`Box::leak` at realize), so admission is never returned.
///
/// # Errors
/// [`check`]'s refusal.
pub fn admit(bdf: &str, host_bar1: u64, want: Demand) -> Result<(), String> {
    let mut m = CARDS.lock().map_err(|_| "card budget registry poisoned".to_string())?;
    let (held, n) = m.get(bdf).copied().unwrap_or_default();
    check(bdf, host_bar1, held, n, want)?;
    m.insert(
        bdf.to_string(),
        (Demand { bar1: held.bar1 + want.bar1, store: held.store + want.store }, n + 1),
    );
    Ok(())
}

/// Store bytes (and device count) this process's OTHER devices already hold on `bdf` — carried
/// into a store refusal so it names the neighbours, not just RM's `NoMemory`.
#[must_use]
pub fn store_held(bdf: &str) -> (u64, usize) {
    CARDS
        .lock()
        .ok()
        .and_then(|m| m.get(bdf).map(|(d, n)| (d.store, *n)))
        .unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1 << 20;

    #[test]
    fn one_default_device_fits_a_256_mib_card() {
        let d = Demand::of(128 * MIB, 32 * MIB, 8192 * MIB);
        assert!(check("0000:01:00.0", 256 * MIB, Demand::default(), 0, d).is_ok());
    }

    #[test]
    fn two_default_devices_on_one_256_mib_card_are_refused_by_name() {
        let d = Demand::of(128 * MIB, 32 * MIB, 4096 * MIB);
        let e = check("0000:01:00.0", 256 * MIB, d, 1, d).expect_err("2 × 177 MiB > 256 MiB");
        assert!(e.contains("0000:01:00.0") && e.contains("BAR1 budget exceeded"), "{e}");
    }

    #[test]
    fn two_small_bar1_devices_share_one_card() {
        let d = Demand::of(64 * MIB, 32 * MIB, 4096 * MIB);
        assert!(check("0000:01:00.0", 256 * MIB, d, 1, d).is_ok(), "2 × 113 MiB ≤ 256 MiB");
    }

    #[test]
    fn distinct_cards_do_not_share_a_budget() {
        let d = Demand::of(128 * MIB, 32 * MIB, 8192 * MIB);
        admit("test:aa:00.0", 256 * MIB, d).expect("first card");
        admit("test:bb:00.0", 256 * MIB, d).expect("second card is its own budget");
        assert!(admit("test:aa:00.0", 256 * MIB, d).is_err(), "the first card is now full");
        assert_eq!(store_held("test:aa:00.0"), (8192 * MIB, 1), "a refusal is not admitted");
    }
}
