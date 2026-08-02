//! ★★★ **Decision (b): what crosses is MEMORY, and the VMM cannot `ioctl` it.**
//!
//! `isolate_vmm_fd_crossing.md` §12. `fd_crossing.rs` proved the transport; every run there
//! was socketpair-level and in-process, and §11 item 3 carried the bound in as many words:
//! *"no verb uses the crossing yet, so it has never run against a real isolate."* This file
//! is that bound being paid. Every test below drives a **real spawned child process** over
//! a **real socket**, and the descriptor that comes back is one the child really minted.
//!
//! ## ★★★ The property, and why asserting the mapping works would not establish it
//!
//! > **The VMM cannot issue an RM ioctl on what it received.**
//!
//! A test that exports a backing, maps it, and reads bytes back proves the *mechanism* and
//! says nothing about the property: the identical test passes against a design that hands
//! the VMM `/dev/nvidia0`, because a device descriptor `mmap`s perfectly well too. So the
//! property is asserted the only way it can be — by **issuing the escapes** and watching
//! the kernel refuse them ([`the_vmm_cannot_issue_an_rm_ioctl_on_what_it_received`]).
//!
//! ⚠ And an `ENOTTY` assertion is vacuous on its own: a broken ioctl wrapper that never
//! reached a syscall would produce it too. So the **same** wrapper, on the **same
//! descriptor**, is first made to *succeed* with a request the kernel does serve for a
//! file. That control is not decoration — it is what makes the refusals mean something
//! about **RM** rather than about our code. ★ It also had to be corrected once: see
//! [`FIONREAD`].
//!
//! ## What runs here needs no GPU
//!
//! `RmMode::Loopback` spawns the genuine embedded isolate; its export arm calls the
//! **same** `mint_fabricated` the production backend does, because minting a `memfd` is not
//! an RM semantic and there is nothing for a fixture to model. The device arm refuses,
//! identically, in both.

use kayfabe_abi::bringup::{NV_ESC_CHECK_VERSION_STR, NV_ESC_RM_ALLOC_MEMORY, NV_IOCTL_MAGIC};
use kayfabe_abi::generated::nvos::{NV_ESC_RM_ALLOC, NV_ESC_RM_CONTROL, NV_ESC_RM_FREE};
use kayfabe_arch::ids::GpuId;
use kayfabe_isolate::{
    ExportRequest, ExportSource, HostHandle, Isolate as _, IsolateId, RmError, Worker,
};
use kayfabe_isolate_host::export::ExportRegistry;
use kayfabe_isolate_host::{HostIsolate, HostIsolateFactory, ParkVerb, RmMode};
use kayfabe_linux_raw::{
    Backing, CharDevice, DescriptorKind, HostOffset, HostPageSize, HostProt, MappedRegion,
    RawError, ioctl,
};
use kayfabe_vmm::Prot;
use std::collections::BTreeMap;
use std::os::fd::{AsFd, OwnedFd};

/// `ENOTTY` — *"inappropriate ioctl for device"*. What Linux answers when the object behind
/// a descriptor has no handler for the request, which for a `memfd` is every request.
const ENOTTY: i32 = 25;

/// `FIONREAD` on Linux — `_IOR('T', 0x1B, int)`.
///
/// ★★★ **The positive control, and it was found by being WRONG first.** This started as
/// *"a socket serves it, a `memfd` does not"* — and the run said `Ok(0)` on the `memfd`,
/// because Linux answers `FIONREAD` for an ordinary file generically, before any
/// filesystem is consulted. Suspecting the instrument turned a broken control into a
/// **better** one: the same wrapper, on the **same received object**, serves this request
/// and refuses every RM escape. Holding the object fixed and varying only the request is
/// what isolates *"this object has no RM handler"* from *"our ioctl call does not work"*.
const FIONREAD: u64 = 0x541B;

