//! The isolate ⇄ VMM descriptor crossing, tested against the hazards a descriptor has
//! **because it is an owning resource with a source** — not against the happy path.
//!
//! `isolate_vmm_fd_crossing.md` §8 lists the seven; this file is that list, in order:
//!
//! 1. leak on every error path — every refusal closes what it refused
//! 2. `MSG_CTRUNC` — induced by undersizing the allowance, never ignored
//! 3. descriptor-table exhaustion — the cap, tested *at* the cap
//! 4. `O_CLOEXEC` on receipt — proven through a real `exec`, with a positive control
//! 5. cross-isolate — refused by name, because topology is an assumption and this is a check
//! 6. lifetime — ⊘ **not covered**; see `does_not_observe_isolate_lifetime` for why
//! 7. type validation — the kernel is asked, the peer's claim is not believed
//!
//! ## ★★ The house style these follow
//!
//! **Derived checks, not enumerated lists.** The descriptor-leak assertions compare the
//! *whole* set of open descriptors before and after, so a descriptor leaked at a number
//! nobody thought of is still caught, and a new refusal arm added tomorrow is covered
//! tomorrow with no edit here. The alternative — asserting a count, or naming the
//! descriptors expected to be open — is how `unrealize_gives_back_every_reference_…`
//! passed for months while never looking at a reference.

use kayfabe_arch::ids::GpuId;
use kayfabe_isolate::IsolateId;
use kayfabe_isolate_host::fdcross::{
    CrossedFd, FdFrameError, FdOrigin, read_frame_with_fds, write_frame_with_fds,
};
use kayfabe_linux_raw::{DescriptorKind, MAX_FDS_PER_FRAME, RawError};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;

// =====================================================================================
// Instruments
// =====================================================================================

/// ★★★ The descriptor table is **process-wide**, and libtest runs tests in threads of one
/// process. Every assertion in this file about which descriptors are open is therefore a
/// statement about the whole process, and a sibling test opening a socket in parallel
/// perturbs it.
///
/// This is not a hypothetical: the first run of this file failed four tests
/// (`a_char_device_is_refused_where_a_regular_file_was_promised`,
/// `a_descriptor_of_the_wrong_kind_…`, `a_descriptor_on_a_frame_that_then_fails_…`,
/// `more_descriptors_than_allowed_…`) reporting descriptors that were "still open" and
/// fd-set diffs of `{…14, 16, 19}` against `{…15, 16, 19, 22}` — **none of which were the
/// code's doing**. The instrument was measuring its neighbours.
///
/// So every test here takes this lock. Poisoning is recovered from rather than
/// propagated: one test's panic must report *its own* failure, not turn every later test
/// into a lock error that hides it.
static FD_TABLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    FD_TABLE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Every descriptor this process currently holds — **excluding the one doing the
/// looking**.
///
/// ★ The whole set, not a count: a leak that happens to coincide with a close is a leak
/// this still catches, and a count is exactly the check that would miss it.
///
/// ★★ THE OBSERVER MUST NOT APPEAR IN ITS OWN MEASUREMENT, and this cost a run to learn
/// (task #131, 2026-07-31, this file).
/// `read_dir("/proc/self/fd")` opens a descriptor, and the kernel hands out the **lowest
/// free** number — which, immediately after a refusal closed fd 3, is fd 3. The second
/// snapshot therefore contained a 3 again and the assertion read it as *"the refused
/// descriptor is still open"* when the refusal had worked perfectly. The observer's own
/// handle is identified by what it points at (`/proc/<pid>/fd`) and dropped from the
/// result, so measuring can no longer be mistaken for a leak.
fn open_fds() -> BTreeSet<u32> {
    open_fd_targets()
        .into_iter()
        .filter(|(_, target)| !is_the_observers_own_handle(target))
        .map(|(n, _)| n)
        .collect()
}

