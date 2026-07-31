# The isolate ⇄ VMM descriptor crossing

> **Status: built, at the transport layer — and §12 now has its FIRST VERB.** Tasks `#131`
> (the transport) and `#133` (the verb). ★ **Read §12 before §2:** the owner ruled on
> 2026-07-31 that the mapping work moves behind an isolate verb, so for the classes that
> verb covers **the GPU descriptor no longer crosses at all**. §2 is left standing as the
> analysis that produced the decision, with its one false clause corrected in place.

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

**What it cannot do:** it cannot obtain a descriptor the isolate did not choose to open, and
it cannot open a *different* device (there is no path and no directory descriptor on that
side).

⚠ **CORRECTED 2026-07-31 (`guest_blast_radius.md` F14).** This list used to end with *"and it
cannot escalate — a descriptor confers exactly the access the opener had."* **That is false
for RM.** `RmIoctl` sets `secInfo.privLevel = osIsAdministrator() ? RS_PRIV_LEVEL_USER_ROOT :
RS_PRIV_LEVEL_USER` at the **top of every escape**
(`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:304`, the sole occurrence), and
`nv_file_private_t` carries no privilege field at all. ⇒ **a descriptor confers the access
the CALLER HAS AT IOCTL TIME.** On a root VMM the same descriptor that yields 768
non-privileged controls in the isolate yields all 265 `PRIVILEGED` ones. Permission-checked-
at-`open` is how ordinary files work and it is not how this driver works.

### ⚠ Does the threat model change? Yes, in one direction, and it is worth saying so

The `#96` property as usually stated — *"an unprivileged, confined process is what drives
the GPU"* — is **weakened** by this crossing, because after it the VMM also holds a
descriptor that can drive the GPU. What survives is the narrower and still-valuable
property: *"the process that **opens** the GPU, and the only process that can **reach** the
device nodes, is unprivileged and confined."*

⊘ Do not restate the broader property as though it survived. The C carries the same
weakening and never wrote it down; this note is the place it gets written down.

★★★ **The owner chose the second option, on 2026-07-31, and rejected the first on
threat-model grounds.** Two mechanisms were named here — a `seccomp` filter on the VMM, or
*"moving memslot installation behind a verb the isolate performs"*. The ruling, verbatim:

> *"(a) isn't really possible since the VMM does much more than only our project, we can't
> sandbox that, it isn't our job also (that's the VMM's job). Our worst case scenario for
> security is if the VMM (the hypervisor) is compromised; a further compromise is out of
> scope (like the isolated container/namespace the VMM runs in)."*

⇒ §12 is the verb. And note what the same ruling does to this section's own framing: a
compromised VMM is the **boundary**, so the weakening recorded above is a weakening of the
`#96` property as an engineering statement, not a hole in the blast-radius property P —
`guest_blast_radius.md` §1.1 carries that correction.

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

1. ~~**A verb.**~~ ★ **SETTLED AND BUILT — §12.** The owner authorised the change to
   `RmBackend` on 2026-07-31 and it is `RmBackend::export_backing`. ⚠ Note what it is *not*:
   the question here was posed as *"hand me the device descriptor"*, and the answer the owner
   gave is the opposite one — **hand me the memory**. The C's `ISOLATE_CMD_OPEN_DEVICE` is
   therefore **not** the shape that was ported.
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
2. ~~**§2's posture weakening**~~ — ★ **DECIDED, 2026-07-31.** Not a `seccomp` filter: §12's
   verb, so that for the classes it covers no `ioctl`-capable descriptor is held by the VMM
   at all. ⚠ **Still open for the class §12 cannot cover** — real device MMIO — where the
   answer today is that the mapping is *refused by name* rather than performed. See §12.4.
3. ~~**No verb uses the crossing yet**~~ — ★ **PAID IN PART.** `export_backing` runs the
   crossing against a **real spawned isolate over a real socket**
   (`crates/kayfabe-isolate-host/tests/export_backing.rs`, 7 tests, 7 bites). ⚠ What is still
   unrun is a real `/dev/nvidia*`: no descriptor from a real driver has crossed, and the
   sharpest assertion in that file — that an RM escape on the received backing is refused —
   has never been *seen to fail*, because making it fail needs a descriptor that genuinely
   serves one. Owed on the hardware box.
