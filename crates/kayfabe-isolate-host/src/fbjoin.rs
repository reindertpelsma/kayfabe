//! ★★★★★ **The isolate's half of a joined framebuffer leaf** — `fb_cpu_view.md` §4.
//!
//! One table per **isolate**, holding, for each leaf the VMM asked to join, the isolate's
//! own `mmap` of the fabricated backing that leaf now lives in.
//!
//! ## ★★ Why the table is per-isolate and NOT on the backend
//!
//! An isolate is a **pool**. `crate::child` runs one [`crate::rm::HostRmBackend`] per worker
//! thread, and the VMM's requests are served by whichever worker's socket is idle — a join
//! and the read of it need not land on the same one. A table on the backend would therefore
//! be correct on every one-worker test, on every `cargo test` in this workspace and in
//! Clippy, and wrong at the first boot with a pool of two. That is not a hypothetical: it is
//! the failure `w229` measured and fixed one plane over, and the reason this module exists
//! as a shared object rather than as three fields on a struct.
//!
//! ⊘ There is deliberately **no** constructor path that gives a backend a private table.
//! `HostRmBackend::with_fb_joins` installs a shared one or the backend has none, and a
//! backend with none **refuses the verb by name** ([`crate::rm::FB_JOIN_NO_TABLE`]) rather
//! than minting one it alone can see.
//!
//! ## ⚠ What the mapping's lifetime is for
//!
//! RM pins the pages behind an `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` at allocation time, and
//! keeps them pinned until the object is freed. Tearing the mapping out from under a live
//! descriptor leaves the GPU MMU pointed at pages this process no longer describes — so the
//! [`kayfabe_linux_raw::MappedRegion`] is held **here**, for as long as the isolate, and the
//! descriptor is freed through the ordinary [`kayfabe_isolate::VerbPlan::Release`] path.
//!
//! ⊘ The `memfd` descriptor is **not** kept: `Backing::SharedFile`'s own docs record that the
//! mapping outlives the descriptor, and [`crate::export::ChildExports`] already holds the
//! authoritative end of it. A second copy here would be a second lifetime for one file.

use kayfabe_linux_raw::{HostOffset, MappedRegion, RawError};
use std::sync::Mutex;

/// One joined leaf, as the isolate sees it.
#[derive(Debug)]
struct Join {
    /// The emulated device's framebuffer-physical base of this leaf. ⊘ The **VMM's**
    /// number: nothing in this process derives it, and it is the only key by which the
    /// instrument can name a range.
    phys: u64,
    /// Bytes.
    len: u64,
    /// The GPU VA the descriptor was placed at. Recorded for diagnostics; ⊘ never used as a
    /// key, because two address spaces may legitimately bind one VA and only `phys` is
    /// unique to the emulated device.
    at: u64,
    /// The isolate's view of the backing. See the module docs for why it lives this long.
    region: MappedRegion,
}

/// ★★★ Every framebuffer leaf this isolate has joined.
#[derive(Debug, Default)]
pub struct FbJoinTable {
    joins: Mutex<Vec<Join>>,
    /// ★★★★★ **LEG A2 — the raw handles of the `OS_DESCRIPTOR`s this isolate minted BY
    /// JOINING**, and therefore the complete set of objects a channel may be born over.
    ///
    /// # ⊘ Why the set lives HERE and not on the worker
    ///
    /// An isolate is a **pool**: the worker that joins a leaf need not be the worker later
    /// asked to birth a channel over it. A per-worker set is a bug no single-worker test can
    /// observe — the same reason [`FbJoinTable`] itself is shared, and one this campaign has
    /// already paid for once.
    ///
    /// # ★★★ What it enforces, and why a private constructor could not
    ///
    /// The owner's invariant is *no fake framebuffer at a real GPU VA of an isolate except
    /// the scratchpad*. `kayfabe_isolate::AdoptedGuestRing` is built in the core from an
    /// address-table binding declaring `BackingBytes::JoinsGuestWindow` — but it crosses an
    /// **IPC boundary** as four integers and is rebuilt in the child, which cannot see the
    /// address table. ⇒ Membership of this set is the far-side re-check, and it is the only
    /// one that is not defeated by the crossing.
    joined_objects: Mutex<std::collections::BTreeSet<u32>>,
}