/// Does this link target name the `/proc/<pid>/fd` directory itself — i.e. the handle the
/// snapshot is being taken through?
fn is_the_observers_own_handle(target: &str) -> bool {
    target.starts_with("/proc/") && target.ends_with("/fd")
}

/// Every descriptor this process holds, **with what it points at** — for the assertions
/// that must not be fooled by descriptor-number reuse.
fn open_fd_targets() -> BTreeMap<u32, String> {
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc must be mounted for this suite")
        .filter_map(|e| {
            let e = e.ok()?;
            let n = e.file_name().to_str()?.parse::<u32>().ok()?;
            let t = std::fs::read_link(e.path()).ok()?;
            Some((n, t.to_string_lossy().into_owned()))
        })
        .collect()
}

/// Run `f` and assert it left the descriptor table exactly as it found it.
///
/// The `read_dir` inside [`open_fds`] takes a descriptor of its own, which is released
/// before it returns; a leak inside `f` therefore shifts the number that `read_dir` gets
/// on the second call and the two sets differ. Both the leaked descriptor and the shift
/// are visible — which is the point of comparing sets rather than sizes.
fn leaks_nothing<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let before = open_fds();
    let out = f();
    let after = open_fds();
    assert_eq!(
        before, after,
        "{what} changed this process's open-descriptor set; a refused descriptor that \
         stays open is the leak this boundary exists to prevent"
    );
    out
}

/// Run a refusal that **consumes** a descriptor, and assert it closed *exactly* that one.
///
/// ★ Two assertions, and the second is the one that matters: the refused descriptor is
/// gone, **and nothing else changed**. Asserting only the first would pass against an
/// implementation that closed the descriptor and leaked a different one; asserting only a
/// count would pass against one that closed the wrong descriptor entirely.
fn refusal_closes_exactly<T>(what: &str, raw: u32, f: impl FnOnce() -> T) -> T {
    let before = open_fds();
    assert!(
        before.contains(&raw),
        "{what}: fd {raw} must be open before the refusal, or this proves nothing"
    );
    let out = f();
    let after = open_fds();
    assert!(
        !after.contains(&raw),
        "{what}: the refused descriptor {raw} is STILL OPEN — a refused descriptor that \
         stays open is the leak this boundary exists to prevent"
    );
    let mut expected = before;
    expected.remove(&raw);
    assert_eq!(
        expected, after,
        "{what}: exactly the refused descriptor should have closed, and nothing else \
         should have changed"
    );
    out
}

/// A character device — what `/dev/nvidia0` is, and what the GPU-fd direction carries.
fn a_char_device() -> OwnedFd {
    OwnedFd::from(std::fs::File::open("/dev/null").expect("/dev/null"))
}

