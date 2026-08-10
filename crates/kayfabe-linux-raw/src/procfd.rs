//! ★★★★★ The descriptors **this process already holds** — found by their PROPERTIES.
//!
//! `guest_ram_crossing.md` §3, option (A). A hypervisor that was launched with
//! `memory-backend-memfd,share=on` is holding guest RAM open as a `memfd`. A `memfd` has
//! no filesystem path, so nothing can *name* it — but it is an **open descriptor in this
//! process**, and a device built into that hypervisor is inside that process. This module
//! is how such a device finds it, and the whole design of the module is about the ways
//! that search can be wrong.
//!
//! ## ⊘⊘ Nothing here may key on a descriptor NUMBER, and that is measured
//!
//! `guest_ram_crossing.md` §4.5: the same image, the same flag, two physical boxes —
//! guest RAM was **fd 14 on `vh2` and fd 15 on `vh`**. A probe keyed on the number, or on
//! *"the first memfd"*, would have been right on one bench and would have silently
//! selected **the framebuffer** on the other. So [`MemfdCandidate::listed_as`] is
//! *reported*, for the log, and is never an input to a decision.
//!
//! The properties that ARE decided on:
//!
//! 1. the `readlink` of `/proc/self/fd/N` begins with the literal prefix `/memfd:` —
//!    ⊘ **a prefix of the whole link, never a substring of a path**. §1.1's trap 3 is a
//!    substring match on `memfd` that also matched the boot's *log file*, because the boot
//!    tag was `memfd1`;
//! 2. the name after that prefix matches **exactly**. ⊘ §1.1's trap 1: QEMU names the
//!    `memfd` after the **backend type** and not after your `id=`, so a lookup keyed on the
//!    `id=` finds nothing on a boot where guest RAM is sitting there the whole time — and
//!    an empty result reads as *"the backend is not there"*;
//! 3. it is **mapped `rw-s` in this process**. §1.1's trap 2 is that there are always at
//!    least two `memfd`s in a QEMU — `displaysurface` exists even in the control boot — so
//!    a scan that takes the first match gets the framebuffer. Being shared-mapped is what
//!    separates *"a `memfd` this process happens to hold"* from *"the memory this process
//!    is running a machine out of"*;
//! 4. and **exactly one** candidate matches. Two is not a weaker version of finding it; it
//!    is a different fact, and [`MemfdRefusal::Ambiguous`] says so by name.
//!
//! ## ★★★ This module needs NO new `unsafe_code` relaxation, and the reason is real
//!
//! `KvmVm::discover_in_this_process` had to `dup` a number read out of `/proc/self/fd`,
//! through `BorrowedFd::borrow_raw`, because a KVM descriptor is an **anonymous inode**:
//! re-opening `/proc/self/fd/N` answers `ENXIO`. A `memfd` is not an anonymous inode — it
//! is a real shmem file, and `/proc/self/fd/N` **re-opens**. That is the mechanism the
//! §1 measurement already used from another process entirely.
//!
//! ⇒ Every descriptor this module returns comes from `std::fs::OpenOptions`, so the
//! guest-RAM crossing costs **zero** relaxations on the containment ratchet — which is
//! why this file is not named `*_unsafe.rs` and needs no entry in the audited surface. It is
//! *stronger* than a `dup`: a re-open has its own open-file description, so the isolate's
//! descriptor does not share a file offset with the hypervisor's own.
//!
//! ## The race, and the ONE it does not close
//!
//! A number listed by `read_dir` can be closed and recycled before it is opened. So:
//! **read the link first** — which is also what stops this module from re-`open`ing an
//! arbitrary device node it has no business touching — then open, and then **re-derive
//! every property from the descriptor we now own**. Nothing else in the process can change
//! what our own descriptor names. Losing the race that way is a candidate that fails its
//! own checks, never a wrong block of memory.
//!
//! ⊘⊘ **And that argument is not sufficient, which was measured.** The number a census
//! recycles onto is most often *its own re-open*, and re-deriving the properties of that
//! descriptor confirms every one of them — it is a real `memfd` with the right name, size
//! and inode. A verification cannot tell an object from itself. See
//! [`MemfdCensus::take_of_this_process`] for the two mechanisms that do close it, and for
//! why the census is over **blocks** rather than descriptors.

use crate::error::RawError;
use std::collections::BTreeSet;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;