4. **`MAX_FDS_PER_FRAME = 4` is a bound, not a measurement.** No message in the C carries
   more than one. If a multi-plane message ever needs more, raise it as a design change.
5. **Descriptor budget interaction.** `linux-raw` already has `descriptor_budget`; nothing
   yet charges received descriptors against it, so a slow leak across many isolates would
   surface as `EMFILE` somewhere unrelated rather than as a refusal here.
6. **No fuzzing of the cmsg walk.** The walk trusts the kernel's own `cmsg_len`, which is
   correct, but the frame layer above it parses peer-controlled lengths and is only
   tested by example.

---

## 12. ★★★ The verb — decision (b), and the boundary it does not reach

> **Task `#133`, built 2026-07-31.** `RmBackend::export_backing`. The owner's ruling is
> quoted in §2; this section is what it became, and — as a **first-class result** rather
> than a caveat — how far it reaches.

### 12.1 The shape, and where each piece lives

| piece | where | what |
|---|---|---|
| `ExportSource`, `ExportRequest`, `ExportedBacking` | `kayfabe-isolate` (**pure**) | value types; no OS type appears |
| `RmError::NotExportableAsMemory` | `kayfabe-isolate` | ⊘ the named boundary |
| `RmBackend::export_backing` | `kayfabe-isolate` | the verb, in the same shape as its siblings |
| `Worker::export_backing` | `kayfabe-isolate` | R1 assert + the foreign-handle gate, beside `fb_read` |
| `Request::ExportBacking` / `Reply::Backing` | `isolate-host::proto` | one variant per verb, as the port requires |
| `ChildExports` / `ExportRegistry` | `isolate-host::export` | the two tables, one per side |
| `mint_fabricated` | `isolate-host::rm` | the memfd mint, shared by the real backend and the fixture |

★ `RmBackend` lives in a pure logic crate, which is why two previous agents declined to
touch it unasked and why this needed an owner decision rather than a refactor. The verb
carries **no OS types**: it names a token, an offset, a length and a `kayfabe_vmm::Prot`,
exactly as `export_surface` names a `SurfaceHandle`.

### 12.2 ★★ What crosses instead of the device descriptor

A **sealed `memfd`** (`kayfabe_linux_raw::SharedRam`: `F_SEAL_SHRINK | F_SEAL_GROW |
F_SEAL_SEAL`). The isolate mints it, the descriptor rides the export reply's ancillary data,
the VMM adopts it and `mmap`s it, and the pages both processes see are the same pages.

Three properties follow, and each is asserted:

1. **It is memory, per the kernel.** `ExportRegistry::adopt` hands `CrossedFd::adopt` the
   promise `DescriptorKind::RegularFile`, so a child answering with a character device is
   refused **by name** and the descriptor is *closed* on the refusal path. ★ This check is
   deliberately independent of the child's own refusal in §12.4: a compromised isolate is
   inside the threat model, so the parent does not take its word for anything.
2. **There is no RM surface behind it.** Every NVIDIA frontend escape issued on the received
   descriptor answers `ENOTTY`. Measured, not argued — see §12.6.
3. **`max_fds = 1` on exactly one reply.** Every other verb's reply is read by the fd-free
   `read_frame`, which supplies no control buffer at all, so a child that attaches a
   descriptor to an `Alloc` reply has it dropped by the kernel. §6's allowance, used.

★ The token is minted **twice and never carried**: the child indexes its own table, the
parent indexes its own, and `Reply::Backing` has no token field. A peer-supplied index into
*our* registry would let a compromised child make the VMM install one backing where it asked
for another — a mapping of the wrong bytes, which this design ranks as the worst outcome
available. The two are associated by the channel being 1-deep, not by a correlator.

### 12.3 ★★★ HOW FAR (b) REACHES — the result, and it is a two-class answer

> **(b) is COMPLETE for memfd-backed regions and INCOMPLETE for real device MMIO.**