/// A regular file — what a `memfd` reports as, and what the shareable-backing direction
/// carries. Named distinctively so [`open_fd_targets`] can find it by *target*, which is
/// what makes the `CLOEXEC` proof immune to descriptor-number reuse.
fn a_regular_file(tag: &str) -> (OwnedFd, String) {
    let path = format!(
        "/tmp/kayfabe-fdcross-{}-{}-{tag}.probe",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .expect("probe file");
    std::fs::remove_file(&path).ok();
    (OwnedFd::from(f), path)
}

/// The two ends of the isolate wire.
fn wire() -> (UnixStream, UnixStream) {
    UnixStream::pair().expect("socketpair")
}

fn iso(proc: u32, gpu: u32) -> IsolateId {
    IsolateId::new(proc, GpuId(gpu))
}

// =====================================================================================
// The crossing itself — and the proof that it is a crossing
// =====================================================================================

/// ★★★ The descriptor that arrives is **the same kernel object**, not merely one of the
/// same type.
///
/// This is the anti-vacuity check for the whole file. Every other test here would pass
/// against an implementation that quietly opened its own descriptor and never sent
/// anything; this one writes through the *received* descriptor and reads it back through
/// the *original*, so only a real `SCM_RIGHTS` transfer satisfies it.
#[test]
fn a_received_descriptor_is_the_same_object_as_the_one_that_was_sent() {
    let _fd_table = serialized();
    let (vmm, isolate) = wire();
    let (file, _) = a_regular_file("identity");

    write_frame_with_fds(isolate.as_fd(), b"open-device-reply", &[file.as_fd()])
        .expect("send with one descriptor");

    let mut body = Vec::new();
    let mut fds = Vec::new();
    assert!(read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 1).expect("receive"));
    assert_eq!(body, b"open-device-reply", "the body must survive intact");
    assert_eq!(fds.len(), 1, "exactly one descriptor crossed");

    // Write through the RECEIVED descriptor…
    let mut received = std::fs::File::from(fds.pop().unwrap());
    received.write_all(b"written through the crossing").unwrap();
    received.flush().unwrap();

    // …and read it back through the ORIGINAL. Same object, or nothing.
    let mut original = std::fs::File::from(file);
    original.rewind().unwrap();
    let mut back = String::new();
    original.read_to_string(&mut back).unwrap();
    assert_eq!(
        back, "written through the crossing",
        "a write through the received descriptor must be visible through the original — \
         otherwise no descriptor crossed and every other test in this file is vacuous"
    );
}

/// Both directions, because the C has both and this port had neither:
/// `ISOLATE_RESP_OPEN_DEVICE` (isolate → VMM, a character device) and
/// `ISOLATE_CMD_RECEIVE_FD` (VMM → isolate, a shareable backing).
#[test]
fn both_directions_carry_a_descriptor() {
    let _fd_table = serialized();
    let (vmm, isolate) = wire();

    // isolate → VMM: the GPU device node.
    let dev = a_char_device();
    write_frame_with_fds(isolate.as_fd(), b"resp-open-device", &[dev.as_fd()]).unwrap();
    let (mut body, mut fds) = (Vec::new(), Vec::new());
    assert!(read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 1).unwrap());
    let crossed = CrossedFd::adopt(
        fds.pop().unwrap(),
        FdOrigin::Isolate(iso(7, 0)),
        DescriptorKind::CharDevice,
    )
    .expect("a character device promised, a character device delivered");
    assert_eq!(crossed.kind(), DescriptorKind::CharDevice);

    // VMM → isolate: the shareable backing.
    let (ram, _) = a_regular_file("backing");
    write_frame_with_fds(vmm.as_fd(), b"cmd-receive-fd", &[ram.as_fd()]).unwrap();
    let (mut body2, mut fds2) = (Vec::new(), Vec::new());
    assert!(read_frame_with_fds(isolate.as_fd(), &mut body2, &mut fds2, 1).unwrap());
    assert_eq!(body2, b"cmd-receive-fd");
    let backing = CrossedFd::adopt(
        fds2.pop().unwrap(),
        FdOrigin::Vmm,
        DescriptorKind::RegularFile,
    )
    .expect("a regular file promised, a regular file delivered");
    assert_eq!(backing.kind(), DescriptorKind::RegularFile);
}

/// A frame with no descriptors is an ordinary frame — no empty `SCM_RIGHTS`, no surprise.
#[test]
fn a_frame_without_descriptors_carries_none() {
    let _fd_table = serialized();
    let (vmm, isolate) = wire();
    write_frame_with_fds(isolate.as_fd(), b"plain reply", &[]).unwrap();
    let (mut body, mut fds) = (Vec::new(), Vec::new());
    assert!(read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 0).unwrap());
    assert_eq!(body, b"plain reply");
    assert!(fds.is_empty());
}

// =====================================================================================
// Hazard 7 — type validation, and hazard 1 on its refusal path
// =====================================================================================