/// The `readlink` prefix the kernel gives every `memfd`. A **prefix of the whole link**.
const MEMFD_LINK_PREFIX: &str = "/memfd:";

/// What the kernel appends once the (always-unlinked) `memfd` inode has no name left.
const DELETED_SUFFIX: &str = " (deleted)";

/// One `memfd` this process holds open, re-opened so that we own it.
#[derive(Debug)]
pub struct MemfdCandidate {
    /// The `memfd`'s creation name, as `readlink` reports it — `memory-backend-memfd`,
    /// `displaysurface`, `kayfabe-guest-ram`.
    name: String,
    /// Its length, from `fstat` on **our** descriptor.
    bytes: u64,
    /// Its inode, from the same `fstat`. This is what a `/proc/self/maps` line is joined
    /// against, so that "is it mapped shared" is a fact about *this object* rather than
    /// about a path string that appeared twice.
    inode: u64,
    /// ★ Its `st_dev`, from the same `fstat`.
    ///
    /// An inode number is unique only within a device, and the consumer of this census now
    /// joins it against an identity the **hypervisor** reported for a region's backing file
    /// — a different process's `fstat` of a different descriptor. Carrying `dev` makes that
    /// join total rather than very-probably-right. Every `memfd` in fact lives on the one
    /// internal `shmem` mount, which is exactly why the missing half would never have been
    /// caught here.
    dev: u64,
    /// Whether some `rw-s` mapping in this process names this inode.
    shared_mapped: bool,
    /// ★ The number it was **listed at**, for the log only.
    ///
    /// ⊘ Deliberately not usable as a selector: this is the value that was 14 on one
    /// bench and 15 on another, and the field exists so a run can *report* what it found
    /// rather than so a caller can look for it again.
    listed_as: i32,
    /// Our own descriptor for it.
    file: File,
}

impl MemfdCandidate {
    /// The creation name, after the `/memfd:` prefix and before ` (deleted)`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its length in bytes.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Whether an `rw-s` mapping in this process names it.
    #[must_use]
    pub fn shared_mapped(&self) -> bool {
        self.shared_mapped
    }

    /// The descriptor number it was listed at — **for the log**, see the field's note.
    #[must_use]
    pub fn listed_as(&self) -> i32 {
        self.listed_as
    }

    /// Its inode, so a caller can prove two references are to one object.
    #[must_use]
    pub fn inode(&self) -> u64 {
        self.inode
    }

    /// Its `st_dev`, the other half of the filesystem identity. See the field's note for why
    /// both halves are carried.
    #[must_use]
    pub fn dev(&self) -> u64 {
        self.dev
    }

    /// Take the descriptor and the length the kernel reported for it.
    ///
    /// ★ The length is handed over **with** the descriptor, in one value, because the two
    /// are one fact: `GuestRamPlane` is documented to take the VMM's number rather than
    /// `lseek` for its own, and a pair that can be split is a pair that will be.
    #[must_use]
    pub fn into_descriptor(self) -> (OwnedFd, u64) {
        (OwnedFd::from(self.file), self.bytes)
    }
}

/// Why no single `memfd` could be selected. ⊘ Two arms, because *"there is none"* and
/// *"there are several"* are different operator problems with different fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemfdRefusal {
    /// Nothing in this process is a shared-mapped `memfd` of that name.
    NoSuchMemfd,
    /// More than one is. ⊘ Refused rather than resolved by any tie-break: every tie-break
    /// available here (lowest number, largest, first listed) is exactly the
    /// keyed-on-position rule §4.5 measured moving between two boxes.
    Ambiguous {
        /// How many matched.
        matched: usize,
    },
}

/// Every `memfd` open in this process, each re-opened under our own ownership.
///
/// Taking a census is deliberately separate from selecting from it: on a refusal the
/// caller must be able to print **what it did see**. `guest_ram_crossing.md` §1.1 trap 1
/// is a probe that found nothing and read as *"the backend is not there"* — the absence of
/// a match is not the absence of the thing, and the only cure is showing the list.
#[derive(Debug)]
pub struct MemfdCensus {
    seen: Vec<MemfdCandidate>,
}

