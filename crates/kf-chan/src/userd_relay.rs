//! ★★ 2026-10-08 — **the USERD relay's step** (`docs/design/V3_USERD_RELAY.md` §2), pure so its bounds and
//! its ordering are tested without a GPU.
//!
//! A Passthrough twin whose USERD the host cannot adopt (a Windows per-process channel's USERD is a slot
//! of the guest RM's system-memory pool, at an IOVA wider than a USERD may be) gets a kayfabe-owned USERD.
//! Each doorbell of the channel runs one [`step`] on a worker: the guest's `GP_PUT` is loaded, bounded to
//! the ring's entry count and, when it moved, stored into the twin's USERD and rung; then the engine's
//! `GP_GET` is written back into the guest's slot. Nothing else is read or copied: no GP entry, no
//! push-buffer word.

/// The relay's own state, per twin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayState {
    /// `gpFifoEntries` as the guest declared it at the channel's alloc (fixed for the twin's life).
    pub entries: u32,
    /// The `GP_PUT` last stored into the twin's USERD.
    pub host_put: u32,
    /// Doorbells forwarded (a new `GP_PUT` stored and rung).
    pub forwarded: u64,
    /// Guest `GP_PUT` values refused (outside `[0, entries)`).
    pub refused: u64,
    /// `GP_GET` write-backs made.
    pub gets: u64,
    /// ★ 2026-10-08 (`KF3_RELAY_GET_REFRESH`, [`refresh`]): the `GP_GET` last stored into the guest's slot.
    pub guest_get: u32,
    /// Refreshes that stored a new `GP_GET` (outside a doorbell step).
    pub refreshed: u64,
    /// Engine `GP_GET` values outside `[0, entries)`: never stored into the guest's slot by a refresh.
    pub get_refused: u64,
}

impl RelayState {
    /// A relay for a ring of `entries` GP entries, its USERD freshly zeroed by host RM.
    #[must_use]
    pub fn new(entries: u32) -> RelayState {
        RelayState {
            entries,
            host_put: 0,
            forwarded: 0,
            refused: 0,
            gets: 0,
            guest_get: 0,
            refreshed: 0,
            get_refused: 0,
        }
    }
}

/// The memory a step touches — four bytes each way, never more.
pub trait RelayIo {
    /// A plain load of `GP_PUT` from the guest's USERD slot.
    ///
    /// # Errors
    /// The slot cannot be read.
    fn guest_put(&mut self) -> Result<u32, String>;
    /// A plain store of `GP_PUT` into the twin's (kayfabe-owned) USERD.
    ///
    /// # Errors
    /// The view cannot be written.
    fn store_host_put(&mut self, put: u32) -> Result<(), String>;
    /// A release fence (stores before it are visible before stores after it).
    fn fence(&mut self);
    /// The host doorbell of the twin.
    ///
    /// # Errors
    /// The host refused it.
    fn ring(&mut self) -> Result<(), String>;
    /// A plain load of the engine-written `GP_GET` from the twin's USERD.
    ///
    /// # Errors
    /// The view cannot be read.
    fn host_get(&mut self) -> Result<u32, String>;
    /// A plain store of `GP_GET` into the guest's USERD slot.
    ///
    /// # Errors
    /// The slot cannot be written.
    fn store_guest_get(&mut self, get: u32) -> Result<(), String>;
}

/// What one step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A new `GP_PUT` was stored and rung.
    Forwarded(u32),
    /// The guest's `GP_PUT` had not moved: nothing rung.
    Unchanged,
    /// The guest's `GP_PUT` was outside the ring: not stored, not rung (fail closed).
    Refused(u32),
}

/// ★ One relay step (`V3_USERD_RELAY.md` §2.1-2.2), in this order: load the guest's `GP_PUT`; bound it;
/// if it moved — fence, store it into the twin's USERD, fence, ring; then write the twin's `GP_GET` back to
/// the guest. A refused `GP_PUT` still gets the write-back (it is host-derived and harmless).
///
/// # Errors
/// A memory or doorbell failure, by name; the state is left as it was before the failing access.
pub fn step(st: &mut RelayState, io: &mut dyn RelayIo) -> Result<Outcome, String> {
    let put = io.guest_put()?;
    let outcome = if put >= st.entries {
        st.refused += 1;
        Outcome::Refused(put)
    } else if put == st.host_put {
        Outcome::Unchanged
    } else {
        io.fence();
        io.store_host_put(put)?;
        io.fence();
        io.ring()?;
        st.host_put = put;
        st.forwarded += 1;
        Outcome::Forwarded(put)
    };
    let get = io.host_get()?;
    io.store_guest_get(get)?;
    st.gets += 1;
    st.guest_get = get;
    Ok(outcome)
}

/// What one [`refresh`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// The engine's `GP_GET` moved since the last store: stored into the guest's slot.
    Stored(u32),
    /// The engine's `GP_GET` is the one the guest already has: nothing stored.
    Same,
    /// The engine's `GP_GET` is outside the ring (not a value the engine writes for this ring):
    /// nothing stored, counted.
    Refused(u32),
}

