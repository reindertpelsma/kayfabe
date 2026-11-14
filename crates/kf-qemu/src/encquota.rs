//! ★ Security review 1 F-05 (= S1-42), 2026-11-14: the per-VM cap on host NVENC session slots.
//!
//! A guest NVENC session acquire (`kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE`) takes one of the
//! HOST's GPU-wide session slots, on OUR host client. Without a cap a guest process looping the
//! acquire takes every slot, and other VMs and host applications can no longer encode
//! (cross-tenant denial of service from guest userspace). This book is the cap, checked BEFORE the
//! host call: a reservation is taken first, the host call runs, and the reservation is either kept
//! (the host said yes) or cancelled (the host said no). A guest client's sessions are released
//! with the guest's release or with the client's free.
//!
//! **The number.** [`ENC_SESSIONS_PER_VM`] = 3. Design choice, not measured on a box:
//! - GeForce drivers enforce a driver-wide limit on concurrent NVENC sessions; NVIDIA's public
//!   "Video Encode and Decode GPU Support Matrix" lists 3 for years, then 5, then 8 on current
//!   drivers (INFERRED from the public matrix; the host limit was never probed by
//!   `kf-encoder-session-probe`, which cycles one session). Workstation and data-centre parts have
//!   no such limit.
//! - 3 is the limit GeForce guest software lived with for years (OBS record + stream needs 2), so
//!   a correct guest is not expected to need more. Two VMs at the cap still leave at least 2 of 8
//!   slots to the host and other tenants.
//! - On a part with no driver limit the cap is stricter than the hardware; raising it there is an
//!   owner decision (the host's own limit is not exposed through an unprivileged control we use).
//!
//! The refusal is `NV_ERR_INSUFFICIENT_RESOURCES`, the status RM gives when a resource is
//! exhausted. Refusals are counted; the caller logs only the ones [`crate::reset::loud`] picks.

use std::collections::HashMap;

/// The most host NVENC session slots one VM may hold at once (see the module text for why 3).
pub(crate) const ENC_SESSIONS_PER_VM: u32 = 3;

/// The per-VM NVENC session book: slots held per guest client, and their sum.
#[derive(Debug)]
pub(crate) struct EncSessions {
    held: HashMap<u32, u32>,
    total: u32,
    cap: u32,
    refused: u64,
}

/// One reserved slot for `client`, taken before the host call. Not `Copy`/`Clone`: it is spent
/// exactly once, by [`EncSessions::cancel`] (the host refused) or [`EncSessions::keep`].
#[derive(Debug)]
#[must_use = "a reservation is kept or cancelled"]
pub(crate) struct Reservation {
    client: u32,
}

/// A refused acquire: the VM is at its cap. `refused` counts every refusal so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) held: u32,
    pub(crate) cap: u32,
    pub(crate) refused: u64,
}

impl Default for EncSessions {
    fn default() -> Self {
        Self::with_cap(ENC_SESSIONS_PER_VM)
    }
}

impl EncSessions {
    /// A book with cap `cap` (tests use small caps; production uses [`ENC_SESSIONS_PER_VM`]).
    pub(crate) fn with_cap(cap: u32) -> Self {
        Self {
            held: HashMap::new(),
            total: 0,
            cap,
            refused: 0,
        }
    }

    /// Reserve one slot for `client`, or refuse when the VM already holds `cap` (reserved slots
    /// count: they are host calls in flight).
    pub(crate) fn reserve(&mut self, client: u32) -> Result<Reservation, Refusal> {
        if self.total >= self.cap {
            self.refused = self.refused.saturating_add(1);
            return Err(Refusal {
                held: self.total,
                cap: self.cap,
                refused: self.refused,
            });
        }
        self.total = self.total.saturating_add(1);
        let c = self.held.entry(client).or_insert(0);
        *c = c.saturating_add(1);
        Ok(Reservation { client })
    }

    /// The host granted the slot: the reservation becomes a held slot (it is already counted).
    pub(crate) fn keep(&self, r: Reservation) {
        let Reservation { client: _ } = r;
    }

    /// The host refused the slot: give the reservation back.
    pub(crate) fn cancel(&mut self, r: Reservation) {
        self.drop_one(r.client);
    }

    /// The host released one of `client`'s slots. Returns the client's remaining count.
    pub(crate) fn released(&mut self, client: u32) -> u32 {
        self.drop_one(client);
        self.of(client)
    }

    /// Every slot `client` held, removed from the book (its free: the caller releases them).
    pub(crate) fn take_client(&mut self, client: u32) -> u32 {
        let n = self.held.remove(&client).unwrap_or(0);
        self.total = self.total.saturating_sub(n);
        n
    }

    /// Slots `client` holds.
    pub(crate) fn of(&self, client: u32) -> u32 {
        self.held.get(&client).copied().unwrap_or(0)
    }

    /// Slots the VM holds (all clients, reservations included).
    #[cfg(test)]
    pub(crate) fn total(&self) -> u32 {
        self.total
    }