impl FbJoinTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt the isolate-side mapping of a leaf that has just been described to RM and
    /// placed.
    ///
    /// ⊘ Called **after** the RM chain succeeds, never before: a table entry for a leaf whose
    /// `map_gpu_va` refused would answer the instrument about memory no engine can reach,
    /// which is a green line for a join that does not exist.
    pub fn install(&self, phys: u64, len: u64, at: u64, region: MappedRegion) {
        let mut t = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        t.push(Join {
            phys,
            len,
            at,
            region,
        });
    }

    /// ★★★★★ **LEG A2 — record that `obj` is an object this isolate minted by JOINING** a
    /// framebuffer leaf, i.e. one memory with the guest's own window.
    ///
    /// ⊘ Called on the same success path as [`Self::install`] and for the same reason: an
    /// object whose fixed map refused is not a joined window, and remembering it would let a
    /// channel be born over memory the guest cannot reach.
    pub fn remember_object(&self, obj: u32) {
        self.joined_objects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(obj);
    }

    /// ★★★★★ **Is `obj` a joined window?** — the far-side half of the owner invariant.
    ///
    /// ⊘ A `false` is a REFUSAL, never a downgrade: the caller must refuse the birth by
    /// name, not fall back to allocating a ring of its own. Falling back would make an armed
    /// evidence run and its own control produce the same channel.
    #[must_use]
    pub fn is_joined_object(&self, obj: u32) -> bool {
        self.joined_objects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&obj)
    }

    /// How many leaves are joined. Diagnostics only.
    #[must_use]
    pub fn len(&self) -> usize {
        self.joins.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether none are.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every joined leaf as `(phys, len, at)`, ascending by `phys` — for a boot line that
    /// can state what this isolate is holding.
    ///
    /// ★ Sorted, for [`kayfabe_device::FbStore::resident_frames`]'s reason: an insertion-
    /// ordered list makes two boots of one binary produce differently-ordered evidence.
    #[must_use]
    pub fn census(&self) -> Vec<(u64, u64, u64)> {
        let t = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<(u64, u64, u64)> = t.iter().map(|j| (j.phys, j.len, j.at)).collect();
        v.sort_unstable();
        v
    }

    /// ★★★ **The instrument** — read `buf.len()` bytes at framebuffer address `phys` out of
    /// the isolate's own view, then (when `poke` is `Some`) leave a per-word pattern there.
    ///
    /// `Ok(false)` = **no joined leaf covers the whole range**. ⊘ Never zeros: an unjoined
    /// range answering with a zero fill is indistinguishable from a joined one holding
    /// zeros, and those are opposite findings.
    ///
    /// ★ The read happens **before** the poke, always. That ordering is what makes one round
    /// trip carry both directions: the reply is what the *other* side wrote, and the poke is
    /// the stimulus the other side then reads. Reversed, the caller would be handed its own
    /// pattern back and every run would pass.
    ///
    /// ⊘ The pattern is **per word** — word *i* is `base + i` — never a repeated constant.
    /// A whole-buffer compare against one repeated word passes on any single correct word,
    /// and a truncated or misaddressed read would still match.
    ///
    /// # Errors
    /// Whatever the mapping refused the access with.
    pub fn peek(&self, phys: u64, buf: &mut [u8], poke: Option<u32>) -> Result<bool, RawError> {
        let t = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        let want = buf.len() as u64;
        let Some((j, off)) = t.iter().find_map(|j| {
            let end = phys.checked_add(want)?;
            let jend = j.phys.checked_add(j.len)?;
            (phys >= j.phys && end <= jend).then(|| (j, phys - j.phys))
        }) else {
            return Ok(false);
        };
        j.region.read_into(HostOffset::new(off), buf)?;
        if let Some(base) = poke {
            let mut image = vec![0u8; buf.len()];
            for (i, w) in image.chunks_exact_mut(4).enumerate() {
                w.copy_from_slice(&base.wrapping_add(i as u32).to_le_bytes());
            }
            j.region.write_from(HostOffset::new(off), &image)?;
        }
        Ok(true)
    }
}

kayfabe_util::assert_send_sync!(FbJoinTable);

#[cfg(test)]
mod tests {
    use super::*;
    use kayfabe_linux_raw::{Backing, CachePolicy, HostPageSize, HostProt, SharedRam};

    fn joined(phys: u64, len: u64) -> FbJoinTable {
        let ram = SharedRam::create(len).expect("memfd");
        let fd = ram.dup_for_export().expect("dup");
        let region = MappedRegion::map(
            Backing::SharedFile {
                fd: std::os::fd::AsFd::as_fd(&fd),
                offset: 0,
            },
            len,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            HostPageSize::query(),
        )
        .expect("map");
        let t = FbJoinTable::new();
        t.install(phys, len, 0x2_0020_0000, region);
        // ⊘ `ram` is dropped here on purpose: the mapping outlives the descriptor, which is
        // the property the production path relies on and which a test that kept the fd
        // alive would never exercise.
        t
    }

    /// ★ `Ok(false)` for an address no join covers — the MISS the whole instrument turns on.
    #[test]
    fn an_unjoined_address_is_a_miss_and_not_a_page_of_zeros() {
        let t = joined(0x1_0000, 0x1_0000);
        let mut buf = [0u8; 64];
        assert_eq!(t.peek(0x9_0000, &mut buf, None), Ok(false));
        // ⊘ And a range that STARTS inside and ends outside is also a miss, refused whole:
        // a truncated answer is a wrong answer wearing a partial-success costume.
        let mut edge = [0u8; 128];
        assert_eq!(t.peek(0x1_ffc0, &mut edge, None), Ok(false));
    }

    /// ★★ The poke is written AFTER the read, so one round trip carries both directions.
    #[test]
    fn the_read_precedes_the_poke_so_one_call_carries_both_directions() {
        let t = joined(0x1_0000, 0x1_0000);
        let mut first = [0u8; 64];
        assert_eq!(t.peek(0x1_0000, &mut first, Some(0xa19a_5a5b)), Ok(true));
        // The first read saw the memfd's own zero fill — not the pattern it was about to
        // write. A reversed implementation would have returned the pattern here.
        assert_eq!(first, [0u8; 64]);
        let mut second = [0u8; 64];
        assert_eq!(t.peek(0x1_0000, &mut second, None), Ok(true));
        assert_eq!(
            u32::from_le_bytes(second[0..4].try_into().unwrap()),
            0xa19a_5a5b
        );
        // ⊘ Per-word, so a store that wrote one repeated word would fail here.
        assert_eq!(
            u32::from_le_bytes(second[4..8].try_into().unwrap()),
            0xa19a_5a5c
        );
    }
}
