# The isolate ⇄ VMM descriptor crossing

> **Status: built, at the transport layer.** Task `#131`. The mechanism, its refusals and
> its tests are in the tree; the *verbs* that will use it are §10, and `#128`'s timer
> passthrough is explicitly not built here.

## 1. ★ The precondition `#128` has, and that nothing in this port could satisfy

`register_plane_read_native.md` rules that on the GPU's register / doorbell BAR pages
**reads are native passthrough and writes are trapped**. Read-native passthrough of a host
GPU page is two operations, and they are not in the same process:

| step | needs | held by |
|---|---|---|
| `mmap` the host GPU's BAR | the **GPU descriptor** (`/dev/nvidia*`) | the **isolate** — unprivileged, cap-dropped, own userns (`#96`, `2575177`) |
| install the mapping as a guest memslot | `KVM_SET_USER_MEMORY_REGION` on the **VM descriptor** | the **VMM** |

A KVM memslot names a userspace address **in the VMM's own address space**, so the mapping
must exist there. ⇒ **something has to cross**, and until this task nothing could: a grep
of the whole Rust tree found zero `SCM_RIGHTS`, zero `sendmsg`/`recvmsg` carrying
descriptors. Only the VMM → isolate direction was even designed, and only for guest RAM
(`l1_os_shell.md` §4.4.1).

The C has had both directions since its first isolate:
`ISOLATE_CMD_RECEIVE_FD` (VMM → isolate) and `ISOLATE_RESP_OPEN_DEVICE` (isolate → VMM,
*"stub opens /dev/nvidia\*, replies w/ SCM_RIGHTS fd"*), plus `ISOLATE_CMD_SETUP_RING` for
the double-mmap — `C: src/common/nvkvm_isolate_proto.h:43,51,54,67`. This is a **port of
that**, not a new mechanism.

## 2. ★★★ The posture question, answered plainly

Handing a GPU descriptor to the VMM looks like it undoes `#96` — the property that a
cap-dropped, userns-confined process is what drives the GPU. It does not, and the reason is
worth stating exactly rather than asserting:

> **The *open* stays on the unprivileged side.** The isolate opens `/dev/nvidia*` — it is
> the process with the device directory, inside the mount namespace where those nodes
> exist — and replies with a descriptor. The VMM never reaches for `/dev/nvidia*` itself,
> never needs the path, and never needs the privilege to traverse to it. This is exactly
> what the C does, and it is why the C's design is the one to port.

**What the VMM can do with a received GPU descriptor**, stated so it can be argued with:

- `mmap` it — which is the point: the BAR page has to exist in the VMM's address space for
  a memslot to name it.
- `ioctl` it. **Nothing structurally prevents this**, and that is the honest answer.
- Keep it past the isolate's death (§9).

**What it cannot do:** it cannot obtain a descriptor the isolate did not choose to open, it
cannot open a *different* device (there is no path and no directory descriptor on that
side), and it cannot escalate — a descriptor confers exactly the access the opener had.

### ⚠ Does the threat model change? Yes, in one direction, and it is worth saying so

The `#96` property as usually stated — *"an unprivileged, confined process is what drives
the GPU"* — is **weakened** by this crossing, because after it the VMM also holds a
descriptor that can drive the GPU. What survives is the narrower and still-valuable
property: *"the process that **opens** the GPU, and the only process that can **reach** the
device nodes, is unprivileged and confined."*

⊘ Do not restate the broader property as though it survived. The C carries the same
weakening and never wrote it down; this note is the place it gets written down. If the
owner wants the broader property back, the mechanism that buys it is a **`seccomp` filter
on the VMM refusing `ioctl` on descriptors of this class**, or moving memslot installation
behind a verb the isolate performs — both are design changes, not adjustments.

★ Note the asymmetry that makes this tolerable: the VMM is the **more** trusted side. It
already holds the KVM descriptor, all guest RAM, and every isolate's socket. A GPU
descriptor adds to a process that could already do more damage than the GPU descriptor
grants. The dangerous direction is the other one, and that direction is §7.

## 3. The operation split

**Isolate-side** — everything that needs the device:

- opening `/dev/nvidia*`, `/dev/nvidiactl`, `/dev/nvidia-uvm`
- every RM `ioctl`: allocation, control, VA mapping, channel scheduling, CE copies
- `mmap` of GPU objects for the isolate's own use
- ★ minting the descriptor that crosses, in the `ISOLATE_RESP_OPEN_DEVICE` direction

**VMM-side** — everything that needs the VM:

- `KVM_SET_USER_MEMORY_REGION`, slot-number allocation, `MemoryRegion` registration
- `mmap` of a *received* descriptor into the VMM's address space, so a memslot can name it
- minting the shareable backing (`memfd`) and the SPSC rings, in the
  `ISOLATE_CMD_RECEIVE_FD` / `SETUP_RING` direction
- spawning, sandboxing and reaping isolates

**Neither side decides what the other's descriptor is for.** The transport carries a
descriptor and a promise about its *kind*; policy about which message may carry one is the
protocol's, and it is expressed as the `max_fds` allowance on each read (§6).

## 4. What was built

| layer | file | what |
|---|---|---|
| raw | `crates/kayfabe-linux-raw/src/scm_unsafe.rs` | `send_with_fds`, `recv_with_fds`, `descriptor_kind`, `require_kind`, `MAX_FDS_PER_FRAME` |
| transport | `crates/kayfabe-isolate-host/src/fdcross.rs` | `write_frame_with_fds` / `read_frame_with_fds` (the fd-carrying twins of `proto::write_frame` / `read_frame`), `CrossedFd`, `FdOrigin` |
| tests | `crates/kayfabe-isolate-host/tests/fd_crossing.rs` | 18 tests, the seven hazards of §8 |

One framing property is load-bearing and easy to get backwards: **ancillary data attaches
to the first byte of a `sendmsg`**. `proto::write_frame` already emits `len‖body` in one
`write_all` for an unrelated reason (a signal-interrupted partial write must not
desynchronise the channel); here that same shape is what lets the receiver associate
descriptors with a frame at all. ⇒ the control buffer is supplied on the **length-word
read**, not the body read. Supply it on the body read and the bytes arrive perfectly while
the kernel silently drops the descriptors.

## 5. ★★★ A received descriptor is untrusted input

The message body carries the sender's **claim**. `fstat` reports what the object **is**.
They are independent, and on the VMM's receive side the peer is the isolate — deliberately
the *less* trusted end.

`CrossedFd::adopt` refuses the mismatch by name — `RawError::DescriptorKindRefused
{ expected, actual }` — **before** the descriptor is usable, because the next thing the VMM
does with a "GPU device" is `mmap` it and install the result as a guest memslot. A
compromised isolate answering with its own end of a socket, a directory, or a descriptor
onto an unrelated file is the vector; without the check it is a working one.

⊘ **The C does not have this check.** `C: src/qemu/nvkvm_isolate.c:441-462` gates on the
*message type* that may legitimately carry a descriptor and closes the rest — which is a
real defence (its own audit finding R2-M1, against descriptor-table exhaustion) and says
nothing about what the descriptor **is**. Porting that gate alone would reproduce the gap,
so both are here: the type check in §5, the message-type gate as §6's allowance.

★ `adopt` takes the descriptor **by value**. A validator that borrows leaves its caller
holding an open descriptor it has just been told to distrust; taking ownership means the
refusal path *closes* it, by `Drop`, with no arm to forget.

## 6. The allowance, and `MSG_CTRUNC`

`recv_with_fds` takes a per-call `max_fds`, and the control buffer is **sized from it**.
Everything follows from that one decision:

- A peer that attaches more gets `MSG_CTRUNC`; the kernel closes the excess itself; the
  frame is refused with `RawError::TooManyDescriptors { limit }`.
- ⊘ **`MSG_CTRUNC` is never ignored.** It is the *only* evidence that truncation happened —
  a receiver that skips the flag believes it received everything while the sender believes
  it handed over more. The frame is refused **whole**, never served from the prefix that
  fitted, because a peer must not get to choose which of its descriptors we act on.
