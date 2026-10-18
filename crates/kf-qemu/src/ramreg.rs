//! ★ Registering guest RAM with the host RM — the seam under [`crate::mem::MemPlane::guest_ram_object`].
//!
//! The registration is ONE `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over the whole guest memfd, so that
//! a sysmem leaf at memfd offset `o` is object offset `o` (single-handle offsets, native USERD /
//! notifier bindings and exact-start unmaps all rest on that; none of it is touched here).
//! The host driver pins the range under the process's mmap lock. On a memfd nobody has touched,
//! that pin also allocates and zeroes every page — ~12 GiB for the Windows VM — inside the lock,
//! while a vCPU thread that needs the mmap write lock (a malloc arena grow inside a memory-topology
//! commit, BQL held) queues behind it and the main loop queues behind the BQL.
//!
//! So the pages are made present FIRST, here, in named-size chunks ([`PREWARM_POPULATE_CHUNK`])
//! on the VA-manager thread (`fallocate` takes no mmap lock; each call returns), and only then is
//! the one registration issued: the pin finds present pages. Event-driven, no timer, no flag:
//! the order is the only mechanism. A backing that cannot be populated this way is not an error:
//! the registration then proceeds as before (the pin faults the pages in itself).

use std::time::Instant;

/// The most bytes ONE populate call may cover. Named so that one call's cost is bounded and
/// reviewable: it allocates and zeroes this many bytes (two `fallocate` passes, MEASURED ~0.58 s/GiB
/// on the dev box, so ~37 ms), takes no mmap lock, and then returns. A multiple of every base and
/// 2 MiB huge page size.
pub const PREWARM_POPULATE_CHUNK: u64 = 64 << 20;

/// What the guest-RAM backing offers the registration.
pub trait RamRegistrar {
    /// Make the backing bytes `[off, off+len)` present (allocated, zero where new). `Err` means
    /// "this backing cannot be populated this way", by name.
    ///
    /// # Errors
    /// The backing's refusal.
    fn populate(&self, off: u64, len: u64) -> Result<(), String>;

    /// Register `[0, len)` of the backing as ONE host-RM object and return its handle.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn register(&self, len: u64) -> Result<u32, String>;
}

/// How the populate step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopulateOutcome {
    /// Every byte of `[0, len)` was covered, in this many calls.
    Populated {
        /// Number of populate calls made.
        chunks: u64,
    },
    /// The backing refused at byte `at`; nothing after `at` was populated and the registration
    /// ran anyway (the pin faults the rest in, as before this step existed).
    Declined {
        /// First byte not populated.
        at: u64,
        /// The refusal.
        why: String,
    },
}

/// One registration's result and its two costs, separately, so a boot log can say which one a
/// stall belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    /// The host-RM object handle.
    pub obj: u32,
    /// How populating ended.
    pub populate: PopulateOutcome,
    /// Microseconds spent populating.
    pub populate_us: u128,
    /// Microseconds spent in the single registration call.
    pub register_us: u128,
}