/// ★ The descriptor table is **process-wide** and libtest runs tests as threads of one
/// process, so a test that measures `/proc/self/fd` measures its neighbours too. This is
/// `fd_crossing.rs`'s finding, inherited verbatim rather than rediscovered: without this
/// lock, three tests here failed on their first run reading each other's descriptors.
static FD_TABLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Serialise **every** test in this file.
///
/// ★ Every one, not only the ones that snapshot `/proc/self/fd`. A test that merely spawns
/// an isolate mutates the table a neighbour is measuring — and worse, the embedded image is
/// published into a `memfd` **once per process**, so whichever test spawns first adds a
/// descriptor that appears from nowhere in another test's "nothing else changed" set. It
/// did, and this comment is what that red run bought.
fn serialized() -> std::sync::MutexGuard<'static, ()> {
    FD_TABLE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Every NVIDIA RM frontend escape this test aims at a received backing, with the argument
/// size the driver's request number encodes.
///
/// ★ Five, not one. The property is *"no RM ioctl"*, and a single escape would leave the
/// statement quantified over a list of one — the shape `gates_quantified_over_a_list`
/// records three instances of. They span both the `nv_escape.h` and `nvos` numbering
/// families, which is the axis a typo would hide in.
fn rm_escapes() -> Vec<(&'static str, u8, usize)> {
    vec![
        ("NV_ESC_RM_ALLOC", NV_ESC_RM_ALLOC as u8, 64),
        ("NV_ESC_RM_CONTROL", NV_ESC_RM_CONTROL as u8, 64),
        ("NV_ESC_RM_FREE", NV_ESC_RM_FREE as u8, 32),
        ("NV_ESC_RM_ALLOC_MEMORY", NV_ESC_RM_ALLOC_MEMORY, 64),
        ("NV_ESC_CHECK_VERSION_STR", NV_ESC_CHECK_VERSION_STR, 264),
    ]
}

/// Spawn a real isolate. `Loopback` because this file is about the crossing, not the driver.
fn isolate(id: IsolateId) -> HostIsolate {
    let factory = HostIsolateFactory::new(RmMode::Loopback).with_park(ParkVerb::Nothing);
    let iso = factory.spawn_host(id);
    assert!(
        iso.spawn_error().is_none(),
        "the isolate did not start: {:?}",
        iso.spawn_error()
    );
    iso
}

/// Check a worker out of `iso`, run `f` against it, and check it back in.
fn with_worker<T>(iso: &mut HostIsolate, f: impl FnOnce(&mut Worker) -> T) -> T {
    let mut w = iso.checkout().expect("a worker");
    let out = f(&mut w);
    iso.checkin(w);
    out
}