- `max_fds == 0` is the ordinary case and is the port of the C's R2-M1 sweep — but where
  the C closes stray descriptors in a loop a later `case` can forget to run, here the
  kernel never hands them over at all.
- Because the cap is per-call, the refusal is **testable by undersizing it**: send two,
  receive with `max_fds = 1`, watch it fire. It was watched.

## 7. ★★★ Cross-isolate: the security one

Per-process isolates are the architecture, and `#14` — two concurrent CUDA applications —
is this rewrite's founding problem. **A descriptor from isolate A reaching isolate B is a
breach, not an untidiness**: it is a live handle onto A's GPU objects landing in B's table.

`CrossedFd` therefore carries its `FdOrigin`, and `lend_to(target)` is the **only** way to
obtain a sendable borrow:

- `FdOrigin::Vmm` → any isolate. The VMM minted it; it names no isolate's objects.
- `FdOrigin::Isolate(a)` → **only** `a`. Anything else is
  `RawError::ForeignDescriptor { origin, target }`.

⊘ Refused, not prevented by topology. Topology is a property of today's call graph; this is
a check. The identity compared is the whole `IsolateId` — ★ a proc's own GPU-0 and GPU-1
isolates are *different isolates*, and an identity that compared only the proc would let a
descriptor cross between them.

**The C's one cross-isolate transfer is a deliberate exception and stays out of scope.**
`ISOLATE_CMD_XISO_IMPORT` (`C: src/common/nvkvm_isolate_proto.h:57`, its `#110`) brokers a
dma-buf from an owning isolate into a compositor's, guarded by a *comment* —
*"QEMU guarantees both isolates belong to the same VM"* — rather than by a check. It is a
Mode-1 graphics feature. If it is ever ported, it is an explicitly argued exception with an
owner ruling, and the default above is what it must argue against.

## 8. What is tested, hazard by hazard

`crates/kayfabe-isolate-host/tests/fd_crossing.rs`, 18 tests.

| # | hazard | covered? |
|---|---|---|
| 1 | leak on every error path | **yes** — every refusal arm asserts the refused descriptor closed *and nothing else changed* |
| 2 | `MSG_CTRUNC` | **yes** — induced by undersizing the allowance |
| 3 | descriptor-table exhaustion | **yes** — tested *at* the cap, and one past it, both directions |
| 4 | `O_CLOEXEC` on receipt | **yes** — through a real `exec`, with a positive control |
| 5 | cross-isolate | **yes** — refused by name; same-proc/different-GPU too |
| 6 | isolate lifetime | ⊘ **NO** — see §9 |
| 7 | type validation | **yes** — char device / regular file / socket, all three |

★ The anti-vacuity test is `a_received_descriptor_is_the_same_object_as_the_one_that_was_sent`:
it writes through the **received** descriptor and reads it back through the **original**.
Every other test in the file would pass against an implementation that quietly opened its
own descriptor and sent nothing.

★★ Two instrument defects were found by these tests failing, and both are recorded in the
file because they are the reusable lesson:

1. **The descriptor table is process-wide; libtest runs tests in threads of one process.**
   Four tests failed on the first run measuring their neighbours' descriptors. Fixed with a
   file-wide lock.