/// ★★★ THE BITE. The peer promises a GPU character device and attaches a regular file;
/// the receiver refuses **by name**, and closes what it refused.
///
/// This is the vector the whole check exists for: without it the VMM would `mmap`
/// whatever came back and install the result as a guest memslot.
#[test]
fn a_descriptor_of_the_wrong_kind_is_refused_by_name_and_closed() {
    let _fd_table = serialized();
    let (vmm, isolate) = wire();
    let (impostor, _) = a_regular_file("impostor");

    write_frame_with_fds(
        isolate.as_fd(),
        b"i-opened-the-gpu-for-you",
        &[impostor.as_fd()],
    )
    .unwrap();
    let (mut body, mut fds) = (Vec::new(), Vec::new());
    assert!(read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 1).unwrap());
    let received = fds.pop().unwrap();
    let raw = received.as_raw_fd() as u32;

    let err = refusal_closes_exactly("refusing a descriptor of the wrong kind", raw, || {
        CrossedFd::adopt(
            received,
            FdOrigin::Isolate(iso(3, 0)),
            DescriptorKind::CharDevice,
        )
        .expect_err("a regular file is not a character device")
    });

    assert_eq!(
        err,
        RawError::DescriptorKindRefused {
            expected: DescriptorKind::CharDevice,
            actual: DescriptorKind::RegularFile,
        },
        "the refusal must name both what was promised and what arrived"
    );
}

/// The mirror: a character device offered where a shareable backing was promised. Same
/// refusal, opposite direction — so the check is not one-sided.
#[test]
fn a_char_device_is_refused_where_a_regular_file_was_promised() {
    let _fd_table = serialized();
    let dev = a_char_device();
    let raw = dev.as_raw_fd() as u32;
    let err = refusal_closes_exactly("refusing a char device promised as a backing", raw, || {
        CrossedFd::adopt(dev, FdOrigin::Vmm, DescriptorKind::RegularFile).expect_err("refused")
    });
    assert_eq!(
        err,
        RawError::DescriptorKindRefused {
            expected: DescriptorKind::RegularFile,
            actual: DescriptorKind::CharDevice,
        }
    );
}

/// A socket is neither, and the refusal says so rather than falling into a default arm.
/// This is the shape a *compromised* peer reaches for: its own end of a pipe or socket.
#[test]
fn a_socket_is_refused_as_neither_kind() {
    let _fd_table = serialized();
    let (a, _b) = wire();
    let sock: OwnedFd = OwnedFd::from(a);
    let err = CrossedFd::adopt(sock, FdOrigin::Vmm, DescriptorKind::CharDevice)
        .expect_err("a socket is not a character device");
    match err {
        RawError::DescriptorKindRefused {
            expected: DescriptorKind::CharDevice,
            actual: DescriptorKind::Other { file_type },
        } => assert_ne!(file_type, 0, "the refusal carries the real S_IFMT bits"),
        other => panic!("expected a kind refusal naming S_IFMT, got {other:?}"),
    }
}

// =====================================================================================
// Hazards 2 & 3 — MSG_CTRUNC and the cap
// =====================================================================================

/// ★★ `MSG_CTRUNC`, induced by undersizing the allowance: the sender hands over two
/// descriptors, the receiver permits one. The kernel closes the excess **silently**, so a
/// receiver that ignored the flag would believe it got everything while the sender
/// believed it handed over two.
#[test]
fn more_descriptors_than_allowed_is_refused_and_leaks_nothing() {
    let _fd_table = serialized();
    let (vmm, isolate) = wire();
    let (one, _) = a_regular_file("ctrunc-a");
    let (two, _) = a_regular_file("ctrunc-b");

    write_frame_with_fds(
        isolate.as_fd(),
        b"two-attached",
        &[one.as_fd(), two.as_fd()],
    )
    .unwrap();

    let err = leaks_nothing("refusing an over-allowance frame", || {
        let (mut body, mut fds) = (Vec::new(), Vec::new());
        let e = read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 1)
            .expect_err("two descriptors arrived where one was allowed");
        assert!(
            fds.is_empty(),
            "a refused frame hands back no descriptors at all — not the prefix that fitted"
        );
        e
    });

    assert_eq!(
        err,
        FdFrameError::Os(RawError::TooManyDescriptors { limit: 1 }),
        "the refusal must name the allowance that was exceeded"
    );
}

