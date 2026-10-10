//! ⚠ DIAGNOSTIC (2026-10-10, TDR hunt) — a lock-free ring of the guest-interrupt events this device decided, with the
//! wall clock (UTC ms), who decided it and what came of it. Only filled when `KF3_COMPLETION_PROBE` is set (the probe
//! thread dumps it beside its other lines); one `OnceLock` load when off. Records nothing the guest can influence
//! beyond a count; never read by a decision.
//!
//! Entry (u64): `utc_ms` (low 40 bits) | `src` << 40 | `vector` << 48 | `res` << 56.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const N: usize = 8192;
static RING: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static IDX: AtomicUsize = AtomicUsize::new(0);

/// Source: the worker's `latch_and_deliver` (a host-side completion latched on a CPU_INTR vector).
pub const SRC_LATCH: u8 = 1;
/// Source: a guest register write whose effect re-sends a pending message (LEAF_EN_SET / TOP_EN_SET).
pub const SRC_GUEST_WRITE: u8 = 3;
/// Source base: a host non-stall edge judged for engine `i` is `SRC_ENGINE + i`.
pub const SRC_ENGINE: u8 = 16;
/// Source: a host `FIFO_EVENT_MTHD` edge.
pub const SRC_FIFO: u8 = 40;

/// Result: a message was raised (eventfd written).
pub const RES_MESSAGE: u8 = 0;
/// Result: latched but not asserted (leaf/top disabled by the guest): nothing raised.
pub const RES_HELD: u8 = 1;
/// Result: `NotArmed` (the guest had not armed the event).
pub const RES_NOT_ARMED: u8 = 2;
/// Result: owed (paced).
pub const RES_OWED: u8 = 3;
/// Result: `Unvectored` / refused / relay off.
pub const RES_OTHER: u8 = 4;

fn utc_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
        & 0xFF_FFFF_FFFF
}

/// Record one event (no-op unless the probe is on).
pub fn note(src: u8, vector: u32, res: u8) {
    if crate::chan::completion_probe_ms().is_none() {
        return;
    }
    let i = IDX.fetch_add(1, Ordering::Relaxed) % N;
    let e = utc_ms() | (u64::from(src) << 40) | ((u64::from(vector) & 0xFF) << 48) | (u64::from(res) << 56);
    RING[i].store(e, Ordering::Relaxed);
}

/// The events of the last `window_ms` (oldest first), as `(utc_ms, src, vector, res)`.
#[must_use]
pub fn recent(window_ms: u64) -> Vec<(u64, u8, u8, u8)> {
    let now = utc_ms();
    let end = IDX.load(Ordering::Relaxed);
    let mut v = Vec::new();
    for k in 0..N.min(end) {
        let e = RING[(end - 1 - k) % N].load(Ordering::Relaxed);
        let t = e & 0xFF_FFFF_FFFF;
        if t == 0 || now.saturating_sub(t) > window_ms {
            break;
        }
        v.push((t, (e >> 40) as u8, (e >> 48) as u8, (e >> 56) as u8));
    }
    v.reverse();
    v
}

/// A summary of the last `window_s` seconds: per second, per (src,res) counts, then the last 24 events.
#[must_use]
pub fn dump(window_s: u64) -> Vec<String> {
    let ev = recent(window_s * 1000);
    let now = utc_ms();
    let mut out = vec![format!(
        "kf3: IRQ-RING utc_ms_now={now} events_in_last_{window_s}s={}",
        ev.len()
    )];
    let mut by: std::collections::BTreeMap<(u64, u8, u8, u8), u32> = std::collections::BTreeMap::new();
    for (t, s, v, r) in &ev {
        *by.entry((now.saturating_sub(*t) / 1000, *s, *v, *r)).or_insert(0) += 1;
    }
    for ((ago, s, v, r), n) in by.iter().rev() {
        out.push(format!(
            "kf3: IRQ-RING   {ago}s ago: src={s} vec={v} res={r} x{n}"
        ));
    }
    for (t, s, v, r) in ev.iter().rev().take(24).rev() {
        out.push(format!(
            "kf3: IRQ-RING   last: utc_ms={t} (-{}ms) src={s} vec={v} res={r}",
            now.saturating_sub(*t)
        ));
    }
    out
}