impl MemfdCensus {
    /// Enumerate this process's `memfd`s.
    ///
    /// # Errors
    /// [`RawError::Syscall`] if `/proc/self/fd` cannot be listed at all — which means
    /// `/proc` is not mounted, and is a deployment fault rather than an empty answer.
    pub fn take_of_this_process() -> Result<Self, RawError> {
        let shared_inodes = shared_mapped_inodes();
        // ★★★★★ **THE CENSUS OCCUPIES THE NUMBERS IT IS ABOUT TO VISIT.**
        //
        // `[measured 2026-08-10, `cargo test -p kayfabe-linux-raw --lib procfd`]`, from a
        // debug print of `/proc/self/fd` before and after; held from now on by
        // `tests::a_shared_mapped_memfd_is_found_by_name_and_is_the_same_object`, which
        // fails with `left: 2, right: 1` the moment either mechanism below is removed.
        //
        // ⊘ And the first explanation was WRONG — it is written here in the form the debug
        // print produced rather than in the form it was guessed. The guess was *"`read_dir`
        // is lazy, so the loop enumerates its own descriptors"*; draining the listing first
        // changed nothing, because that was not the mechanism.
        //
        // `open` returns the **lowest free** descriptor number. The `read_dir` above holds
        // one (say 4) and releases it when it is drained; the loop then re-opens the
        // `memfd` at 3, and the kernel hands that re-open **number 4** — the very number
        // still sitting in the list, now naming a different object than when it was
        // listed. Visiting it finds a `memfd` of the right name, the right size and the
        // right inode: our own.
        //
        // ⊘ And note what does NOT close this. Re-deriving every property from the
        // descriptor we own — the discipline this module's header argues for, and which is
        // right for the *recycled onto something else* case — passes here with flying
        // colours, because the recycled number really is a legitimate `memfd` of exactly
        // the name asked for. A verification cannot distinguish an object from itself.
        //
        // ⊘⊘ And the consequence was not cosmetic. In production the duplicate carries the
        // same name, the same inode and the same shared-mapped state as guest RAM, so the
        // selector would have seen **two** matches, answered [`MemfdRefusal::Ambiguous`],
        // and refused the one boot this module exists to serve.
        //
        // Two mechanisms, closing two different things:
        //   1. `own` — numbers this census itself opened are skipped, so the instrument
        //      cannot observe itself at all;
        //   2. `by_inode` — the census is over **blocks, not descriptors**. A hypervisor
        //      is entitled to hold two descriptors on one `memfd`, and that must not read
        //      as two blocks either. Two *different* `memfd`s of one name have different
        //      inodes and stay ambiguous, which is the fact worth refusing.
        let numbers: Vec<i32> = std::fs::read_dir("/proc/self/fd")
            .map_err(|e| RawError::Syscall {
                call: "read_dir(/proc/self/fd)",
                errno: e.raw_os_error(),
            })?
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()))
            .collect();
        let mut seen: Vec<MemfdCandidate> = Vec::new();
        let mut own: BTreeSet<i32> = BTreeSet::new();
        let mut by_inode: BTreeSet<u64> = BTreeSet::new();
        for number in numbers {
            if own.contains(&number) {
                continue;
            }
            let path = std::path::PathBuf::from(format!("/proc/self/fd/{number}"));
            // ★ READ THE LINK BEFORE OPENING ANYTHING. Two reasons, and the second is the
            // one that matters: (1) it is cheap, and (2) `/proc/self/fd/N` for a character
            // device is a *second open of that device*. A census that opened every
            // descriptor to see what it was would be opening `/dev/nvidiactl` again as a
            // side effect of looking.
            let Ok(link) = std::fs::read_link(&path) else {
                continue;
            };
            if memfd_name(&link).is_none() {
                continue;
            }
            // Re-open. Read-write, because guest RAM is written by both sides; a `memfd`
            // is a real shmem file, so unlike a KVM anonymous inode this succeeds.
            let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
            else {
                continue;
            };
            own.insert(raw_number(&file));
            // ★★★ EVERY PROPERTY BELOW IS RE-DERIVED FROM THE DESCRIPTOR WE NOW OWN.
            // Between the `read_dir` and the `open`, `number` could have been closed and
            // recycled onto something else; nothing can change what *our* descriptor
            // names. So the link is read again — through the new number, not the old one —
            // and the stat is taken on the open file.
            let Ok(link) = std::fs::read_link(format!("/proc/self/fd/{}", raw_number(&file)))
            else {
                continue;
            };
            let Some(name) = memfd_name(&link) else {
                continue;
            };
            let Ok(meta) = file.metadata() else {
                continue;
            };
            let inode = meta.ino();
            if !by_inode.insert(inode) {
                continue;
            }
            seen.push(MemfdCandidate {
                name,
                bytes: meta.len(),
                inode,
                dev: meta.dev(),
                shared_mapped: shared_inodes.contains(&inode),
                listed_as: number,
                file,
            });
        }
        Ok(MemfdCensus { seen })
    }

    /// Everything the census found, for a refusal message.
    #[must_use]
    pub fn seen(&self) -> &[MemfdCandidate] {
        &self.seen
    }

    /// ★★★ Select **the one** shared-mapped `memfd` called `name`.
    ///
    /// # Errors
    /// [`MemfdRefusal::NoSuchMemfd`] when none matches, [`MemfdRefusal::Ambiguous`] when
    /// more than one does.
    pub fn the_only_shared_memfd_named(self, name: &str) -> Result<MemfdCandidate, MemfdRefusal> {
        let mut matched: Vec<MemfdCandidate> = self
            .seen
            .into_iter()
            .filter(|c| c.shared_mapped && c.name == name)
            .collect();
        match matched.len() {
            0 => Err(MemfdRefusal::NoSuchMemfd),
            1 => Ok(matched.remove(0)),
            n => Err(MemfdRefusal::Ambiguous { matched: n }),
        }
    }
}