/// The `max_fds == 0` case — a message the protocol says carries no descriptors, and a
/// peer that attaches one anyway.
///
/// ★ This is the port of the C's R2-M1 sweep (`C: src/qemu/nvkvm_isolate.c:441-462`,
/// *"a compromised stub could attach a fd to ANY other response type"*). The C closes
/// them in a loop a later `case` can forget to run; here the allowance is zero, so the
/// kernel never hands them over and the frame is refused outright.
#[test]
fn a_descriptor_on_a_message_that_may_not_carry_one_is_refused() {
    let _fd_table = serialized();
    let (vmm, isolate) = wire();
    let (stray, _) = a_regular_file("stray");
    write_frame_with_fds(isolate.as_fd(), b"generic-ok", &[stray.as_fd()]).unwrap();

    let err = leaks_nothing(
        "refusing a stray descriptor on a no-descriptor message",
        || {
            let (mut body, mut fds) = (Vec::new(), Vec::new());
            let e = read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 0).expect_err("refused");
            assert!(fds.is_empty());
            e
        },
    );
    assert_eq!(
        err,
        FdFrameError::Os(RawError::TooManyDescriptors { limit: 0 })
    );
}

/// ★ Tested **at** the cap, both sides of it: exactly [`MAX_FDS_PER_FRAME`] crosses, and
/// one more is refused. A bound that is never tested at its own value is a bound nobody
/// has checked the comparison operator of.
#[test]
fn the_cap_admits_exactly_its_own_number_and_refuses_one_more() {
    let _fd_table = serialized();
    let (vmm, isolate) = wire();
    let held: Vec<OwnedFd> = (0..MAX_FDS_PER_FRAME + 1)
        .map(|i| a_regular_file(&format!("cap{i}")).0)
        .collect();

    // At the cap: admitted.
    let at_cap: Vec<BorrowedFd<'_>> = held[..MAX_FDS_PER_FRAME].iter().map(AsFd::as_fd).collect();
    write_frame_with_fds(isolate.as_fd(), b"at-the-cap", &at_cap).unwrap();
    let (mut body, mut fds) = (Vec::new(), Vec::new());
    assert!(read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, MAX_FDS_PER_FRAME).unwrap());
    assert_eq!(
        fds.len(),
        MAX_FDS_PER_FRAME,
        "the cap admits its own number"
    );
    drop(fds);

    // One past it: the SENDER refuses, so a caller bug never becomes a wire event.
    let past_cap: Vec<BorrowedFd<'_>> = held.iter().map(AsFd::as_fd).collect();
    let err = write_frame_with_fds(isolate.as_fd(), b"past-the-cap", &past_cap)
        .expect_err("one more than the cap");
    assert!(
        matches!(err, FdFrameError::Os(RawError::Unsupported { .. })),
        "expected an Unsupported refusal, got {err:?}"
    );

    // And a receiver cannot raise its own allowance past the boundary's bound.
    let (mut b2, mut f2) = (Vec::new(), Vec::new());
    let err = read_frame_with_fds(vmm.as_fd(), &mut b2, &mut f2, MAX_FDS_PER_FRAME + 1)
        .expect_err("an allowance beyond the bound");
    assert!(
        matches!(err, FdFrameError::Os(RawError::Unsupported { .. })),
        "expected an Unsupported refusal, got {err:?}"
    );
}

// =====================================================================================
// Hazard 4 — O_CLOEXEC on receipt, exercised through a real exec
// =====================================================================================