| mapping | class | under (b) |
|---|---|---|
| guest RAM | memfd (VMM-minted) | already memory; needs no verb — the VMM→isolate direction |
| the emulated device's framebuffer / instance window / `PRAMIN` view (`viewer_install::HostBacking::VmmOwned`) | memfd | ✔ **covered**; already VMM-minted, and now mintable by the isolate too |
| any fabricated aperture — bytes that exist only because we wrote them | memfd | ✔ **covered**, `ExportSource::Fabricated` |
| the isolate's SPSC rings / shared control pages | memfd | ✔ covered by the same mechanism |
| ⊘ the **real card's** framebuffer (`viewer_install::HostBacking::HostGpuFramebuffer`) | device | ✘ **not covered** |
| ⊘ a channel's ring / USERD / the `AMPERE_USERMODE_A` doorbell window | device | ✘ **not covered** |
| ⊘ BAR0 register pages — `#128`'s read-native `NV_PTIMER` passthrough | device | ✘ **not covered** |

**Why the device class cannot be covered — three independent reasons.** ★ Two of the three
are **`[src@580]` readings** at a named file:line and no GPU was switched on for either; the
third is a reading of this tree. That is deliberately enough here — both driver citations are
unconditional refusals on the source path with no runtime input — but they are *readings*,
said as readings, per `claim_ledger.md`:

1. **The only object whose `mmap` yields a host GPU page is `/dev/nvidia<N>`** carrying a
   registered mapping context (`RmConnection::map_cpu`; the offset must be zero and the
   context is one-shot per descriptor — `ogkm-580: kernel-open/nvidia/nv-mmap.c:533-536`,
   `nv-usermap.c:53-57`). That is a **character device**, i.e. exactly the thing (b) exists
   to stop crossing. Crossing it would be option (a) wearing a verb's clothes.
2. **NVIDIA's dma-buf is the one escape hatch, and it is shut on discrete parts.** A dma-buf
   fd is *not* an RM surface — its `file_operations` are the dma-buf ones, and
   `nv_get_file_private` would refuse it — so it would have satisfied (b) cleanly. But CPU
   mapping of one is gated:
   `*pbCanMmap = pGpu->getProperty(pGpu, PDB_PROP_GPU_ZERO_FB)`
   (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:5609`, whose own comment reads
   *"mmap is allowed only for 0FB chips (iGPU)"*), and `nv_dma_buf_mmap` refuses outright
   when it is false (`ogkm-580: kernel-open/nvidia/nv-dmabuf.c:1246-1250`). Every discrete
   card this project targets has a framebuffer. ⇒ on our hardware a dma-buf of device memory
   **cannot be mapped by the CPU at all**, so it cannot back a memslot.
   ⚠ The neighbouring `NV0000_CTRL_OS_UNIX_EXPORT_OBJECT_TO_FD` is not an alternative
   either: it attaches an export handle to an **existing NVIDIA device fd**
   (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/os.c:2274-2280` — `nv_get_file_private`), so
   what crosses is still an RM escape surface.
3. **Our own memory plane refuses the result independently.**
   `kayfabe_linux_raw::GuestWindow::place` rejects `Backing::DeviceFile` with
   `RawError::DeviceBackingNotPlaceable`, so even a crossed device descriptor could not be
   installed by `Vmm::map_guest` as it stands.

★★ **The boundary coincides with a boundary the tree already had, and that is the deepest
form of the result.** `Backing::attainable_cache_policy` answers `Some(WriteBack)` for a
shared file and **`None`** for a device file, because for a device file *"the driver already
decided … and userspace cannot read it back"*. The class (b) can export is **exactly** the
class whose effective CPU memory type is knowable. One boundary, two consequences: what we
can hand over as memory is what we can also state the memory type of.

### 12.4 ⊘ The refusal, and why it is a result rather than a gap

`ExportSource::HostDeviceMemory` is **always** `RmError::NotExportableAsMemory { memory }` —
in the real backend, in the loopback fixture, and in `MockRmBackend`. It is a variant of the
request type rather than an absence, so the incomplete half of (b) is *expressible*, and a
test watches it fire.