/// ★ 2026-10-08 (`V3_USERD_RELAY.md` §2.2's follow-up, `KF3_RELAY_GET_REFRESH`, default off) — **the
/// `GP_GET` write-back alone, outside a doorbell**: a plain load of the engine-written `GP_GET` from the
/// twin's USERD and, when it moved, a plain store of it into the guest's slot. Run on a host event (an
/// engine's non-stall wake, BEFORE the guest's interrupt is raised, so the guest's handler reads the
/// fresh value) and on the worker's park tick while the relay lives. ⊘ It never loads the guest's
/// `GP_PUT`, never stores the twin's, never rings: the value written is the engine's own, never a guest
/// value and never a forged one. A value outside `[0, entries)` is not stored (fail closed, counted).
///
/// # Errors
/// A memory failure, by name; the state is left as it was.
pub fn refresh(st: &mut RelayState, io: &mut dyn RelayIo) -> Result<Refresh, String> {
    let get = io.host_get()?;
    if get >= st.entries {
        st.get_refused += 1;
        return Ok(Refresh::Refused(get));
    }
    if get == st.guest_get {
        return Ok(Refresh::Same);
    }
    io.store_guest_get(get)?;
    st.guest_get = get;
    st.refreshed += 1;
    Ok(Refresh::Stored(get))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Model {
        guest_put: u32,
        host_put: u32,
        host_get: u32,
        guest_get: u32,
        rung: u32,
        log: Vec<&'static str>,
        fail_ring: bool,
    }

    impl RelayIo for Model {
        fn guest_put(&mut self) -> Result<u32, String> {
            self.log.push("load guest PUT");
            Ok(self.guest_put)
        }
        fn store_host_put(&mut self, put: u32) -> Result<(), String> {
            self.log.push("store host PUT");
            self.host_put = put;
            Ok(())
        }
        fn fence(&mut self) {
            self.log.push("fence");
        }
        fn ring(&mut self) -> Result<(), String> {
            self.log.push("ring");
            if self.fail_ring {
                return Err("host refused".into());
            }
            self.rung += 1;
            Ok(())
        }
        fn host_get(&mut self) -> Result<u32, String> {
            self.log.push("load host GET");
            Ok(self.host_get)
        }
        fn store_guest_get(&mut self, get: u32) -> Result<(), String> {
            self.log.push("store guest GET");
            self.guest_get = get;
            Ok(())
        }
    }

    #[test]
    fn a_moved_put_is_stored_between_fences_then_rung_then_get_written_back() {
        let mut st = RelayState::new(32768);
        let mut m = Model {
            guest_put: 3,
            host_get: 1,
            ..Model::default()
        };
        assert_eq!(step(&mut st, &mut m), Ok(Outcome::Forwarded(3)));
        assert_eq!(
            m.log,
            [
                "load guest PUT",
                "fence",
                "store host PUT",
                "fence",
                "ring",
                "load host GET",
                "store guest GET"
            ]
        );
        assert_eq!((m.host_put, m.rung, m.guest_get), (3, 1, 1));
        assert_eq!((st.host_put, st.forwarded, st.gets), (3, 1, 1));
    }

    #[test]
    fn an_unmoved_put_rings_nothing_but_refreshes_get() {
        let mut st = RelayState::new(8);
        let mut m = Model {
            guest_put: 0,
            host_get: 0,
            ..Model::default()
        };
        assert_eq!(step(&mut st, &mut m), Ok(Outcome::Unchanged));
        assert_eq!(m.rung, 0);
        assert_eq!(
            m.log,
            ["load guest PUT", "load host GET", "store guest GET"]
        );
    }

    #[test]
    fn hostile_puts_outside_the_ring_never_reach_the_twin() {
        for (entries, put) in [
            (8, 8),
            (8, 9),
            (32768, 32768),
            (32768, u32::MAX),
            (1, 1),
            (2048, 0x8000_0000),
        ] {
            let mut st = RelayState::new(entries);
            let mut m = Model {
                guest_put: put,
                host_get: 0,
                ..Model::default()
            };
            assert_eq!(
                step(&mut st, &mut m),
                Ok(Outcome::Refused(put)),
                "{entries} {put}"
            );
            assert_eq!(m.rung, 0);
            assert_eq!(m.host_put, 0, "never stored");
            assert!(!m.log.contains(&"store host PUT"));
            assert_eq!(st.refused, 1);
            assert_eq!(st.host_put, 0);
        }
    }

    #[test]
    fn the_last_valid_index_is_forwarded_and_wraps_back_to_zero() {
        let mut st = RelayState::new(8);
        let mut m = Model {
            guest_put: 7,
            ..Model::default()
        };
        assert_eq!(step(&mut st, &mut m), Ok(Outcome::Forwarded(7)));
        m.guest_put = 0;
        assert_eq!(step(&mut st, &mut m), Ok(Outcome::Forwarded(0)));
        assert_eq!(m.rung, 2);
    }

    #[test]
    fn a_refused_doorbell_leaves_the_state_as_it_was() {
        let mut st = RelayState::new(8);
        let mut m = Model {
            guest_put: 5,
            fail_ring: true,
            ..Model::default()
        };
        assert!(step(&mut st, &mut m).is_err());
        assert_eq!(
            (st.host_put, st.forwarded),
            (0, 0),
            "the next step tries again"
        );
    }

    #[test]
    fn a_refresh_only_loads_the_engine_get_and_stores_it_never_put_never_ring() {
        let mut st = RelayState::new(8192);
        let mut m = Model {
            guest_put: 0x24,
            ..Model::default()
        };
        // The doorbell: forwarded, the engine has fetched nothing yet (run76's shape: one doorbell,
        // the guest's GP_GET left at 0 while the engine's reached 0x24).
        assert_eq!(step(&mut st, &mut m), Ok(Outcome::Forwarded(0x24)));
        assert_eq!(m.guest_get, 0);
        m.host_get = 0x24;
        m.log.clear();
        assert_eq!(refresh(&mut st, &mut m), Ok(Refresh::Stored(0x24)));
        assert_eq!(m.log, ["load host GET", "store guest GET"]);
        assert_eq!((m.guest_get, m.rung, m.host_put), (0x24, 1, 0x24));
        assert_eq!((st.guest_get, st.refreshed, st.forwarded), (0x24, 1, 1));
    }

    #[test]
    fn a_refresh_with_no_engine_progress_stores_nothing() {
        let mut st = RelayState::new(8);
        let mut m = Model::default();
        assert_eq!(refresh(&mut st, &mut m), Ok(Refresh::Same));
        assert_eq!(m.log, ["load host GET"]);
        assert_eq!(st.refreshed, 0);
    }

    #[test]
    fn a_refresh_never_stores_an_engine_get_outside_the_ring() {
        for get in [8u32, 9, u32::MAX] {
            let mut st = RelayState::new(8);
            let mut m = Model {
                host_get: get,
                guest_get: 3,
                ..Model::default()
            };
            assert_eq!(refresh(&mut st, &mut m), Ok(Refresh::Refused(get)));
            assert_eq!(m.guest_get, 3, "the guest's slot is untouched");
            assert!(!m.log.contains(&"store guest GET"));
            assert_eq!(st.get_refused, 1);
        }
    }

    #[test]
    fn refreshes_coalesce_and_a_later_step_keeps_them_consistent() {
        let mut st = RelayState::new(64);
        let mut m = Model {
            guest_put: 10,
            ..Model::default()
        };
        assert_eq!(step(&mut st, &mut m), Ok(Outcome::Forwarded(10)));
        m.host_get = 4;
        assert_eq!(refresh(&mut st, &mut m), Ok(Refresh::Stored(4)));
        // Several wakes with no engine progress in between: one store only.
        assert_eq!(refresh(&mut st, &mut m), Ok(Refresh::Same));
        assert_eq!(refresh(&mut st, &mut m), Ok(Refresh::Same));
        m.host_get = 10;
        assert_eq!(refresh(&mut st, &mut m), Ok(Refresh::Stored(10)));
        assert_eq!(st.refreshed, 2);
        // A doorbell step's write-back is recorded too, so the next refresh compares against it.
        m.guest_put = 12;
        assert_eq!(step(&mut st, &mut m), Ok(Outcome::Forwarded(12)));
        assert_eq!(st.guest_get, 10);
        m.host_get = 12;
        assert_eq!(refresh(&mut st, &mut m), Ok(Refresh::Stored(12)));
    }

    #[test]
    fn a_refresh_that_cannot_store_leaves_the_state_as_it_was() {
        struct NoStore;
        impl RelayIo for NoStore {
            fn guest_put(&mut self) -> Result<u32, String> {
                Ok(0)
            }
            fn store_host_put(&mut self, _: u32) -> Result<(), String> {
                Ok(())
            }
            fn fence(&mut self) {}
            fn ring(&mut self) -> Result<(), String> {
                Ok(())
            }
            fn host_get(&mut self) -> Result<u32, String> {
                Ok(5)
            }
            fn store_guest_get(&mut self, _: u32) -> Result<(), String> {
                Err("slot unmapped".into())
            }
        }
        let mut st = RelayState::new(8);
        assert!(refresh(&mut st, &mut NoStore).is_err());
        assert_eq!((st.guest_get, st.refreshed), (0, 0));
    }

    #[test]
    fn a_zero_entry_ring_refuses_every_put() {
        let mut st = RelayState::new(0);
        let mut m = Model::default();
        assert_eq!(step(&mut st, &mut m), Ok(Outcome::Refused(0)));
    }
}