/// ★★ A received descriptor must not survive into a child's `exec`.
///
/// `O_CLOEXEC` clears at the child's **own** `exec`, not at `fork` — the root cause of
/// the `ETXTBSY` flake measured this week at `7a36616` (32/300 against 0/300). The VMM
/// spawns isolates, so a descriptor received from isolate A and still open across the
/// spawn of isolate B is A's descriptor inside B.
///
/// ★ The check is by **link target**, not by descriptor number, so it cannot be fooled by
/// number reuse — and it carries its own **positive control**: the same file, handed to a
/// second child as its stdin, *is* found by the identical scan. Without that control the
/// test would pass just as happily against a child that could see nothing at all.
#[test]
fn a_received_descriptor_does_not_survive_into_a_childs_exec() {
    let _fd_table = serialized();
    let (vmm, isolate) = wire();
    let (file, path) = a_regular_file("cloexec");

    write_frame_with_fds(isolate.as_fd(), b"here-is-a-descriptor", &[file.as_fd()]).unwrap();
    let (mut body, mut fds) = (Vec::new(), Vec::new());
    assert!(read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 1).unwrap());
    let received = fds.pop().unwrap();

    // The parent really does hold it — otherwise "the child cannot see it" is trivially
    // true for the wrong reason.
    assert!(
        open_fd_targets().values().any(|t| t.contains(&path)),
        "the parent must hold the received descriptor for this test to mean anything"
    );

    // NEGATIVE: a child that execs must not find it.
    let out = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("ls -l /proc/self/fd")
        .output()
        .expect("spawn a child that execs");
    let seen = String::from_utf8_lossy(&out.stdout);
    assert!(
        !seen.contains(&path),
        "a received descriptor survived into a child's exec — it was not MSG_CMSG_CLOEXEC.\n\
         child fd table:\n{seen}"
    );

    // POSITIVE CONTROL: the same file, deliberately passed as the child's stdin, IS found
    // by the identical scan. This is what proves the negative above is about CLOEXEC and
    // not about a child that sees nothing.
    let control = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("ls -l /proc/self/fd")
        .stdin(std::process::Stdio::from(
            received.try_clone().expect("clone for the control"),
        ))
        .output()
        .expect("spawn the control child");
    let control_seen = String::from_utf8_lossy(&control.stdout);
    assert!(
        control_seen.contains(&path),
        "the positive control failed: a descriptor deliberately handed to the child was \
         not visible either, so the negative assertion above proves nothing.\n\
         child fd table:\n{control_seen}"
    );
}

// =====================================================================================
// Hazard 5 — cross-isolate. The security one.
// =====================================================================================

/// ★★★ A descriptor from isolate A, offered to isolate B, is refused **by name**.
///
/// `#14` — two concurrent CUDA applications — is this rewrite's founding problem, and a
/// per-process isolate that can be handed another process's GPU descriptor is not a
/// per-process isolate. ⊘ This is refused rather than assumed-away by topology: topology
/// is a property of today's call graph, and this is a check.
#[test]
fn a_descriptor_from_one_isolate_cannot_be_handed_to_another() {
    let _fd_table = serialized();
    let (a, b) = (iso(1, 0), iso(2, 0));
    let dev = CrossedFd::adopt(
        a_char_device(),
        FdOrigin::Isolate(a),
        DescriptorKind::CharDevice,
    )
    .expect("adopted from isolate A");

    // Back to its owner: fine.
    dev.lend_to(a)
        .expect("a descriptor may go back to the isolate it came from");

    // To a different isolate: refused, naming both.
    let err = dev
        .lend_to(b)
        .expect_err("isolate B must not receive isolate A's descriptor");
    match err {
        RawError::ForeignDescriptor { origin, target } => {
            assert_ne!(
                origin, target,
                "the refusal must name two different isolates"
            );
        }
        other => panic!("expected ForeignDescriptor, got {other:?}"),
    }
}

