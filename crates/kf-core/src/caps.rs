//! Per-VM caps on host twins — §9.1.
//!
//! > *"The host driver enforces **no per-client quota**. ⇒ On a host with more than one VM, one
//! > guest can **starve the others** by allocating twins — channels, engine objects, address
//! > spaces, and one host object per non-coalescable system-memory leaf."*
//!
//! ★★★ **And the cap is not a number we invent.** §9.1: *"**We already told the guest how many
//! channels it may have**, so that number is the cap, and the same applies to every other twin
//! class: **refuse past it, by name, counted.**"*
//!
//! ⇒ That is the whole design: the guest was handed a limit in `GET_GSP_STATIC_INFO`, so enforcing
//! exactly that limit is **not a policy choice** — a guest exceeding it has already ignored what we
//! told it, and refusing is consistent rather than arbitrary. ⊘ Inventing a *different* number
//! would be a policy, and would be wrong in one of two directions for every workload.

/// The twin classes §9.1 names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Twin {
    Channel,
    EngineObject,
    AddressSpace,
    /// *"one host object per non-coalescable system-memory leaf"* — the one that scales with guest
    /// behaviour rather than with guest configuration, and so the one most able to starve a
    /// neighbour.
    SysmemLeaf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// ⊘ By name and counted, never a silent failure or a generic OOM.
    OverDeclaredCap { twin: Twin, cap: u32, asked: u32 },
}

/// One VM's twin accounting.
#[derive(Debug)]
pub struct VmCaps {
    caps: [u32; 4],
    live: [u32; 4],
    refused: [u64; 4],
}

fn ix(t: Twin) -> usize {
    match t {
        Twin::Channel => 0,
        Twin::EngineObject => 1,
        Twin::AddressSpace => 2,
        Twin::SysmemLeaf => 3,
    }
}

impl VmCaps {
    /// ⊘ Built from **what we told the guest**, not from a tuned constant.
    pub fn from_declared(channels: u32, engine_objects: u32, address_spaces: u32, sysmem_leaves: u32) -> VmCaps {
        VmCaps {
            caps: [channels, engine_objects, address_spaces, sysmem_leaves],
            live: [0; 4],
            refused: [0; 4],
        }
    }

    pub fn acquire(&mut self, t: Twin) -> Result<(), Refusal> {
        let i = ix(t);
        if self.live[i] >= self.caps[i] {
            self.refused[i] += 1;
            return Err(Refusal::OverDeclaredCap { twin: t, cap: self.caps[i], asked: self.live[i] + 1 });
        }
        self.live[i] += 1;
        Ok(())
    }

    pub fn release(&mut self, t: Twin) {
        let i = ix(t);
        self.live[i] = self.live[i].saturating_sub(1);
    }

    #[inline]
    pub fn live(&self, t: Twin) -> u32 {
        self.live[ix(t)]
    }
    /// ⚠ Counted, so a zero is evidence. A guest that never hits a cap and a cap that is never
    /// enforced look identical without this.
    #[inline]
    pub fn refused(&self, t: Twin) -> u64 {
        self.refused[ix(t)]
    }
}

/// The guest's framebuffer aperture — §9.1's second half.
///
/// > *"the guest's framebuffer aperture is a **device-global** resource on the host. The window
/// > through which a CPU sees GPU memory is a fixed size shared by every process on that GPU. ⇒
/// > Size the guest's at startup **from what is actually free**, and make an unbackable page a
/// > **named refusal** rather than a silent hole — **the guest reading an unbacked hole is a wrong
/// > answer, not an error.**"*
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backing {
    Backed { host_offset: u64 },
    /// ⊘ Named, never a hole. A hole returns plausible bytes and the guest proceeds on them.
    RefusedUnbackable,
}

#[derive(Debug)]
pub struct Aperture {
    len: u64,
    backed_len: u64,
    refusals: u64,
}

impl Aperture {
    /// ⊘ `free_now` is measured at startup, not assumed from the board's nominal size: the window
    /// is shared with every other process on that GPU.
    pub fn size_from_free(requested: u64, free_now: u64) -> Aperture {
        let len = requested.min(free_now);
        Aperture { len, backed_len: len, refusals: 0 }
    }

    pub fn back(&mut self, offset: u64, len: u64) -> Backing {
        match offset.checked_add(len) {
            Some(end) if end <= self.backed_len => Backing::Backed { host_offset: offset },
            _ => {
                self.refusals += 1;
                Backing::RefusedUnbackable
            }
        }
    }

    #[inline]
    pub fn len(&self) -> u64 {
        self.len
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[inline]
    pub fn refusals(&self) -> u64 {
        self.refusals
    }
}