⊘ **Two ways of faking coverage, both explicitly refused.** (i) Crossing the device fd
anyway, behind the verb — that is (a) with extra steps and it is the exact thing the ruling
rejected. (ii) Copying the device pages into a `memfd` — a copy is not a mapping; the guest
would read a snapshot of a live aperture, which is the forged-completion class with a longer
fuse.

★ **What this does to `viewer_install::InstallRefusal::HostGpuBackingHasNoVerb`.** Its
*behaviour* is unchanged and still correct — an object whose bytes are the real card's still
stops the drain and installs nothing. What changes is its **reason**: it is no longer *"no
verb uses the crossing, and adding one is an owner decision"* but *"the verb exists, the
decision was taken, and the verb refuses this class for the three reasons in §12.3."* The
refusal has moved from **unbuilt** to **decided**.

### 12.5 What the mmap installer can rely on

`kayfabe_vmm_qemu::viewer_install` is the consumer. The verb was shaped for it:

- **Per object, not per page.** `export_backing` takes a length and returns one backing;
  nothing here enumerates a page, and the installer's consolidation is free to place many
  objects inside one exported backing at different offsets.
- **The merge key survives.** `ExportedBacking` carries `prot` as **what was granted**, not
  what was asked, so a run's `MergeKey::prot` can be read off the outcome. The `cache` field
  of the key is `WriteBack` **by construction** for everything (b) exports — that is
  `attainable_cache_policy`'s answer for a shared file, not a request — which is why the
  verb deliberately does *not* carry a memory-type field of its own: a second enum beside
  `CachePolicy` would be a second source of truth for one fact.
- **`HostRegion` is one hop away.** `ExportRegistry::dup(token)` yields an `OwnedFd` the VMM
  adopts exactly as `KvmMachine::register_backing` already adopts one — whose own rustdoc
  reads *"create a host backing an isolate would have handed us"*. That sentence is now
  literal.

### 12.6 What was measured (task `#133`, 2026-07-31, 38-core build box, base `90eb50f`)

`crates/kayfabe-isolate-host/tests/export_backing.rs` — 7 tests, all green, each driving a
**real spawned isolate child over a real socket**. The load-bearing one is
`the_vmm_cannot_issue_an_rm_ioctl_on_what_it_received`: five NVIDIA frontend escapes
(`RM_ALLOC`, `RM_CONTROL`, `RM_FREE`, `RM_ALLOC_MEMORY`, `CHECK_VERSION_STR`) issued on the
received backing, every one `ENOTTY`, with a positive control on the **same descriptor** that
must succeed first.

★★ **The control was wrong the first time, and correcting it made it better.** It began as
*"a socket serves `FIONREAD`, a `memfd` does not"* — and the `memfd` answered `Ok(0)`,
because Linux serves `FIONREAD` for an ordinary file generically. Holding the *object* fixed
and varying only the *request* is the version that isolates the claim. Two instrument
defects from `fd_crossing.rs` were also re-met and are recorded in the file: the descriptor
table is process-wide across libtest threads, and `read_dir` reuses the number a refusal just
closed.

**Seven bites, each induced, watched to fire on exactly its own tests, and removed:**

| bite | fired |
|---|---|
| the child lends `/dev/null` instead of the backing it minted | 3 tests |
| …and `CrossedFd::adopt`'s kind check removed as well | 4 tests |
| the export reply read with `max_fds = 0` | 3 tests |
| `serve_one` routed through `execute`, so no descriptor is attached | 4 tests |
| the device arm answers with a `memfd` instead of refusing | 1 test (the boundary's own) |
| the foreign-handle gate removed from `Worker::export_backing` | 1 test |
| `ExportRegistry::adopt` always returns token 0 | 1 test |

⊘ **The bite that could NOT be paid, named rather than omitted:** making the `ENOTTY`
assertion fail needs a descriptor that genuinely **serves** an RM escape — a real
`/dev/nvidia*`. No substitute character device answers those request numbers, so that
assertion is true and has never been seen to fail. It is owed on the hardware box, and until
then the property is carried by the two kind assertions beside it — one through `fstat`, one
through `/proc/self/fd`, deliberately independent so that removing the check does not silence
both.