/// The `memfd` creation name inside a `readlink` result, or `None` if the link is not a
/// `memfd`'s.
///
/// ⊘ `strip_prefix` on the **whole** link, never `contains`: §1.1's trap 3 is a substring
/// match on `memfd` matching a log file whose boot tag happened to be `memfd1`.
fn memfd_name(link: &std::path::Path) -> Option<String> {
    let s = link.to_str()?;
    let rest = s.strip_prefix(MEMFD_LINK_PREFIX)?;
    Some(rest.strip_suffix(DELETED_SUFFIX).unwrap_or(rest).to_owned())
}

/// The descriptor number of a file we own — used only to build the `/proc/self/fd` path
/// we re-read the link through.
fn raw_number(file: &File) -> i32 {
    use std::os::fd::AsRawFd;
    file.as_raw_fd()
}

/// Every inode this process has an `rw-s` mapping of.
///
/// ★ Joined on the **inode**, not on the mapping's pathname. Both are on the line, and the
/// pathname is the tempting one — but two different `memfd`s can carry the same creation
/// name (the census's own tests build exactly that), so a pathname join would report the
/// decoy as mapped. An inode is the object.
///
/// A `/proc/self/maps` that cannot be read yields an empty set, which makes every
/// candidate un-mapped and therefore refused. ⊘ That is the safe direction: it cannot turn
/// a wrong descriptor into a selected one.
fn shared_mapped_inodes() -> BTreeSet<u64> {
    let mut inodes = BTreeSet::new();
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
        return inodes;
    };
    for line in maps.lines() {
        // `<range> <perms> <offset> <dev> <inode> <path…>`
        let mut fields = line.split_whitespace();
        let (Some(_range), Some(perms), Some(_offset), Some(_dev), Some(inode)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        // `rw-s`: writable AND shared. A read-only or private mapping of a `memfd` is not
        // the shape a machine's RAM has, and admitting it would widen the discriminator
        // that separates guest RAM from every other `memfd` in the process.
        if !(perms.as_bytes().get(1) == Some(&b'w') && perms.as_bytes().get(3) == Some(&b's')) {
            continue;
        }
        // Inode 0 is how `/proc/self/maps` spells "no file behind this mapping" — every
        // anonymous mapping in the process carries it. Admitting it would make one shared
        // anonymous region mark every `memfd` in the census as mapped.
        match inode.parse::<u64>() {
            Ok(0) | Err(_) => continue,
            Ok(inode) => {
                inodes.insert(inode);
            }
        }
    }
    inodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backing, CachePolicy, HostPageSize, HostProt, MappedRegion, SharedRam};
    use std::ffi::CString;

    fn page() -> u64 {
        HostPageSize::query().bytes()
    }

    /// ★★ Every test names its own `memfd`.
    ///
    /// ⊘ Not hygiene: this crate's test binary runs its threads **in parallel**, and
    /// several other tests create `SharedRam` (which is a `memfd` named
    /// `kayfabe-guest-ram`) and map it shared. A census test keyed on that name would see
    /// another test's block and report [`MemfdRefusal::Ambiguous`] — intermittently. That
    /// is the same instrument failure `guest_ram_crossing.md` §4.3 corrected: a
    /// process-wide count read every other test's descriptors.
    fn named(tag: &str) -> CString {
        CString::new(format!("kf-census-{tag}")).expect("a name with no NUL")
    }

    fn block(tag: &str, pages: u64) -> SharedRam {
        SharedRam::create_named(&named(tag), pages * page()).expect("a shared block")
    }

    fn map_shared(ram: &SharedRam, len: u64) -> MappedRegion {
        MappedRegion::map(
            Backing::SharedFile {
                fd: ram.as_backing_fd(),
                offset: 0,
            },
            len,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            HostPageSize::query(),
        )
        .expect("a shared mapping")
    }

    /// The base case: a shared-mapped `memfd` is found by its name, and the descriptor
    /// handed back names the **same object** the census was taken over.
    ///
    /// ★ The inode is the assertion rather than "a file came back": a probe that opened
    /// the wrong `/proc/self/fd` entry would also return *a* descriptor.
    #[test]
    fn a_shared_mapped_memfd_is_found_by_name_and_is_the_same_object() {
        let tag = "base";
        let ram = block(tag, 2);
        let _m = map_shared(&ram, 2 * page());
        let census = MemfdCensus::take_of_this_process().expect("a census");
        assert_eq!(
            census
                .seen()
                .iter()
                .filter(|c| c.name() == format!("kf-census-{tag}"))
                .count(),
            1,
            "★★★ ONE block must be reported ONCE. The census re-opens what it finds, and \
             `/proc/self/fd` is enumerated lazily — so a loop that opened while iterating \
             counted its OWN re-open as a second `memfd` of the same name, same inode and \
             same shared-mapped state, i.e. as `Ambiguous`. That refuses the one boot this \
             exists to serve."
        );
        let found = census
            .the_only_shared_memfd_named(&format!("kf-census-{tag}"))
            .expect("the block is there");
        assert_eq!(found.bytes(), 2 * page(), "the kernel's own length");
        assert!(found.shared_mapped());
        let (_fd, bytes) = found.into_descriptor();
        assert_eq!(bytes, 2 * page());
    }

    /// ★★★★ **The bite that a name-only probe fails, and that "take the first" fails
    /// too.**
    ///
    /// Two `memfd`s carry the *same creation name*; only the second is mapped shared. The
    /// decoy is created **first**, so the kernel gives it the **lower** descriptor number
    /// — which is precisely what "the first `memfd`" and any hardcoded number would
    /// select. §4.5 measured that number moving between two physical boxes, so this test
    /// asserts the selection is not merely correct but correct *for the property*: the
    /// chosen candidate is the mapped one, and its number is the higher of the two.
    #[test]
    fn the_unmapped_decoy_at_a_lower_number_is_not_selected() {
        let tag = "decoy";
        let decoy = block(tag, 1);
        let real = block(tag, 4);
        let _m = map_shared(&real, 4 * page());

        let census = MemfdCensus::take_of_this_process().expect("a census");
        let both: Vec<i32> = census
            .seen()
            .iter()
            .filter(|c| c.name() == format!("kf-census-{tag}"))
            .map(MemfdCandidate::listed_as)
            .collect();
        assert_eq!(
            both.len(),
            2,
            "the census sees BOTH, decoy included: {both:?}"
        );
        let lowest = *both.iter().min().expect("two entries");

        let found = census
            .the_only_shared_memfd_named(&format!("kf-census-{tag}"))
            .expect("exactly one is mapped shared");
        assert_eq!(
            found.bytes(),
            4 * page(),
            "★ the MAPPED block was selected, not the decoy"
        );
        assert_ne!(
            found.listed_as(),
            lowest,
            "★★ and it is NOT the lowest-numbered match — a probe keyed on position \
             would have taken the decoy"
        );
        drop(decoy);
    }

    /// ★★★ **One block held at TWO descriptor numbers is ONE candidate.**
    ///
    /// A hypervisor is entitled to hold more than one descriptor on its own RAM — nothing
    /// forbids it and nothing announces it. A census over *descriptors* would report that
    /// as two, and the selector would answer [`MemfdRefusal::Ambiguous`] and refuse a
    /// perfectly ordinary boot. So the census is over **blocks**, joined on the inode.
    ///
    /// ⊘ The bite is the test below it: two *different* blocks of one name still refuse.
    /// Without that, this test would also pass against a census that had simply stopped
    /// distinguishing blocks at all.
    #[test]
    fn one_block_at_two_descriptor_numbers_is_one_candidate() {
        let tag = "twofd";
        let ram = block(tag, 1);
        let second = ram
            .dup_for_export()
            .expect("a second descriptor on one block");
        let _m = map_shared(&ram, page());
        let census = MemfdCensus::take_of_this_process().expect("a census");
        assert_eq!(
            census
                .seen()
                .iter()
                .filter(|c| c.name() == format!("kf-census-{tag}"))
                .count(),
            1,
            "★ two descriptors, one block, one candidate"
        );
        census
            .the_only_shared_memfd_named(&format!("kf-census-{tag}"))
            .expect("and it selects");
        drop(second);
    }

    /// ⊘ Two shared-mapped blocks of one name is **ambiguous**, not a coin flip.
    #[test]
    fn two_shared_mapped_blocks_of_one_name_are_refused_by_name() {
        let tag = "ambig";
        let a = block(tag, 1);
        let b = block(tag, 1);
        let _ma = map_shared(&a, page());
        let _mb = map_shared(&b, page());
        let census = MemfdCensus::take_of_this_process().expect("a census");
        assert_eq!(
            census
                .the_only_shared_memfd_named(&format!("kf-census-{tag}"))
                .err(),
            Some(MemfdRefusal::Ambiguous { matched: 2 })
        );
    }

    /// A name nothing carries is [`MemfdRefusal::NoSuchMemfd`] — the arm an operator who
    /// forgot the launch flag must see.
    #[test]
    fn a_name_nothing_carries_is_no_such_memfd() {
        let census = MemfdCensus::take_of_this_process().expect("a census");
        assert_eq!(
            census
                .the_only_shared_memfd_named("kf-census-nothing-is-called-this")
                .err(),
            Some(MemfdRefusal::NoSuchMemfd)
        );
    }

    /// ★★★ **The ordering test — the one a copying probe cannot pass.**
    ///
    /// The census is taken **first**; the sentinel is written through the original block
    /// **afterwards**; and it is read back through the descriptor the census handed over.
    /// A probe that copied the range at census time — or that opened some other file of
    /// the right size — would return the pre-write bytes. Only a descriptor onto the same
    /// shmem object answers.
    ///
    /// ⊘ Written the obvious way (write, census, read) this would pass against a copy.
    #[test]
    fn a_write_made_after_the_census_is_visible_through_the_censused_descriptor() {
        let tag = "order";
        let ram = block(tag, 1);
        let mine = map_shared(&ram, page());

        let census = MemfdCensus::take_of_this_process().expect("a census");
        let found = census
            .the_only_shared_memfd_named(&format!("kf-census-{tag}"))
            .expect("the block is there");
        let (fd, bytes) = found.into_descriptor();

        // The write happens AFTER the census and after the descriptor was handed over.
        mine.write_from(crate::HostOffset::ZERO, &[0xA5u8; 8])
            .expect("writing the sentinel");

        let theirs = MappedRegion::map(
            Backing::SharedFile {
                fd: std::os::fd::AsFd::as_fd(&fd),
                offset: 0,
            },
            bytes,
            HostProt::ReadOnly,
            CachePolicy::WriteBack,
            HostPageSize::query(),
        )
        .expect("mapping the descriptor the census returned");
        let mut got = [0u8; 8];
        theirs
            .read_into(crate::HostOffset::ZERO, &mut got)
            .expect("reading it back");
        assert_eq!(
            got, [0xA5u8; 8],
            "★ the census's descriptor must see a write made after it was taken"
        );
    }

    /// The link parser: a prefix of the whole link, and never a substring.
    #[test]
    fn only_a_leading_memfd_prefix_names_a_memfd() {
        assert_eq!(
            memfd_name(std::path::Path::new(
                "/memfd:memory-backend-memfd (deleted)"
            ))
            .as_deref(),
            Some("memory-backend-memfd")
        );
        assert_eq!(
            memfd_name(std::path::Path::new("/memfd:displaysurface")).as_deref(),
            Some("displaysurface")
        );
        assert_eq!(
            memfd_name(std::path::Path::new("/workspace/bench/run_memfd1_qemu.log")),
            None,
            "★ §1.1 trap 3: a boot tagged `memfd1` makes a LOG FILE match a substring search"
        );
        assert_eq!(
            memfd_name(std::path::Path::new("/dev/shm/memfd:x")),
            None,
            "the prefix is anchored"
        );
    }
}