/// Same proc, **different GPU** — still a different isolate, and still refused.
///
/// ★ The trap this closes: an identity that compares only the proc would let a descriptor
/// cross between a proc's own two GPU isolates, which is exactly the collapse
/// `IsolateId`'s `Debug` impl exists to make visible.
#[test]
fn the_same_proc_on_a_different_gpu_is_still_a_different_isolate() {
    let _fd_table = serialized();
    let dev = CrossedFd::adopt(
        a_char_device(),
        FdOrigin::Isolate(iso(5, 0)),
        DescriptorKind::CharDevice,
    )
    .unwrap();
    assert!(
        dev.lend_to(iso(5, 1)).is_err(),
        "proc 5's GPU-0 isolate and proc 5's GPU-1 isolate are different isolates"
    );
    assert!(dev.lend_to(iso(5, 0)).is_ok());
}

/// A VMM-minted descriptor — a ring or a shareable backing — may go to any isolate. That
/// is the `ISOLATE_CMD_RECEIVE_FD` / `SETUP_RING` direction, and it names no isolate's
/// objects.
#[test]
fn a_vmm_minted_descriptor_may_go_to_any_isolate() {
    let _fd_table = serialized();
    let (ram, _) = a_regular_file("vmm-minted");
    let backing = CrossedFd::adopt(ram, FdOrigin::Vmm, DescriptorKind::RegularFile).unwrap();
    for id in [iso(1, 0), iso(2, 0), iso(2, 1)] {
        backing
            .lend_to(id)
            .expect("the VMM's own backing is not any isolate's property");
    }
}

/// ⊘ **The refusal is not reachable only through a helper.** `lend_to` is the *only* way
/// to obtain a sendable borrow from a `CrossedFd`, so the check cannot be skipped by
/// forgetting to call it — the borrow and the check are the same call.
///
/// Pinned as a test because it is a property of the API's shape, and API shape is what
/// erodes: if a future `pub fn as_sendable_fd(&self)` appears, this stops compiling only
/// if someone notices. So it is asserted here in prose *and* by the fact that every
/// send path in this file goes through `lend_to` or a local-use borrow.
#[test]
fn the_only_sendable_borrow_goes_through_the_cross_isolate_check() {
    let _fd_table = serialized();
    let dev = CrossedFd::adopt(
        a_char_device(),
        FdOrigin::Isolate(iso(9, 0)),
        DescriptorKind::CharDevice,
    )
    .unwrap();
    // The local-use borrow is deliberately unrestricted (the VMM received it), and is
    // NOT a sendable one: sending requires naming a target.
    let _local: BorrowedFd<'_> = dev.as_local_fd();
    assert!(dev.lend_to(iso(8, 0)).is_err());
}

// =====================================================================================
// Framing, and the error paths that must still close descriptors
// =====================================================================================

/// A peer that goes away between a frame's length and its body is a corpse, and is
/// distinguished from a peer that shut down cleanly.
#[test]
fn a_peer_that_dies_mid_frame_is_distinguished_from_a_clean_shutdown() {
    let _fd_table = serialized();
    // Clean shutdown: no bytes at all.
    let (vmm, isolate) = wire();
    drop(isolate);
    let (mut body, mut fds) = (Vec::new(), Vec::new());
    assert!(
        !read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 1)
            .expect("clean EOF is not an error"),
        "a clean end of stream is Ok(false), a state rather than an error"
    );

    // Mid-frame death: a length promising a body that never comes.
    let (vmm2, mut isolate2) = wire();
    isolate2.write_all(&64u32.to_le_bytes()).unwrap();
    isolate2.write_all(b"only a few").unwrap();
    drop(isolate2);
    let (mut body2, mut fds2) = (Vec::new(), Vec::new());
    let err = read_frame_with_fds(vmm2.as_fd(), &mut body2, &mut fds2, 1).expect_err("a corpse");
    assert_eq!(
        err,
        FdFrameError::Incomplete {
            what: "between a frame's length and its body"
        }
    );
}