2. **The observer must not appear in its own measurement** (task #131, 2026-07-31).
   `read_dir("/proc/self/fd")`
   takes a descriptor, and the kernel hands out the **lowest free** number — which,
   immediately after a refusal closed fd 3, is fd 3. The snapshot then contained a 3 again
   and the assertion read it as *"the refused descriptor is still open"* when the refusal
   had worked perfectly. ⇒ suspect the instrument first; it was the instrument twice.

### What was run (task #131, 2026-07-31, `crates/kayfabe-isolate-host/tests/fd_crossing.rs`)

**What was measured** — run named: task #131, 2026-07-31, on the 38-core build box, base
`c97b640`. All 18 tests in `crates/kayfabe-isolate-host/tests/fd_crossing.rs` pass, and
each defence was induced and **watched to fire** before being restored:

| bite | tests that failed |
|---|---|
| kind check removed from `CrossedFd::adopt` | 3 |
| `MSG_CTRUNC` branch made unreachable | 2 |
| cross-isolate arm in `lend_to` made permissive | 3 |
| `MSG_CMSG_CLOEXEC` dropped from the `recvmsg` | 1 (the `exec` test) |
| refused descriptor `mem::forget`ed instead of dropped | 2 |

⚠ **What was NOT measured**: no real `/dev/nvidia*` descriptor has crossed, because no verb
uses this transport yet (§10 item 1). Every run above is socketpair-level and in-process,
over `/dev/null` and `tmpfs` files. §11 item 3 carries that as the standing bound.

## 9. ⊘ Isolate lifetime is NOT observable here, and is NOT covered

A `CrossedFd` records **which** isolate a descriptor came from. That is all it can know. It
is not told that the isolate has since exited, been reaped, or been replaced by a new
isolate reusing the same `(proc, gpu)` identity — nothing at this seam is notified of any
of those.

**The consequence, stated rather than implied:** a descriptor held across an isolate's
death and then lent to a **new** isolate with the same identity would be permitted by
`lend_to`. That is a real gap. Closing it needs either a generation counter in `IsolateId`
or a lifetime signal from the spawner, and both are owner decisions rather than something
to invent here.

`does_not_observe_isolate_lifetime` asserts the gap is exactly where this section says it
is, so closing it later has a test that must change.

## 10. What `#128` still needs after this

The crossing exists; the timer/register passthrough does not, and was deliberately not
built. In order:

1. **A verb.** `Request`/`Reply` in `proto.rs` are *one variant per `RmBackend` verb* — "the
   port is the protocol" — and `RmBackend` is in a **pure logic crate**. Adding
   "hand me the device descriptor" to it is a change to the port, i.e. an owner decision,
   not something to slip in beside a transport. **This is the next question to settle**, and
   the C's `ISOLATE_CMD_OPEN_DEVICE` is the shape to port once it is settled.
2. **Wiring `ProxyRmBackend::call` / `child.rs::worker_loop`** onto the fd-carrying frames.
   Both already hold the socket; today they use the fd-free `write_frame`/`read_frame`. This
   is mechanical once (1) exists.
3. **A `RamHandle` accessor on the VMM side.** `Installer::exports` holds the `OwnedFd` and
   `RamHandle.token` is just its index — there is no accessor that hands it back out. The
   VMM → isolate direction cannot carry a real backing until there is one.
4. **BAR geometry**: which BAR, which offset, what length, and the refusal when the host
   GPU's BAR is smaller than the guest's view of it.
5. **The write-trap half.** Reads native, writes trapped — §1 of
   `register_plane_read_native.md`. This note builds neither; it builds the crossing the
   read half needs.
6. **The security-model paragraph** `register_plane_read_native.md` §7 already asks for: a
   read-only free-running host counter is a high-resolution timing side channel leaking host
   GPU uptime. Add §2's posture weakening to the same paragraph.

## 11. What remains before this is production-safe

Beyond `#128`'s own list, and answering the owner's question as a list:

1. **§9's lifetime gap** — the one substantive hole. Needs a generation counter or a
   spawner signal.
2. **§2's posture weakening** — decide whether the VMM holding an `ioctl`-capable GPU
   descriptor is acceptable, or whether it needs a `seccomp` filter. ★ Today it is
   *stated*, not *mitigated*.
3. **No verb uses the crossing yet**, so it has never run against a real isolate or a real
   `/dev/nvidia*`. Everything measured here is socketpair-level, in-process, on `/dev/null`
   and `tmpfs` files. That is a genuine bound on what the green means.
4. **`MAX_FDS_PER_FRAME = 4` is a bound, not a measurement.** No message in the C carries
   more than one. If a multi-plane message ever needs more, raise it as a design change.
5. **Descriptor budget interaction.** `linux-raw` already has `descriptor_budget`; nothing
   yet charges received descriptors against it, so a slow leak across many isolates would
   surface as `EMFILE` somewhere unrelated rather than as a refusal here.
6. **No fuzzing of the cmsg walk.** The walk trusts the kernel's own `cmsg_len`, which is
   correct, but the frame layer above it parses peer-controlled lengths and is only
   tested by example.
