//! The **single store**: one host memory object for the VM's lifetime, carved into slices.
//!
//! ⊘ `[measured, w724/E1]` the store reserves the guest's framebuffer as **ONE** host object
//! (11 904 MiB in that run) rather than one object per leaf. The reason is a measured ceiling, not
//! tidiness: `vm.max_map_count` is **65 530** and a per-leaf scheme reached **15 845** live pins
//! on one workload (w291), so per-leaf objects do not scale to a large guest.
//!
//! ★ Safe code holds a [`StoreOffset`], never a pointer. Turning an offset into something
//! dereferenceable is the host adapter's job, behind its own `unsafe` — so a bug in this crate
//! cannot become a bad dereference (§"no VMM pointer in safe code").

use crate::addr::{page_cover, HostToken, StoreOffset, PAGE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRefusal {
    /// The store is full. ⊘ Refused by name — never grown, because the store's whole point is
    /// that its size is decided once and its address never moves.
    Exhausted { want: u64, left: u64 },
    /// A zero-length carve.
    ZeroLength,
    /// The requested span is larger than the store could ever hold.
    LargerThanStore { want: u64, store: u64 },
}

impl StoreRefusal {
    pub fn name(&self) -> &'static str {
        match self {
            StoreRefusal::Exhausted { .. } => "store_exhausted",
            StoreRefusal::ZeroLength => "store_zero_length",
            StoreRefusal::LargerThanStore { .. } => "larger_than_store",
        }
    }
}

/// One host object, carved bump-wise. ⊘ No free list yet: the store is VM-lifetime and the
/// teardown order (§9) frees it whole. A free list is where a use-after-free would live, so it is
/// not added until something needs it.
#[derive(Debug)]
pub struct Store {
    token: HostToken,
    len: u64,
    next: u64,
}

impl Store {
    /// ⊘ `token` names a host object the adapter already created. This crate never allocates.
    pub fn new(token: HostToken, len: u64) -> Store {
        Store { token, len: len & !(PAGE - 1), next: 0 }
    }

    pub fn token(&self) -> HostToken {
        self.token
    }
    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn used(&self) -> u64 {
        self.next
    }
    pub fn left(&self) -> u64 {
        self.len - self.next
    }

    /// Carve `len` bytes. ⊘ Page-covered, for the same reason the join is: a sub-page carve would
    /// let two leaves share a page and make one guest's write visible in another's window.
    pub fn carve(&mut self, len: u64) -> Result<StoreOffset, StoreRefusal> {
        if len == 0 {
            return Err(StoreRefusal::ZeroLength);
        }
        let (_, need) = page_cover(0, len);
        if need > self.len {
            return Err(StoreRefusal::LargerThanStore { want: need, store: self.len });
        }
        if need > self.left() {
            return Err(StoreRefusal::Exhausted { want: need, left: self.left() });
        }
        let at = StoreOffset(self.next);
        self.next += need;
        Ok(at)
    }
}