/// Register `len` bytes of guest RAM: populate in chunks of at most [`PREWARM_POPULATE_CHUNK`],
/// then ONE registration.
///
/// # Errors
/// The registration's refusal. A populate refusal is not an error ([`PopulateOutcome::Declined`]).
pub fn register_guest_ram(r: &dyn RamRegistrar, len: u64) -> Result<Registered, String> {
    let t0 = Instant::now();
    let mut populate = PopulateOutcome::Populated { chunks: 0 };
    let mut off = 0;
    while off < len {
        let n = PREWARM_POPULATE_CHUNK.min(len - off);
        if let Err(why) = r.populate(off, n) {
            // Not fatal: this backing cannot be populated this way (or the host is short of
            // memory, which the registration will then report by its own name). The pin faults
            // in whatever is left, exactly as before this step existed.
            populate = PopulateOutcome::Declined { at: off, why };
            break;
        }
        off += n;
        if let PopulateOutcome::Populated { chunks } = &mut populate {
            *chunks += 1;
        }
    }
    let populate_us = t0.elapsed().as_micros();
    let t1 = Instant::now();
    let obj = r.register(len)?;
    Ok(Registered {
        obj,
        populate,
        populate_us,
        register_us: t1.elapsed().as_micros(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_linux_raw::SharedRam;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Ev {
        Populate { off: u64, len: u64 },
        Register { len: u64, holes: u64 },
    }

    /// A registrar over a REAL sparse memfd: `populate` is the production `fallocate` helper (or a
    /// scripted refusal), `register` measures the holes the memfd has AT THE MOMENT of the call —
    /// what the kernel's pin would have to fault in.
    struct Fake {
        ram: SharedRam,
        log: RefCell<Vec<Ev>>,
        refuse_from: Option<u64>,
        register_result: Result<u32, String>,
    }

    static SEQ: AtomicU64 = AtomicU64::new(0);

    impl Fake {
        fn new(len: u64) -> Fake {
            let name = std::ffi::CString::new(format!(
                "kf-ramreg-test-{}",
                SEQ.fetch_add(1, Ordering::Relaxed)
            ))
            .expect("name");
            Fake {
                ram: SharedRam::create_named(&name, len).expect("memfd"),
                log: RefCell::new(Vec::new()),
                refuse_from: None,
                register_result: Ok(0xcafe),
            }
        }
        fn events(&self) -> Vec<Ev> {
            self.log.borrow().clone()
        }
        fn populates(&self) -> Vec<(u64, u64)> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    Ev::Populate { off, len } => Some((off, len)),
                    Ev::Register { .. } => None,
                })
                .collect()
        }
        fn registers(&self) -> Vec<(u64, u64)> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    Ev::Register { len, holes } => Some((len, holes)),
                    Ev::Populate { .. } => None,
                })
                .collect()
        }
    }

    impl RamRegistrar for Fake {
        fn populate(&self, off: u64, len: u64) -> Result<(), String> {
            self.log.borrow_mut().push(Ev::Populate { off, len });
            if self.refuse_from.is_some_and(|r| off >= r) {
                return Err("scripted: EOPNOTSUPP".to_owned());
            }
            kf_linux_raw::populate_range(self.ram.as_backing_fd(), off, len)
                .map_err(|e| format!("{e:?}"))
        }
        fn register(&self, len: u64) -> Result<u32, String> {
            let holes = kf_linux_raw::hole_bytes(self.ram.as_backing_fd(), len).expect("holes");
            self.log.borrow_mut().push(Ev::Register { len, holes });
            self.register_result.clone()
        }
    }

    const PAGE: u64 = 4096;
    /// 160 MiB = 2.5 chunks of the 64 MiB constant: two full chunks and a half one, so every boundary
    /// case is on the path. A LITERAL on purpose: derived from the constant it would follow a
    /// mutated constant and hide it.
    const LEN: u64 = 160 << 20;
    /// An independent ceiling for one populate call, not derived from the constant under test.
    const CALL_CEILING: u64 = 128 << 20;
    /// The first chunk of [`LEN`] as a literal (see [`LEN`]).
    const SIXTY_FOUR_MIB: u64 = 64 << 20;

    #[test]
    fn the_pin_finds_no_hole_to_fault_in() {
        let f = Fake::new(LEN);
        assert_eq!(
            kf_linux_raw::hole_bytes(f.ram.as_backing_fd(), LEN).expect("holes"),
            LEN,
            "precondition: a fresh memfd is all hole"
        );
        register_guest_ram(&f, LEN).expect("register");
        assert_eq!(
            f.registers(),
            vec![(LEN, 0)],
            "holes at the registration call"
        );
    }

    #[test]
    fn populate_calls_are_bounded_and_tile_the_whole_range() {
        let f = Fake::new(LEN);
        let got = register_guest_ram(&f, LEN).expect("register");
        let calls = f.populates();
        assert!(!calls.is_empty(), "nothing was populated");
        let mut next = 0;
        for (off, len) in &calls {
            assert_eq!(*off, next, "calls must be contiguous from 0: {calls:?}");
            assert!(
                *len > 0 && *len <= PREWARM_POPULATE_CHUNK && *len <= CALL_CEILING,
                "a call of {len:#x} bytes exceeds the chunk {PREWARM_POPULATE_CHUNK:#x} / {CALL_CEILING:#x}"
            );
            next += len;
        }
        assert_eq!(next, LEN, "calls must cover the whole length: {calls:?}");
        assert_eq!(
            got.populate,
            PopulateOutcome::Populated {
                chunks: calls.len() as u64
            }
        );
        assert_eq!(calls.len(), 3, "2.5 chunks are three calls");
    }

    #[test]
    fn every_populate_precedes_the_one_registration() {
        let f = Fake::new(LEN);
        register_guest_ram(&f, LEN).expect("register");
        let ev = f.events();
        let reg = ev
            .iter()
            .position(|e| matches!(e, Ev::Register { .. }))
            .expect("registered");
        assert_eq!(reg, ev.len() - 1, "the registration is last: {ev:?}");
        assert!(ev[..reg].iter().all(|e| matches!(e, Ev::Populate { .. })));
    }

    #[test]
    fn the_registration_stays_one_descriptor_over_the_whole_length() {
        let f = Fake::new(LEN);
        let got = register_guest_ram(&f, LEN).expect("register");
        assert_eq!(f.registers().len(), 1, "exactly one registration call");
        assert_eq!(
            f.registers()[0].0,
            LEN,
            "over the whole length, from offset 0"
        );
        assert_eq!(got.obj, 0xcafe);
    }

    #[test]
    fn less_ram_than_one_chunk_is_one_exact_call() {
        let len = 5 * PAGE;
        let f = Fake::new(len);
        register_guest_ram(&f, len).expect("register");
        assert_eq!(f.populates(), vec![(0, len)]);
        assert_eq!(f.registers(), vec![(len, 0)]);
    }

    #[test]
    fn a_refusing_backing_falls_back_to_the_unpopulated_registration() {
        let mut f = Fake::new(LEN);
        f.refuse_from = Some(SIXTY_FOUR_MIB);
        let got = register_guest_ram(&f, LEN).expect("a populate refusal is not an error");
        assert_eq!(
            got.populate,
            PopulateOutcome::Declined {
                at: SIXTY_FOUR_MIB,
                why: "scripted: EOPNOTSUPP".to_owned()
            }
        );
        // The first chunk was populated, the refused one stopped the loop (no further calls),
        // and the registration still ran, once, over everything.
        assert_eq!(f.populates().len(), 2, "stops at the first refusal");
        let regs = f.registers();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].0, LEN);
        assert_eq!(
            regs[0].1,
            LEN - SIXTY_FOUR_MIB,
            "only the first chunk is present"
        );
    }

    #[test]
    fn a_backing_that_cannot_fallocate_at_all_is_not_fatal() {
        // /dev/null: the real kernel answers ENODEV to fallocate. Production's own helper, a real fd.
        struct Null(std::fs::File, std::cell::Cell<u32>);
        impl RamRegistrar for Null {
            fn populate(&self, off: u64, len: u64) -> Result<(), String> {
                use std::os::fd::AsFd;
                kf_linux_raw::populate_range(self.0.as_fd(), off, len).map_err(|e| format!("{e:?}"))
            }
            fn register(&self, _len: u64) -> Result<u32, String> {
                self.1.set(self.1.get() + 1);
                Ok(7)
            }
        }
        let null = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .expect("/dev/null");
        let p = Null(null, std::cell::Cell::new(0));
        let got = register_guest_ram(&p, 8 * PAGE).expect("fallback");
        assert!(matches!(
            got.populate,
            PopulateOutcome::Declined { at: 0, .. }
        ));
        assert_eq!(p.1.get(), 1, "the registration still ran, once");
    }

    #[test]
    fn a_registration_refusal_is_returned_after_the_populate() {
        let mut f = Fake::new(3 * PAGE);
        f.register_result = Err("host RM: refused".to_owned());
        let e = register_guest_ram(&f, 3 * PAGE).expect_err("refusal");
        assert_eq!(e, "host RM: refused");
        assert_eq!(f.populates(), vec![(0, 3 * PAGE)]);
    }

    #[test]
    fn the_chunk_is_a_named_bound_that_is_sane() {
        assert_eq!(PREWARM_POPULATE_CHUNK % (2 << 20), 0, "a 2 MiB multiple");
        assert!(PREWARM_POPULATE_CHUNK >= 4 << 20, "not degenerate");
        assert!(
            PREWARM_POPULATE_CHUNK <= 128 << 20,
            "one call is allocate+zero of this many bytes; keep it a few tens of ms"
        );
    }
}