/// ★★★ Every descriptor this process holds, **with what each one points at**.
///
/// Numbers alone are not enough, and this is `fd_crossing.rs`'s second instrument defect
/// inherited rather than rediscovered: `read_dir` takes a descriptor of its own, and the
/// kernel hands out the **lowest free** number — which, immediately after a refusal closed
/// fd 3, is fd 3. A number-only snapshot then contains a 3 again and reads as *"the refused
/// descriptor is still open"* when the refusal worked perfectly. It cost this file one
/// red run before the lesson was applied. Pairing each number with its link target makes
/// the reuse visible instead of confusing.
fn open_fd_targets() -> BTreeMap<u32, String> {
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc must be mounted for this suite")
        .filter_map(|e| {
            let e = e.ok()?;
            let n = e.file_name().to_str()?.parse::<u32>().ok()?;
            let t = std::fs::read_link(e.path()).ok()?;
            let t = t.to_string_lossy().into_owned();
            // ⊘ The observer's own handle on `/proc/<pid>/fd` — excluded, because it is a
            // different number on every call and would make every comparison fail.
            (!(t.starts_with("/proc/") && t.ends_with("/fd"))).then_some((n, t))
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────────────
// ★★★ THE BITE
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **The property of decision (b), measured rather than argued** —
/// run: task `#133`, 2026-07-31, 38-core build box, base `90eb50f`, 7/7 green with 7 bites
/// each watched to fire (`isolate_vmm_fd_crossing.md` §12.6).
///
/// Four statements, in the order that makes the fourth mean something:
///
/// 1. the export succeeds against a real child;
/// 2. what crossed is, **according to the kernel**, a regular file — not the character
///    device an `RmBackend` could otherwise have answered with;
/// 3. ★ the **positive control**: the very same `CharDevice::ioctl` wrapper, on the very
///    same received descriptor, *serves* `FIONREAD`. So the wrapper reaches the kernel and
///    an `ENOTTY` below is this object refusing **that request**, not our code failing to
///    make a syscall;
/// 4. ★★★ every NVIDIA RM escape issued on that same descriptor is `ENOTTY`.
///
/// ⚠ **The bound, stated rather than left to inference:** this shows the received object
/// has no handler for those request numbers. It does not show that a real `/dev/nvidia*`
/// *would* have served them — no GPU is present, and asserting that needs the hardware
/// box. What it does establish is the property decision (b) is about: the thing the VMM
/// received is not an RM surface.
///
/// ## What each removal does (bites paid, each watched to fire)
///
/// | bite | what goes red |
/// |---|---|
/// | the child lends `/dev/null` instead of the backing it minted | (2) — the kind refusal fires and the export never completes |
/// | …**and** `CrossedFd::adopt`'s kind check removed as well | (2b) — `/proc` still says it is a device, which is why (2b) is asked separately |
/// | the reply read with `max_fds = 0` | (1) — the kernel drops the descriptor and the frame is refused |
/// | `serve_one` routed through `execute`, so no descriptor is attached | (1) |
/// | `HostRmBackend::export_backing` answers the device arm with a `memfd` instead of refusing | `the_device_class_is_refused_by_name_and_nothing_crosses` |
/// | the positive control removed | nothing — which is exactly why it is here |
///
/// ⊘ **One bite is NOT payable on this box and it is the sharpest one:** making (4) fire
/// needs a descriptor that genuinely *serves* an RM escape, i.e. a real `/dev/nvidia*`. No
/// substitute char device answers those request numbers, so (4) is a true statement that
/// has never been seen to fail. It is owed on the hardware box, and until it is paid, (2)
/// and (2b) are what carry the property.
#[test]
fn the_vmm_cannot_issue_an_rm_ioctl_on_what_it_received() {
    let _fd_table = serialized();
    let id = IsolateId::new(41, GpuId(0));
    let mut iso = isolate(id);

    // (1) The export, through the port, over a real socket, into a real child.
    let backing = with_worker(&mut iso, |w| {
        w.export_backing(ExportRequest {
            source: ExportSource::Fabricated,
            len: 2 * 4096,
            prot: Prot::ReadWrite,
        })
    })
    .expect("a fabricated backing must be exportable");
    assert_eq!(backing.len, 2 * 4096);
    assert_eq!(backing.offset, 0);
    assert_eq!(backing.prot, Prot::ReadWrite);

    // (2) What the KERNEL says it is. Not what the child claimed; `CrossedFd::adopt` ran
    // `fstat` before this descriptor was reachable at all.
    assert_eq!(
        iso.exports().kind(backing.token),
        Some(DescriptorKind::RegularFile),
        "★ what crossed must be MEMORY. A `CharDevice` here is the (a) design, and it is \
         the thing decision (b) exists to prevent"
    );

    let received = CharDevice::adopt(iso.exports().dup(backing.token).expect("dup"));

    // (2b) ★★ The same question asked of the KERNEL DIRECTLY, not through the wrapper that
    // records it. `CrossedFd::kind` hands back the promise it CHECKED, so with the check
    // removed it would go on saying `RegularFile` about a device node. The
    // link target cannot be wrong: it is what `/proc` says the object is.
    let raw = std::os::fd::AsRawFd::as_raw_fd(&received.as_fd());
    let target = std::fs::read_link(format!("/proc/self/fd/{raw}"))
        .expect("readlink")
        .to_string_lossy()
        .into_owned();
    assert!(
        target.starts_with("/memfd:"),
        "★★ the received object must BE a memfd according to /proc, independently of what \
         `adopt` recorded; it is {target}"
    );

    // (3) ★ THE POSITIVE CONTROL — on the object under test, not beside it. If this ever
    // stops being served, every ENOTTY below becomes unfalsifiable and this test must be
    // treated as dead rather than green.
    let mut queued = [0u8; 4];
    let served = received.ioctl(FIONREAD, &mut queued, &mut []);
    assert!(
        served.is_ok(),
        "★ NON-VACUITY: the ioctl wrapper must reach the kernel and be SERVED on THIS very \
         descriptor — otherwise every ENOTTY below proves nothing about RM. It said \
         {served:?}"
    );

    // (4) ★★★ THE ASSERTION. Same object, same wrapper, only the request changes.

    for (name, nr, size) in rm_escapes() {
        let request = ioctl::readwrite(NV_IOCTL_MAGIC, nr, size).expect("a valid request number");
        let mut arg = vec![0u8; size];
        assert_eq!(
            received.ioctl(request, &mut arg, &mut []),
            Err(RawError::Syscall {
                call: "ioctl",
                errno: Some(ENOTTY),
            }),
            "★★★ {name} was not refused on the backing the isolate handed up. A descriptor \
             the VMM can drive RM through is F14 reachable, and RM derives privilege from \
             the CALLER at ioctl time"
        );
        assert!(
            arg.iter().all(|&b| b == 0),
            "{name}: a refused escape must not have written into the argument buffer"
        );
    }
}

/// ★★ **What crossed is the isolate's object, not one this process made** — the
/// anti-vacuity companion to the test above.
///
/// Every other test in this file would pass against an implementation that quietly minted
/// its own `memfd` in the parent and sent nothing. Two independent facts rule that out:
///
/// 1. **Provenance.** `/proc/self/fd/<n>` names a `memfd` created under `SharedRam`'s
///    name. Nothing on the parent's side of this test calls `memfd_create` at all — the
///    only one it *does* create anywhere is the isolate image, under a different name —
///    so an object with this name in this table arrived over the socket.
/// 2. **It is real shared memory.** A write through one duplicate is visible through an
///    independently obtained second one. A fabricated per-call descriptor would fail this
///    the moment the two views were compared.
///
/// ⊘ What it does **not** prove, stated rather than implied: that the *isolate* can see the
/// write. Nothing in the port reads the isolate's own view of a backing, so that half is
/// unmeasured here and is `fb_read`'s question, not this one.
#[test]
fn what_crossed_is_the_isolates_own_object_and_it_is_real_shared_memory() {
    let _fd_table = serialized();
    let id = IsolateId::new(42, GpuId(0));
    let mut iso = isolate(id);
    let backing = with_worker(&mut iso, |w| {
        w.export_backing(ExportRequest {
            source: ExportSource::Fabricated,
            len: 4096,
            prot: Prot::ReadWrite,
        })
    })
    .expect("exported");

    // ---- (1) Provenance.
    let first = iso.exports().dup(backing.token).expect("dup");
    let raw = std::os::fd::AsRawFd::as_raw_fd(&first.as_fd());
    let target = std::fs::read_link(format!("/proc/self/fd/{raw}")).expect("readlink");
    let target = target.to_string_lossy().into_owned();
    assert!(
        target.contains("memfd:kayfabe-guest-ram"),
        "★ the descriptor must name the memfd the ISOLATE minted; it names {target}"
    );

    // ---- (2) It is one shared object, reached twice.
    let second = iso.exports().dup(backing.token).expect("second dup");
    let page = HostPageSize::query();
    let writer = MappedRegion::map(
        Backing::SharedFile {
            fd: first.as_fd(),
            offset: 0,
        },
        4096,
        HostProt::ReadWrite,
        kayfabe_linux_raw::CachePolicy::WriteBack,
        page,
    )
    .expect("map the received backing");
    let reader = MappedRegion::map(
        Backing::SharedFile {
            fd: second.as_fd(),
            offset: 0,
        },
        4096,
        HostProt::ReadOnly,
        kayfabe_linux_raw::CachePolicy::WriteBack,
        page,
    )
    .expect("map it again, independently");

    // ★ A pattern chosen so a false pass is impossible: not zero (the initial content of a
    // fresh memfd) and not a repeat of a single byte.
    let pattern: Vec<u8> = (0..64u8)
        .map(|i| i.wrapping_mul(37).wrapping_add(11))
        .collect();
    writer
        .write_from(HostOffset::new(128), &pattern)
        .expect("write through the received descriptor");
    let mut back = vec![0u8; pattern.len()];
    reader
        .read_into(HostOffset::new(128), &mut back)
        .expect("read through the OTHER descriptor");
    assert_eq!(
        back, pattern,
        "★ the two duplicates must name ONE object; if they do not, nothing crossed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The named boundary
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **The incomplete half of (b), as a refusal with a name** — and nothing crosses.
///
/// A host GPU page cannot be handed to the VMM as memory, for three cited reasons
/// (`RmError::NotExportableAsMemory`). This asserts the **exact** variant — `is_err()`
/// would pass for a `BadHandle`, a `NoMemory`, or a wedged channel, and those are the
/// answers that would appear if the verb were broken rather than bounded.
///
/// It also asserts the *silence*: no descriptor arrives, and the registry does not grow.
/// A refusal that still handed something over would be the worst of both.
#[test]
fn the_device_class_is_refused_by_name_and_nothing_crosses() {
    let _fd_table = serialized();
    let id = IsolateId::new(43, GpuId(0));
    let mut iso = isolate(id);

    // A real object in this isolate's namespace, so the refusal cannot be a `BadHandle`
    // wearing another error's clothes.
    let memory = with_worker(&mut iso, |w| {
        w.execute(&kayfabe_isolate::VerbPlan::Publish {
            host_vas: None,
            len: 4096,
            at: kayfabe_arch::ids::GpuVa(0x2_0000_0000),
        })
    })
    .expect("a publish must succeed against the fixture");
    let kayfabe_isolate::VerbReply::Published { memory, .. } = memory else {
        panic!("expected a Published reply");
    };

    let before_fds = open_fd_targets();
    let before_len = iso.exports().len();

    let refused = with_worker(&mut iso, |w| {
        w.export_backing(ExportRequest {
            source: ExportSource::HostDeviceMemory { memory },
            len: 4096,
            prot: Prot::ReadWrite,
        })
    });

    assert_eq!(
        refused,
        Err(RmError::NotExportableAsMemory { memory }),
        "★★★ the device class must refuse BY NAME. Any other error and a caller cannot \
         tell 'the bytes are on the card' from 'the host failed'"
    );
    assert_eq!(
        iso.exports().len(),
        before_len,
        "a refusal must not put a backing in the registry"
    );
    assert_eq!(
        open_fd_targets(),
        before_fds,
        "★ a refused export must not leave a descriptor in this process's table — that \
         would be the crossing happening anyway, silently"
    );
}

/// ★★ A handle from **another** isolate's namespace is refused before the verb runs.
///
/// The gate is in `Worker::export_backing` and it is deliberately not redundant with the
/// refusal above: today every device-class request refuses anyway, so this would pass
/// vacuously — except that it asserts the **exact** variant, and `ForeignHandle` can only
/// come from the gate. A backend that ever learns to serve a device-class export inherits
/// a gate that is already in place and already tested.
#[test]
fn an_export_naming_another_isolates_handle_is_refused_before_the_verb_runs() {
    let _fd_table = serialized();
    let mine = IsolateId::new(44, GpuId(0));
    let theirs = IsolateId::new(45, GpuId(0));
    let mut iso = isolate(mine);
    let foreign = HostHandle::new(theirs, 0x07);

    let refused = with_worker(&mut iso, |w| {
        w.export_backing(ExportRequest {
            source: ExportSource::HostDeviceMemory { memory: foreign },
            len: 4096,
            prot: Prot::ReadWrite,
        })
    });
    assert_eq!(
        refused,
        Err(RmError::ForeignHandle {
            handle: foreign,
            worker_isolate: mine,
        }),
        "★ a cross-namespace handle must be OUR gate's refusal, not the child's answer"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The registry's own refusals — unit-level, so the exact `RawError` is assertable
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **A child that answers with a character device is refused, by name, and the
/// descriptor is CLOSED.**
///
/// This is the enforcement point that does not trust the child at all. The backend's own
/// refusal of the device class is *our* code inside the child; a compromised isolate is
/// inside the threat model (`l1_os_shell.md` §11), so the parent checks what arrived
/// against the kernel independently.
///
/// Asserted at the registry rather than through a spawned child because the exact
/// `RawError` variant is only visible here — over the wire it becomes `RmError::Wedged`,
/// by `ProxyRmBackend::call`'s stated doctrine, and a test asserting `Wedged` could not
/// tell a kind refusal from a dead peer.
#[test]
fn a_backing_that_is_a_character_device_is_refused_and_closed() {
    let _fd_table = serialized();
    let registry = ExportRegistry::new();
    let id = IsolateId::new(46, GpuId(0));
    let node = std::fs::File::open("/dev/null").expect("/dev/null must exist");
    let fd = OwnedFd::from(node);
    let raw = std::os::fd::AsRawFd::as_raw_fd(&fd.as_fd()) as u32;

    let before = open_fd_targets();
    assert_eq!(
        before.get(&raw).map(String::as_str),
        Some("/dev/null"),
        "the probe descriptor must be open and must be the device node, or this proves \
         nothing"
    );

    assert_eq!(
        registry.adopt(fd, id),
        Err(RawError::DescriptorKindRefused {
            expected: DescriptorKind::RegularFile,
            actual: DescriptorKind::CharDevice,
        }),
        "★★★ a character device must be refused BY NAME before it is reachable"
    );
    assert!(
        registry.is_empty(),
        "a refused descriptor must not land in the registry"
    );
    let after = open_fd_targets();
    assert_ne!(
        after.get(&raw).map(String::as_str),
        Some("/dev/null"),
        "★ the refused descriptor is STILL OPEN — `adopt` takes it by value precisely so \
         the refusal path closes it. (Asserted on the TARGET, not the number: the number \
         is reused by the very `read_dir` doing the looking.)"
    );
    let mut expected = before;
    expected.remove(&raw);
    let after_without_reuse: BTreeMap<u32, String> =
        after.into_iter().filter(|(n, _)| *n != raw).collect();
    assert_eq!(
        after_without_reuse, expected,
        "the refusal closed the right descriptor and nothing else"
    );
}

/// A token this registry never minted is [`RawError::UnknownExport`], not a panic and not
/// a silent `None` that a caller could read as an empty backing.
#[test]
fn a_token_this_registry_never_minted_is_a_named_refusal() {
    let _fd_table = serialized();
    let registry = ExportRegistry::new();
    assert_eq!(
        registry.dup(7).err(),
        Some(RawError::UnknownExport { token: 7 }),
        "an unknown token must name itself"
    );
    assert_eq!(registry.kind(7), None);
    assert_eq!(registry.origin(7), None);
}

/// ★ Two exports get **two** tokens and two distinct objects. A registry that reused a
/// token would have the second install silently replace the first's backing — a mapping of
/// the wrong bytes, which is the failure this whole design ranks worst.
#[test]
fn two_exports_get_two_tokens_and_two_distinct_backings() {
    let _fd_table = serialized();
    let id = IsolateId::new(47, GpuId(0));
    let mut iso = isolate(id);
    let mut tokens = Vec::new();
    for len in [4096u64, 8192] {
        let b = with_worker(&mut iso, |w| {
            w.export_backing(ExportRequest {
                source: ExportSource::Fabricated,
                len,
                prot: Prot::ReadWrite,
            })
        })
        .expect("exported");
        assert_eq!(b.len, len);
        tokens.push(b.token);
    }
    assert_ne!(tokens[0], tokens[1], "tokens must not be reused");
    assert_eq!(iso.exports().len(), 2);
    assert_eq!(
        iso.exports().origin(tokens[0]),
        Some(kayfabe_isolate_host::FdOrigin::Isolate(id)),
        "★ provenance travels with the backing — it is what `lend_to` checks"
    );

    // ★ Distinct objects, shown by content rather than by identity: a write into the
    // first must not appear in the second. Two `memfd`s that were secretly one would pass
    // every other assertion in this test.
    let page = HostPageSize::query();
    let map = |t: u64, len: u64| {
        let fd = iso.exports().dup(t).expect("dup");
        let m = MappedRegion::map(
            Backing::SharedFile {
                fd: fd.as_fd(),
                offset: 0,
            },
            len,
            HostProt::ReadWrite,
            kayfabe_linux_raw::CachePolicy::WriteBack,
            page,
        )
        .expect("map");
        (fd, m)
    };
    let (_fd0, m0) = map(tokens[0], 4096);
    let (_fd1, m1) = map(tokens[1], 8192);
    m0.write_from(HostOffset::ZERO, &[0xAB; 16]).expect("write");
    let mut other = [0u8; 16];
    m1.read_into(HostOffset::ZERO, &mut other).expect("read");
    assert_eq!(
        other, [0u8; 16],
        "★ the two backings alias — one export overwrote the other's bytes"
    );
}