    /// Guest clients holding at least one slot.
    pub(crate) fn clients(&self) -> impl Iterator<Item = u32> + '_ {
        self.held.keys().copied()
    }

    /// One of `client`'s slots, gone. An entry never holds 0 (it is removed with its last slot),
    /// so a client without an entry holds nothing and nothing changes.
    fn drop_one(&mut self, client: u32) {
        if let Some(c) = self.held.get_mut(&client) {
            *c = c.saturating_sub(1);
            self.total = self.total.saturating_sub(1);
            if *c == 0 {
                self.held.remove(&client);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hostile: one guest process loops the acquire. Past the cap every acquire is refused, and
    /// counted; the VM never holds more than the cap.
    #[test]
    fn a_looping_client_stops_at_the_cap() {
        let mut b = EncSessions::default();
        let mut kept = 0;
        let mut refused = 0;
        for _ in 0..10_000 {
            match b.reserve(7) {
                Ok(r) => {
                    b.keep(r);
                    kept += 1;
                }
                Err(e) => {
                    refused += 1;
                    assert_eq!(e.cap, ENC_SESSIONS_PER_VM);
                    assert_eq!(e.held, ENC_SESSIONS_PER_VM);
                    assert_eq!(e.refused, refused);
                }
            }
        }
        assert_eq!(kept, ENC_SESSIONS_PER_VM);
        assert_eq!(b.total(), ENC_SESSIONS_PER_VM);
        assert_eq!(b.of(7), ENC_SESSIONS_PER_VM);
        assert_eq!(refused, 10_000 - u64::from(ENC_SESSIONS_PER_VM));
    }

    /// Hostile: many guest clients (processes) each take one. The cap is per VM, not per client.
    #[test]
    fn the_cap_is_per_vm_not_per_client() {
        let mut b = EncSessions::with_cap(3);
        for client in 1..=3 {
            let r = b.reserve(client).expect("under the cap");
            b.keep(r);
        }
        let mut clients: Vec<u32> = b.clients().collect();
        clients.sort_unstable();
        assert_eq!(clients, [1, 2, 3], "the reset's census sees every holder");
        let e = b
            .reserve(4)
            .expect_err("a fourth client is past the VM's cap");
        assert_eq!((e.held, e.cap, e.refused), (3, 3, 1));
        assert_eq!(b.of(4), 0, "a refused client holds nothing");
        assert_eq!(b.total(), 3);
    }

    /// Reservations in flight count against the cap (the check is before the host call).
    #[test]
    fn reservations_in_flight_count() {
        let mut b = EncSessions::with_cap(2);
        let r1 = b.reserve(1).expect("first");
        let r2 = b.reserve(1).expect("second");
        assert!(
            b.reserve(2).is_err(),
            "two host calls in flight fill a cap of 2"
        );
        b.cancel(r1);
        assert_eq!(b.total(), 1, "a host refusal gives the slot back");
        let r3 = b.reserve(2).expect("a cancelled slot is free again");
        b.keep(r2);
        b.keep(r3);
        assert_eq!((b.of(1), b.of(2)), (1, 1));
        assert_eq!(b.total(), 2);
    }

    /// A release frees a slot for anyone; the client entry goes with its last slot.
    #[test]
    fn a_release_frees_a_slot() {
        let mut b = EncSessions::with_cap(1);
        let r = b.reserve(5).expect("first");
        b.keep(r);
        assert!(b.reserve(6).is_err());
        assert_eq!(b.released(5), 0);
        assert_eq!(b.clients().count(), 0, "no empty entries are kept");
        let r = b.reserve(6).expect("the released slot is free");
        b.keep(r);
        assert_eq!(b.of(6), 1);
    }

    /// Hostile: a release with nothing held (or for another client) never underflows the sum or
    /// frees someone else's slot.
    #[test]
    fn a_release_with_none_held_changes_nothing() {
        let mut b = EncSessions::with_cap(2);
        let r = b.reserve(1).expect("first");
        b.keep(r);
        for _ in 0..100 {
            assert_eq!(b.released(2), 0);
        }
        assert_eq!(b.total(), 1);
        assert_eq!(b.of(1), 1);
    }

    /// The client's free returns every slot it held, and they are free again for the VM.
    #[test]
    fn a_client_free_returns_its_slots() {
        let mut b = EncSessions::with_cap(3);
        for _ in 0..3 {
            let r = b.reserve(9).expect("under the cap");
            b.keep(r);
        }
        assert_eq!(b.take_client(9), 3);
        assert_eq!(b.take_client(9), 0, "a second free finds nothing");
        assert_eq!(b.total(), 0);
        let r = b.reserve(10).expect("free again");
        b.keep(r);
        assert_eq!(b.total(), 1);
    }

    /// The production number is the documented one, and leaves the host headroom on the 8-slot
    /// GeForce limit even with two VMs at the cap.
    #[test]
    fn the_production_cap_is_the_documented_number() {
        assert_eq!(ENC_SESSIONS_PER_VM, 3);
        const { assert!(2 * ENC_SESSIONS_PER_VM < 8) };
        assert_eq!(EncSessions::default().cap, ENC_SESSIONS_PER_VM);
    }
}