/// ★ A descriptor that arrived on a frame whose *body* then fails is still closed.
///
/// The order matters and it is the hazard-1 case that is easiest to get wrong: the
/// descriptors arrive with the length word, so a body that never completes leaves them
/// already received. They are owned from the instant they exist, so the unwind closes
/// them.
#[test]
fn a_descriptor_on_a_frame_that_then_fails_is_still_closed() {
    let _fd_table = serialized();
    let (file, path) = a_regular_file("mid-frame");

    let before = open_fds();
    {
        let (vmm, isolate) = wire();
        // Length word + descriptor, then a body that never arrives.
        kayfabe_linux_raw::send_with_fds(isolate.as_fd(), &64u32.to_le_bytes(), &[file.as_fd()])
            .unwrap();
        drop(isolate);

        let (mut body, mut fds) = (Vec::new(), Vec::new());
        let err = read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 1).expect_err("a corpse");
        assert!(matches!(err, FdFrameError::Incomplete { .. }));
        // `fds` holds the descriptor that did arrive; dropping it here is what closes it,
        // and the assertion below is that nothing ELSE stayed open.
        drop(fds);
    }
    drop(file);
    let after = open_fds();
    assert_eq!(
        before.len(),
        after.len() + 1,
        "only the probe file itself should have closed; nothing may be left behind"
    );
    assert!(
        !open_fd_targets().values().any(|t| t.contains(&path)),
        "no descriptor onto the probe file may remain open"
    );
}

/// A declared length beyond `FRAME_MAX` is refused **without reading it** — a length the
/// peer controls must not be able to make us allocate.
#[test]
fn an_oversize_declared_length_is_refused_without_allocating() {
    let _fd_table = serialized();
    let (vmm, mut isolate) = wire();
    let absurd = (1u32 << 21) + 1;
    isolate.write_all(&absurd.to_le_bytes()).unwrap();
    let (mut body, mut fds) = (Vec::new(), Vec::new());
    let err = read_frame_with_fds(vmm.as_fd(), &mut body, &mut fds, 0).expect_err("oversize");
    assert_eq!(
        err,
        FdFrameError::Oversize {
            declared: absurd as usize
        }
    );
    assert!(
        body.is_empty(),
        "nothing was allocated for the peer's number"
    );
}

// =====================================================================================
// Hazard 6 — what this does NOT cover, said plainly
// =====================================================================================

/// ⊘ **Isolate lifetime is NOT observable at this seam, and is NOT covered here.**
///
/// A `CrossedFd` records *which* isolate a descriptor came from, and that is all it can
/// know. It does not learn that the isolate has since exited, been reaped, or been
/// replaced by a new isolate that reuses the same `(proc, gpu)` identity — nothing at
/// this seam is notified of any of those. So the cross-isolate check above answers
/// *"whose is it?"* and cannot answer *"is that one still alive?"*.
///
/// The consequence, stated rather than implied: a descriptor held across an isolate's
/// death and then lent to a **new** isolate with the same identity would be permitted by
/// [`CrossedFd::lend_to`]. It is a real gap; closing it needs a generation counter in
/// `IsolateId` or a lifetime signal from the spawner, and both are owner decisions rather
/// than something to invent here. `isolate_vmm_fd_crossing.md` §9 carries it as open.
///
/// This test asserts only that the gap is where the docs say it is — a same-identity
/// descriptor is accepted — so that closing it later has a test that must change.
#[test]
fn does_not_observe_isolate_lifetime() {
    let _fd_table = serialized();
    let dev = CrossedFd::adopt(
        a_char_device(),
        FdOrigin::Isolate(iso(4, 0)),
        DescriptorKind::CharDevice,
    )
    .unwrap();
    assert!(
        dev.lend_to(iso(4, 0)).is_ok(),
        "identity, not liveness: a reincarnated isolate with the same (proc, gpu) is \
         indistinguishable here, and §9 records that as open"
    );
}
