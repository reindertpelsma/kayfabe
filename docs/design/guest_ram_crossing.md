# The guest-RAM crossing — measured against the code, and against the bench

**Status:** 2026-08-10, branch `guest-ram-crossing`, rev `954f926`, measured on `vh2`
(vast 47373001, RTX 3060 GA106, host 580.159.04 open). Successor to the design sketch
`/workspace/nvidia-gpu-passthrough/docs/design/mode2_guest_ram_crossing.md` (`21e6876`),
which was explicitly *"unverified against the code"* and asked to be led with its refutations.
It carries tasks **#233** (the crossing) and **#231** (the owner's passthrough ruling).

⊘ **This page supersedes that sketch.** Where the two disagree, this one was measured.

---

## 0. ★★★ What the sketch got WRONG — lead with this

### R1. ⊘⊘ "Have the shim adopt the memfd as one of its own windows" is STRUCTURALLY IMPOSSIBLE

Both the sketch's §2 and the task framing say the launch-time path is to add a shared RAM
backend *"and have the shim adopt it as one of its own windows."* A window is exactly the thing
guest RAM can never be. `install_window_inner` refuses on two independent grounds before it ever
looks at a backing (`crates/kayfabe-vmm-qemu/src/lib.rs:1173-1181`):

```rust
let Some(placement) = p.bar_for(gpa, len) else {
    return Err(VmmError::Unsupported("a reservation that is not inside any realized BAR"));
};
if !p.host.bar_is_unbacked_reservation(placement.bar) {
    return Err(VmmError::Unsupported(WINDOW_IN_A_BACKED_BAR));
}
```

Guest DRAM is **not inside any BAR of the `nvkvm-gpu` device**, so the first arm refuses it; and
even if it were, the second demands a BAR the hypervisor does **not** back, which is the
negation of what DRAM is. The `bar_is_unbacked_reservation` contract spells out why this is load
bearing rather than incidental (`crates/kayfabe-vmm-qemu/src/host.rs:246-258`): the accelerator's
listener takes an unconditional early return for a non-RAM region *before allocating a slot*, and
**that early return is the entire safety argument for installing foreign memslots**. A window
over real RAM would get a hypervisor-managed slot over the same guest-physical range as ours, and
only one of the two can win.

⇒ The crossing needs a **new concept that is not a window** — a machine-RAM export whose
lifetime, refusals and token space are its own. Reusing `install_window` would not be a shortcut;
it would be deleting the memslot-safety argument.

### R2. ⊘ "The KVM backend already has the right shape" — the two backends are the SAME CODE

The sketch's §2 says the KVM adapter *"already has the right shape (a `GuestWindow` over a real
sealed memfd); it is not what the bench boots"*, which invites the inference that porting a shape
across is the fix. The repo's own consolidation table measures the two `export_ram` bodies at
**85 % identical, "identical but for the refusal constant"**
(`crates/kayfabe-vmm-kvm/src/lib.rs:149`). Both search kayfabe's own reservations, both end at
`Arc<SharedRam>::dup_for_export()`:

| | QEMU | KVM |
|---|---|---|
| `export_ram` | `vmm-qemu/src/lib.rs:2275-2306` | `vmm-kvm/src/lib.rs:1873-1905` |
| backing created by | `vmm-qemu/src/lib.rs:1229-1245` | `vmm-kvm/src/lib.rs:991-1005` |

The real difference is **ownership, not shape**: under KVM kayfabe *is* the VMM, so its windows
*are* guest RAM; under QEMU kayfabe is a device, and R1 forbids its windows from being RAM. The
missing capability is *acquiring a descriptor for memory kayfabe did not allocate* — which no
amount of shape-porting produces.

### R3. ⊘ "Every other reply is read with an fd allowance of ZERO" — there is no allowance at all

The sketch's §1 (and #233's item (iii)) describe the fd-IN gap as an allowance set to zero,
which reads as *"flip a 0 to a 1."* In production there is **no `recvmsg`-based reader on the
request path at all**. The child reads every request with `read_frame`, a plain `impl Read`
reader with no control buffer (`crates/kayfabe-isolate-host/src/child.rs:331`;
`proto.rs:810`), so the kernel *drops* attached descriptors rather than any code refusing them.
`max_fds = 0` is passed **only in tests** (`tests/fd_crossing.rs:271,:416,:716`). The one
production call with a nonzero allowance is the parent reading the single fd-bearing *reply*
(`isolate.rs:414`). The code's own docstring is already correct where the sketch is not
(`isolate.rs:395-399`): *"Every other verb's reply is read by `read_frame`, which has **no**
control buffer at all."*

⇒ The edit is *"give the child's request loop a `recvmsg` reader"*, not *"raise a limit."*
⊘ Everything below the verb is already built and direction-agnostic:
`send_with_fds`/`recv_with_fds` (`crates/kayfabe-linux-raw/src/scm_unsafe.rs:146`, `:344`), and
the tree says so outright — *"the isolate ⇄ VMM descriptor crossing landed (`SCM_RIGHTS`, both
directions) but **no verb uses it**, deliberately"*
(`crates/kayfabe-vmm-qemu/src/viewer_install.rs:83-85`).

### R4. ★ And the ONE the sketch got right, it got right for a reason it never states

§2 asserts guest RAM must be a shared fd-backed block via `memory-backend-memfd,share=on`. That
is correct, and **§1 below measures it working** — but the sketch never says *how the shim
obtains the descriptor*, and that is the whole question. It is not through `QemuHost`, which is
copy-only by contract (R5). It is that a memfd, though it has no filesystem path, is still an
**open descriptor in the shim's own process**, enumerable through `/proc/self/fd`. Option (A) is
therefore genuinely zero-new-QEMU-surface — but by a mechanism that has to be named, because the
obvious reading of "adopt it" (R1) is the one that cannot work.

### R5. ⚠ Sharpening, not a refutation: the QEMU adapter's blindness is by CONTRACT, not by omission

`QemuHost` (`crates/kayfabe-vmm-qemu/src/host.rs:175-327`) has thirteen methods; the only two
that touch guest memory are `read_region` (`:307`) and `write_region` (`:315`), and the contract
**forbids** widening them: *"A bounded memcpy against **this region's own backing**. It MUST NOT
be spelled as a general read-anywhere accessor"* — because the general entry point takes the
VMM's global lock, putting a foreign lock beneath one of our ranked locks. So option (B) (a new
`(RAMBlock fd, offset)` capability) is not "add an accessor to a trait that forgot one"; it is
adding a **new kind** of capability next to one whose narrowness is a deadlock argument.

---

## 1. ★★★ MEASURED — guest RAM crosses today with NO new QEMU surface

`[measured 2026-08-10, vh2, rev 954f926]`. `scripts/bench/boot_nvkvm.sh` gained
`NVKVM_RAM_BACKEND=memfd` (default empty, so every earlier capture's command line is unchanged):

```sh
-object memory-backend-memfd,id=ram0,size=2048M,share=on \
-machine q35,accel=kvm,memory-backend=ram0 -m 2048
```

Full paired transcript: `docs/reference/bench_evidence/guest_ram_memfd_954f926.out`.
Boot `memfdA` against the guest, then from **another process entirely**:

```
QEMU pid 66884
  fd=14 -> /memfd:memory-backend-memfd (deleted) size=2147483648 (2048.0 MiB)
  fd=22 -> /memfd:displaysurface        (deleted) size=1228800
OPENED from ANOTHER process, 2147483648 bytes
pages with content in first 256 MiB: 45358 / 65536
  Linux version    found=True
  nvkvm-guest      found=True
  systemd          found=True
  nvidia           found=True
```

and the **control**, boot `ctrlram`, same binary, flag absent:

```
=== memfd fds in the DEFAULT (-m 2048) boot ===
  fd=21 -> /memfd:displaysurface (deleted) size=1228800
=== shared (rw-s) mappings >= 1 GiB ===
(end of shared-mapping list)
```

⇒ **Without the flag guest RAM is anonymous and `MAP_PRIVATE`; with it, it is a 2 GiB `rw-s`
memfd holding live guest memory.** The device is unaffected either way — `memory plane realized
(bar0=0xfd000000 bar1=0xe0000000 bar2=0xf0000000)` is byte-identical across the pair, so the flag
is observationally neutral to everything already measured.

### 1.1 ⚠ Three traps this measurement walked into, all cheap and all silent

1. ★★ **QEMU names the memfd after the BACKEND TYPE, not after your `id=`.** The first probe
   searched for `memfd:ram0` — the id given on the command line — and found **nothing**, on a
   boot where guest RAM was sitting at fd 14 the whole time. The real name is
   `/memfd:memory-backend-memfd`. ⊘ A lookup keyed on the id returns empty and *looks like
   "the backend is not there"*, which is the exact failure class this project keeps re-deriving:
   the absence of a match is not the absence of the thing.
2. ★ **There are TWO memfds in the process**, and the decoy is created unconditionally:
   `displaysurface` (1.2 MiB) exists even in the control boot. A scan that takes the first
   `memfd` match gets the framebuffer. **Key on name AND size**, and assert exactly one match.
3. ★ **A substring match on `memfd` also matched the LOG FILE**, because the boot tag was
   `memfd1` and the log is `run_memfd1_qemu.log`. Harmless here only because the size assertion
   caught it. Match on the `/memfd:` prefix of a `readlink`, never on a substring of a path.

### 1.2 ⊘ What this does NOT establish

- Nothing was **mapped into the isolate**, and nothing was pinned into the GPU's VAS. This
  measures that a descriptor for guest RAM **exists and is openable**; the wire verb that carries
  it (R3) and the `OS_DESCRIPTOR` that pins it are not built.
- The probe opened the memfd via `/proc/<pid>/fd/14` from a **root** process on the host. The
  shim's own route is `/proc/self/fd` from *inside* QEMU, which is strictly easier — but it has
  not been written, so it is measured-as-possible, not measured-as-working.
- ⚠ It is a **2 GiB single region**. `bench_rebuild_notes.md` (2026-08-02) records boots at
  `-m 8G`; a machine with RAM split across e820 holes has more than one RAMBlock, and the
  single-fd result above does not generalise to that without re-measuring.

---

## 2. The ledger — built / built-but-unreached / designed-not-built / neither

| thing | status | citation |
|---|---|---|
| `SCM_RIGHTS` transport, both directions | **BUILT** | `linux-raw/src/scm_unsafe.rs:146`, `:344` |
| fd-OUT on one reply (`ExportBacking`→`Backing`) | **BUILT + reached** | `isolate.rs:414`, `child.rs:355-359` |
| fd-IN on any request | **NEITHER** — the child's request reader is not a `recvmsg` reader | `child.rs:331` |
| a wire verb naming guest RAM | **NEITHER** — 15 request tags, none fd-typed | `proto.rs:55-190` |
| `Vmm::export_ram`, all four impls | **BUILT-BUT-UNREACHED** — every caller is a test | `vmm/src/lib.rs:804` |
| QEMU: reach machine RAM as pointer/fd *in-adapter* | **NEITHER**, and by contract | `host.rs:307`, `:315` |
| QEMU: memfd for *kayfabe's own* reservations | **BUILT + reached** | `vmm-qemu/src/lib.rs:1229` |
| bench launched with a shareable RAM backend | ★ **BUILT this rung**, default off | `scripts/bench/boot_nvkvm.sh` |
| guest RAM openable as an fd | ★ **MEASURED** §1 | this page |
| `OS_DESCRIPTOR` alloc | **BUILT**, one caller, a probe | `rm.rs:1530` ← `rm.rs:3697` ← `rmladder.rs:1300` |
| FIXED-VA `map_dma` | **BUILT + reached in production** | `rm.rs:1283-1316`, used by `alloc_channel_at` |
| `placed_as_asked` assertion | **BUILT**, in probe and in production | `rm.rs:2005-2011`, `rm.rs:2894-2900` |
| pinning **guest** RAM into the host GPU VAS | **DESIGNED-NOT-BUILT** | — |

★ Two of the sketch's §4 steps are therefore already **built and hardware-validated**, just not
on this path: step 5's flag set (`NONCONTIG | LOCATION_PCI | COHERENCY_CACHED | MAPPING_NO_MAP`)
matches `crates/kayfabe-abi/src/bringup.rs:508-530` exactly, and step 6's *"assert placement,
refuse by name"* exists twice, one of them on the production channel path with the right reading
already written down: *"it reads RM's [OUT] `dmaOffset` rather than the value we asked for … A
downgraded placement must never be adopted."*

### 2.1 ★★ A latent type-confusion to fix BEFORE the crossing, not after

`export_ram` and `register_backing` push into the **same** `exports: Vec<OwnedFd>` and mint
tokens from the same index space, on **both** backends (`vmm-qemu/src/lib.rs:1413-1416` and
`:2302-2305`; `vmm-kvm/src/lib.rs:1246-1249` and `:1900-1903`). A `RamHandle.token` is therefore
a *valid* `HostRegion.id` and will `MAP_FIXED` guest RAM into a guest window via `map_guest`
(`vmm-qemu/src/lib.rs:2090-2093`). This is inert **only because `export_ram` has no callers** —
i.e. the first production caller is what arms it. Mint the two token spaces separately.

---

## 3. What is still open, and what is now decidable

- ⊘ **OPEN, owner:** launch flag (A) vs adapter capability (B). §1 moves this: (A) is now
  **measured working with zero new QEMU surface**, so it is not merely "faster" — it is free. (B)
  remains the shipping answer for a VMM we do not control the command line of, and R5 says it is
  a new *kind* of capability rather than a widened accessor.
- ⊘ **OPEN, owner:** §5 of the sketch — how much of guest RAM one isolate may reach. Unchanged
  by this rung, and §1 sharpens it: what exists is **one fd for the whole 2 GiB block**, so
  shape (ii) ("per-run descriptors") has no mechanism behind it today and (i)/(iii) are the real
  choice.
- ★ **NOT open, and #233 says so:** two prerequisites are requirements of passthrough rather than
  casualties of it — closing the #14 ring gate (`host_published`/`VasGate` are built but the shell
  passes an **empty** working set, so the gate is vacuous, `device.rs:1806-1817`), and a
  host-private VA reservation in the isolate's VAS (R26n proved RM enforces occupancy).

⊘ **Still forbidden, and this rung does not touch any of it:** no ring or pushbuffer parsing
(#231), no semaphore writer (that is the C's forgery branch), no blocking wait on the vCPU
thread.

---

## 4. ★★★ What LANDED (2026-08-10, task #238) — and lead with the refutations

`[built + unit-measured]` on master. Every claim below is a code/test citation, not a bench run;
§4.4 says plainly what is still unmeasured.

### R6. ⊘⊘ The fd does NOT ride a request — and building fd-IN for it would have been WRONG

§0's R3 is correct that the child's request loop has no `recvmsg` reader, and the task framing
made the natural inference: *the guest-RAM `memfd` crosses on a new fd-bearing request*. It does
not, and the reason is in the companion page's own §2: the descriptor is handed to the isolate
**at a fixed, known number**, *precisely so the seccomp filter installed afterwards can hardcode
that number*. A descriptor arriving per-request has **no fixed number to hardcode**.

⇒ It crosses at **spawn**, through `kayfabe_linux_raw::FdGrant` — the mechanism that already
delivers the control socket, the worker sockets and the park witness — on
`isolate::GUEST_RAM_FD` (a new **6**; `WORKER_FD_BASE` moved to **7**, and the compile-time
descriptor-contract assertion grew an arm so a future edit that collides two grants fails the
build). The wire verb therefore moves **scalars only**.

★ And that is not merely "less work": it is what makes the eventual `SECCOMP_RET_USER_NOTIF`
match trivial rather than a policy decision — every argument of the `mmap` the child is about to
issue is already in the frame that ordered it.

⊘ So the fd-IN gap is **real and off this path**. It stays open, correctly, in §2's ledger.

### R7. ⊘ NO host virtual address crosses the boundary, and the design sketch assumed one would

`mode2_isolate_memory_boundary.md` §3's "Matching" paragraph wants `addr` in the authorization
match, on the ground that *"the VMM dictates it anyway, for VA identity"*. It cannot today, and
the obstacle is a boundary rather than a missing feature: `MappedRegion::addr_at` is
`pub(crate)` in `kayfabe-linux-raw`, deliberately — the one consumer that needs a host address
(`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`, which hands RM an address it then `pin_user_pages`-walks)
is served by patching the value into the ioctl argument **inside** that crate and scrubbing it
back out. `MAP_FIXED_NOREPLACE` likewise appears nowhere in the crate, by policy.

⇒ A mapping is named by a **`HostHandle`** instead. That is not a stand-in for the address: it
buys the cross-isolate check for free, because `Worker::unmap_guest_ram`'s foreign-handle gate
already refuses a name minted by one isolate presented on another — and guest RAM is precisely
the resource whose cross-isolate reach is a real escalation rather than untidiness.

★ And the authorization is **complete without it**, which is the part worth stating rather than
apologising for. §3's circularity rule is about *which guest memory* an isolate may reach. The
host VA says where in the isolate's own address space the pages land, and authorizes nothing —
an isolate that picks its own host VA still cannot pick which guest bytes are there. So the
host VA tightens the *seccomp match* later; it does not plug a gap now.

### R8. ⊘ §2.1's fix is NOT "two vectors" — two zero-based index spaces collide at every index

§2.1 called for minting the two token spaces separately, and the obvious reading (a second
`Vec`) would have left `RamHandle.token == 0` and `HostRegion.id == 0` still interchangeable.
What landed puts the separation **in the value**: `kayfabe_vmm::RAM_EXPORT_TOKEN_TAG` (bit 63)
plus `RAM_TOKEN_AS_A_BACKING`, defined once so three backends cannot drift, with `map_guest`
refusing a tagged id **by name and before the table lookup**. `MockVmm` refuses it too — a mock
that accepted it would be the more dangerous half, since the confusion would then typecheck and
run green in every unit test.

The regression test carries its own bite: strip the tag and the **same index** maps, which is
what proves the two spaces really did collide there rather than the id merely being out of range.

### R9. ⚠ The fd-carrying reader had NO interrupt rule, and it is worse there than elsewhere

Found while building this, latent until now. `proto::read_exact_or_eof` states a
position-dependent rule — at a frame boundary `EINTR` is a landed cancel and must be reported;
mid-frame it is a stray signal and must be retried — and `fdcross::read_frame_with_fds` had
neither half. The two readers share no code, so the rule was true of one and false of the other
with nothing saying so.

⚠ It is **strictly worse** on the fd reader: the descriptors arrive with the frame's *length
word* and are already adopted by the time the body read blocks, so a mid-frame abandon closes
real descriptors **and** strands the frame that named them. Fixed with `RawError::is_interrupted`
(in the raw crate, where errno numbers belong, matched on the errno and never on the call
string), and bite-checked.

### 4.1 The shape, as built

| piece | where |
|---|---|
| `GuestRamGrant` — VMM-originated `(offset, len, prot)`; sole constructor `originated_by_the_vmm` | `crates/kayfabe-isolate/src/lib.rs` |
| `GuestRamMapped` — a `HostHandle`, deliberately not an address (R7) | same |
| `RmBackend::map_guest_ram` / `unmap_guest_ram`; `Worker` wrappers with R1 + the foreign-handle gate | same |
| `RmError::GuestRamUnavailable` — a **deployment** fact, named on the wire | same, + `proto.rs` |
| `GuestRamPlane` — the one `mmap` of guest memory in the isolate; owns the mappings, so isolate death releases them | `crates/kayfabe-isolate-host/src/guestram.rs` |
| `GUEST_RAM_NAME_TAG` — puts a mapping's name beyond RM's 32 bits, so the existing `narrow` gate refuses it as an object handle | same |
| wire verbs 16/17, scalars only, `max_fds = 0` | `crates/kayfabe-isolate-host/src/proto.rs` |
| the spawn-time grant | `crates/kayfabe-isolate-host/src/isolate.rs` |
| `unsafe impl Send for MappedRegion` — and **deliberately not `Sync`**; the argument is written at the impl | `crates/kayfabe-linux-raw/src/mapping_unsafe.rs` |

### 4.2 ★★★ The one test that could not have been written the obvious way

`writes_the_vmm_makes_after_the_mapping_are_visible_to_the_isolate`
(`crates/kayfabe-isolate-host/tests/guest_ram.rs`) drives a **real spawned child** over a real
socket with a real `memfd` on the grant number. The obvious version — write, map, read back —
would pass identically against a design that **copied** the range at map time, which is exactly
§6's refuted alternative. So the ordering *is* the assertion: the isolate maps first, the VMM
writes second, the isolate's view changes. Only a shared mapping can answer.

### 4.3 ⚠ An instrument this rung got wrong, and corrected

The isolate-death test first counted `/proc/self/fd` before and after. It reported **12 against
34** — because the test binary runs its threads in parallel and the count was reading every
other test's descriptors. ⊘ Serialising the file would have been the wrong fix: the count could
never have witnessed the claim, since the mappings being released are in **another process's**
address space and vanish with it. The child-side property is now asserted where it is
observable — `guestram`'s own unit tests, against a plane whose table is in reach — and the
process-side test asserts the converse it *can* see: guest RAM outlives every isolate that saw
it, intact and writable.

### 4.4 ⊘ What is still NOT measured

- **Nothing has run on the bench.** Everything above is unit-and-integration measured on a box
  with no GPU. The `NVKVM_RAM_BACKEND=memfd` flag is landed and §1-measured, but **no VMM code
  calls `with_guest_ram` yet** — the QEMU shim has no route to the hypervisor's own guest-RAM
  descriptor, which is §3's still-open (A)-vs-(B) question and the next thing on this path.
- **No `OS_DESCRIPTOR` has been taken over a guest page.** `GuestRamPlane::with_region` exists
  so the alloc can name one, and has no caller.
- **The enforcement layer is not built** — no fd pinning, no filter, no notify loop, no `munmap`
  confirmation. That is §5's deliberate split: the shape now, the enforcement behind it.

### 4.5 ★ The launch flag, re-run on a SECOND physical box

`[measured 2026-08-10, vh (vast 47029542), shim rev 346921b]` —
`docs/reference/bench_evidence/guest_ram_memfd_vh_346921b.out`, boot logs
`traces/guest_boots/run_vhmemfd_qemu.log` and `run_vhctrl_qemu.log`.

§1 was measured on `vh2`. The paired boot reproduces on `vh`: with the flag, one 2 GiB
`/memfd:memory-backend-memfd` openable from another process and holding live guest memory
(`Linux version`, `systemd`, `nvidia` all present); without it, only the 1.2 MiB
`displaysurface` decoy and **zero** candidates. The `memory plane realized` line is
**byte-identical** across the pair — observational neutrality confirmed on a second machine
rather than restated from the first.

★★ And one of §1.1's traps got a second data point instead of a second mention: the fd
**number moved**, 14 on `vh2` and 15 on `vh`. A probe keyed on the number, or on "the first
memfd", would have been right on one box and wrong on the other. The probe keys on the
`readlink`'s `/memfd:` prefix **and** the size, and asserts exactly one candidate.

⊘ **What this boot does NOT show, stated because every signal says otherwise.** The bench
binary is `346921b`, which predates everything in §4 — and no VMM code calls
`with_guest_ram` at *any* revision. So this is a measurement of the **launch flag**, and it
is not a boot of the shape that landed. A reader who sees "boot evidence, 2026-08-10" beside
§4 and infers the crossing ran end to end would be wrong.

---

## 5. ★★★ THE ROUTE (2026-08-10, task #238 cont.) — the VMM now calls `with_guest_ram`

`[built, unit-measured, and BOOTED on `vh`]`. §4.4's first bullet was *"no VMM code calls
`with_guest_ram` yet — the QEMU shim has no route to the hypervisor's own guest-RAM
descriptor"*. That route now exists, and it is the whole of this rung. It is §3's option
**(A)** — a `/proc/self/fd` census inside the hypervisor's own process — serving a
**(B)**-shaped interface: what crosses is a descriptor plus an extent.

### R10. ⊘ The falsifier I was handed does not exist — `221/479` is not a fraction

The task's falsifier was *"the progress fraction. `221/479` today."* There is no metric in
this repository with denominator 479: `grep -rn 479` over every tracked file returns `ogkm`
line numbers, one `rpctrace` session's record count, and nothing else.

**221 is a target count, not a numerator.** It is §4's own evidence line — *"`cargo test
--workspace --no-fail-fast`: 221 targets"* — the number of test binaries plus doc-test
targets cargo compiles. It moves when a file is added and says nothing about progress;
placed over a denominator it acquires a meaning it does not have.

⇒ The numbers this rung actually moves, all measured on the dev box at `d7af8fb` → `7b0694f`:

| | before | after |
|---|---|---|
| tests passed / failed (`--workspace --no-fail-fast`) | **2586 / 0** | **2595 / 0** |
| test binaries + doc-test targets | 219 | 219 |
| `scripts/ci_gates.sh` failures | 3 | **the same 3** (kayfabe-device bridge-exclusivity, ogkm-version-tag, claim-ledger) |
| claim sites unattributed / conflated / bare-hw | 492 / 75 / 19 | 492 / 75 / 19 |
| `kayfabe-linux-raw` audited relaxations | 91 | **91** — see R11 |

⊘ And *"tests passed"* is not a progress fraction either; it is a floor, saying only that
nothing broke. The honest statement of where the crossing stands is §5.4's ledger.

### R11. ★★★ The descriptor costs ZERO new `unsafe_code` relaxations, and that is not luck

The obvious implementation is the one this repository already has:
`KvmVm::discover_in_this_process` reads a number out of `/proc/self/fd` and `dup`s it
through `BorrowedFd::borrow_raw` — a relaxation whose own `SAFETY:` comment openly states an
obligation it cannot discharge (that no other thread closed the number in between). Copying
that shape here would have cost at least two more.

It is not needed, because the two are **different kinds of object**. A KVM handle is an
anonymous inode — the ratchet's own note records `/proc/self/fd/N` answering `ENXIO` on
re-open, measured. A `memfd` is a real shmem file and **re-opens**; §1's measurement already
relied on exactly that, from another process. So the census is `std::fs::OpenOptions`,
`read_link` and `metadata`, and `kayfabe-linux-raw`'s audited surface is unchanged at 91.

★ It is also *stronger* than a `dup`: a re-open gets its own open-file description, so the
descriptor granted to an isolate does not share a file offset with the hypervisor's own.

### R12. ⊘⊘ "Re-derive every property from the descriptor you own" is NOT sufficient

`[measured 2026-08-10, `cargo test -p kayfabe-linux-raw --lib procfd`]`, and it is the
rung's real finding.

A census that opens the descriptors it enumerates **occupies the numbers it is about to
visit**. `open` returns the lowest free descriptor; the directory handle releases one when
the listing is drained; the first re-open takes that number back; the loop then visits it
and finds — its own re-open. `[measured 2026-08-10, `cargo test -p kayfabe-linux-raw --lib
procfd`]`: a process holding **two** `memfd`s of one name reported **three**.

⊘ The discipline this module's own header argues for does not catch it. Re-deriving the
name, size and inode from the descriptor we now own **confirms every one of them**, because
the recycled number really is a `memfd` of exactly the right name. *A verification cannot
tell an object from itself.*

⊘⊘ And it was not cosmetic. In production the duplicate carries guest RAM's name, inode and
shared-mapped state, so the selector would have answered `MemfdRefusal::Ambiguous` —
**refusing the one boot the module exists to serve**.

★ My first explanation was wrong. I diagnosed *"`read_dir` is lazy, so the loop enumerates
its own new descriptors"*, drained the listing before opening anything, and the count did
not change; a debug print of `/proc/self/fd` gave the mechanism in one line. Two fixes,
closing two different things:

1. numbers this census itself opened are skipped — the instrument cannot observe itself;
2. the census is over **blocks, not descriptors**, joined on the inode. A hypervisor is
   entitled to hold two descriptors on one `memfd`, and that must not read as two blocks.
   ★★ **That is not hypothetical: it is what this rung's own code produces.** §5.5's boot
   shows QEMU holding `/memfd:memory-backend-memfd` at **fd 15 and fd 25, same inode** —
   15 is the hypervisor's, 25 is the shim's adopted re-open. A second `nvkvm-gpu` device
   taking a census after the first would see both. Two *different* `memfd`s of one name have
   different inodes and stay ambiguous, which is the fact worth refusing.

### 5.1 What landed

| piece | where |
|---|---|
| `MemfdCensus` / `MemfdCandidate` / `MemfdRefusal` — the property-keyed probe | `crates/kayfabe-linux-raw/src/procfd.rs` |
| `SharedRam::create_named` — the creation name as a parameter | `crates/kayfabe-linux-raw/src/host_fd_unsafe.rs` |
| `GUEST_RAM_ENV` (`KAYFABE_GUEST_RAM`), `GuestRamSource`, `guest_ram_source_from` | `crates/kayfabe-qemu-raw/src/shim.rs` |
| `QEMU_MACHINE_RAM_MEMFD` — the hypervisor-specific half, kept out of the raw crate | same |
| `guest_ram_is_reachable_on` — the plane × source cross-check | same |
| `with_guest_ram` — census → select → `HostIsolateFactory::with_guest_ram` | same |
| `isolate_factory(plane, guest_ram)` — one more argument, read once in `object_policy` | same |

### 5.2 ★★ The four properties the probe keys on, and the trap each closes

Each is §1.1's or §4.5's, made executable rather than restated:

1. **the `/memfd:` prefix of the whole `readlink`**, never a substring — trap 3, where a
   boot tagged `memfd1` made the *log file* match;
2. **the name after it, exactly** — trap 1, where a lookup keyed on the command line's
   `id=ram0` found nothing on a boot with guest RAM open the whole time, and read as *"the
   backend is not there"*;
3. **mapped `rw-s` in this process**, joined on the **inode** — trap 2. §5.5 found a third
   decoy §1.1 never saw, and it is **ours**: `kayfabe-isolate`, the embedded isolate image's
   own `memfd`, 709 280 bytes, `shared_mapped=false`. It is excluded on two independent
   properties, which is the margin this list is supposed to have;
4. **exactly one match** — §4.5, where the descriptor **number** moved (14 on `vh2`, 15 on
   `vh`), so every position-based tie-break was right on one bench and wrong on the other.
   `listed_as` is printed to the log and decides nothing.

### 5.3 ⊘ Two independent facts are required, and the second is NOT the launch flag

The cheap shape is *"if guest RAM happens to be a shared `memfd`, use it"* — no new
variable. ⊘ That makes the boundary **a coincidence of how the operator started the VM**. A
hypervisor may carry `share=on` for vhost-user, virtiofs or `ivshmem`, and none of those is
a decision to let a GPU isolate map the guest's memory.
`HostIsolateFactory::with_guest_ram` already states the rule the other way round: *"a
factory that defaulted to granting guest RAM would be granting it on every deployment that
never asked, and the grant is the whole boundary."*

⇒ The launch flag makes the descriptor **exist**; `KAYFABE_GUEST_RAM=memfd` is the operator
**asking for it to cross**. Both, or nothing crosses.

★ And the refusal is **at startup**, not at the first doorbell. `RmError::GuestRamUnavailable`
is right at the seam and wrong for an operator: it would arrive twenty seconds into a boot
as one more refusal in a log full of them. ★ On the failing path the **whole census is
printed first** — trap 1 is precisely a probe that found nothing and read as *"the thing is
not there"*.

⊘ `KAYFABE_GUEST_RAM=memfd` on the `stillborn` plane is refused by name too: nobody can hold
the grant, so that run would be indistinguishable from its own negative control.

### 5.4 The ledger, updated — and what is STILL not done

| thing | status |
|---|---|
| bench launched with a shareable RAM backend | **BUILT**, §1, default off |
| guest RAM openable as an fd | **MEASURED**, §1 + §4.5, two boxes |
| the isolate-side shape (`GuestRamGrant`, `GuestRamPlane`, verbs 16/17) | **BUILT**, §4 |
| ★ a VMM caller of `with_guest_ram` | ★ **BUILT this rung** |
| ★ the shim reaching the hypervisor's own descriptor | ★ **BUILT + BOOTED**, §5.5 |
| ★ the spawn-time grant on a live boot (`GUEST_RAM_FD`=6, `WORKER_FD_BASE`=7) | ★★ **MEASURED IN THE CHILD**, §5.5 `w224d` |
| a `GuestRamGrant` ever constructed in production | **NEITHER** — nothing orders a mapping |
| `OS_DESCRIPTOR` over a guest page | **NEITHER** — `GuestRamPlane::with_region` still has no caller |
| GPA → offset for a machine with more than one RAM run | ★ **DECIDED AND MEASURED** — §5.7: STATED by the topology listener, joined on `(st_dev, st_ino)`, 4 runs on the bench; refused by name outside a stated run |
| the enforcement layer (fd pinning, filter, notify, `munmap` confirmation) | **NEITHER**, deliberately behind the shape |

⊘ **Nothing has been mapped into an isolate and nothing has been pinned into a GPU VAS.**
This rung moves a descriptor and an extent to the place that can grant them. The first
`mmap` of guest memory still needs a `GuestRamGrant`, and the only thing that can build one
is a caller that knows *which* guest bytes it wants — step 2, and it is not here.

### 5.5 ★★★ BOOTED — `vh` (vast 47029542, RTX 3060 GA106), shim rev `7b0694f`

Five runs, evidence committed under `traces/guest_boots/`. ★ The revision is the one
**stamped inside the binary** (`strings … | grep kayfabe-rev` →
`7b0694f5e9325f9a792d18e2553b91a382c1c258`), not the worktree's — the 2026-08-01 post-mortem
is what that instrument exists for.

**`w224a` — ARMED** (`NVKVM_RAM_BACKEND=memfd KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd`),
`run_w224a_{qemu,dmesg,probe}.log`. The census printed three `memfd`s and selected one:

```
kayfabe: memfd census — name="memory-backend-memfd" bytes=2147483648 shared_mapped=true (listed at fd 15, …)
kayfabe: memfd census — name="displaysurface"       bytes=1228800    shared_mapped=true (listed at fd 23, …)
kayfabe: memfd census — name="kayfabe-isolate"      bytes=709280     shared_mapped=false (listed at fd 24, …)
kayfabe: ★★★ GUEST-RAM CROSSING ARMED — adopted the hypervisor's memory-backend-memfd block, 2147483648 bytes.
```

★ **fd 15, exactly as §4.5 measured on this box** — and the probe ignored it. ★★ And the
**third** `memfd` is new information: `kayfabe-isolate` is *our own* embedded image, so the
decoy population is not the two §1.1 knew about, and one of them is created by this project.

The guest booted normally: `SMI_RC=0`, 34 `dmesg` lines / 31 `NVRM`, `isolates: 1
materialized, 1 live, 0 refusing`, `doorbells: 2 arrived, 2 served, 0 REFUSED`.

**`w224c` — the negative control**, same binary, same script, `KAYFABE_GUEST_RAM` **unset**.
No census line, no adopt line, and the device path is unchanged. Normalised for timestamps
and with the census lines removed, `run_w224a_qemu.log` and `run_w224c_qemu.log` are
**213 lines each with three differing lines**, all of them ordinary run-to-run variance:
`registers: 4406 reads / 188982 writes` vs `4407 / 188990`, `interrupt tree: 626` vs `634`
register accesses, and the CPU-CE semaphore's guest-chosen target address. ⇒ **arming the
crossing is observationally neutral to everything else this pair measures**
`[2026-08-10, `vh`, rev 7b0694f]`, which is exactly what must be true while nothing is
mapped.

**`w224b` — the refusal**, `KAYFABE_GUEST_RAM=memfd` with the launch flag **absent**,
`run_w224b_qemu.log`. QEMU exits `rc=1` **at device realize**, having first printed the two
`memfd`s it *did* see, and then:

```
nvkvm: the register plane refused to build (3): KAYFABE_GUEST_RAM=memfd, and no shared-mapped
`memfd` named `memory-backend-memfd` is open in this process — see the census above for what IS. …
```

**`w224m` — the layout**, `run_w224m_mtree.log`: QEMU's own `info mtree -f` flat view, which
is the VMM **stating** the map rather than us deriving it. For `-m 2048` on q35 there are
**12 `ram0` ranges, 0 of them non-identity** (`GPA == offset` for every one, including the
legacy `rom` aliases at `0xc0000`–`0xfffff`), and **no range at or above 4 GiB**. See §5.6
for why that settles the bench and not the mechanism.

**`w224d` — ★★★ THE SANDBOXED CHILD REALLY HOLDS IT**, `run_w224d_isolatefd.log`, taken by
`scripts/bench/probe_guest_ram_holders.sh` while the guest driver ran. Every `/proc/*/fd`
in the host, joined on guest RAM's **inode**:

```
guest-RAM inode=757510
  pid=1056047 comm=qemu-system-x86  fd=15 exe=…/qemu-system-x86_64
  pid=1056047 comm=qemu-system-x86  fd=25 exe=…/qemu-system-x86_64
  pid=1056143 comm=memfd:kayfabe-i  fd=6  exe=/memfd:kayfabe-isolate (deleted)
```

⇒ The isolate child holds guest RAM at **fd 6 — `isolate::GUEST_RAM_FD` exactly** — on a
real boot, which is the half `tests/guest_ram.rs` can only assert on a GPU-less box. And the
two QEMU-side descriptors on one inode are R12's second mechanism, measured in production:
15 is the hypervisor's, 25 is the shim's adopted re-open.

⚠ **Three prior attempts failed, and both selectors I reached for were wrong** — worth
recording, because they are the *same* trap this repository already carries for
`qemu-system-x86_64`:

- **`comm` is `memfd:kayfabe-i`.** The isolate is `execveat`-ed from a `memfd`, so the kernel
  derives `comm` from the descriptor's own name — **including the `memfd:` prefix** — and
  truncates it at 15 characters. `pgrep -x kayfabe-isolate` can therefore **never match**,
  which is `/proc/PID/comm` truncating to 15 for the second time in this project.
- **It is not a direct child of QEMU.** `ps -eo ppid` finds nothing under the QEMU pid; the
  namespaced spawn reparents it.

★ The **inode** was the only selector that worked, and it is the same lesson as the census
itself: name and position are guesses, identity is a fact.

⊘ **What these boots do NOT show** `[2026-08-10, `vh`, rev 7b0694f]`. No guest byte was
mapped into an isolate and none was pinned into a GPU VAS — the ledger above says so, and
`guest-RAM refusals 0` in the register census is *zero because nothing asked*, not because
something succeeded. All five runs establish is that the descriptor is **in the isolate's
hands**; the first `mmap` still waits on a `GuestRamGrant` that nothing constructs.

### 5.6 ⊘ The GPA→offset map: what `w224m` shows, and what it does not decide

`OS_DESCRIPTOR` describes one contiguous host VA range while guest GPAs need not be
contiguous, so step 2 needs a map from a guest-physical run to an offset in this descriptor.
§5.5's `w224m` `[2026-08-10, `vh`]` settles it for the bench: identity, one run, nothing
above 4 GiB.

⊘ **That is a property of one command line, not of the mechanism.**
`bench_rebuild_notes.md` records boots at `-m 8G`, where RAM is split across the 4 GiB PCI
hole and the high run's offset is the size of the low one. The census cannot answer it
either: it yields a descriptor and its **extent**, and an extent is not a layout. Deriving
one from the machine type would be re-deriving a VMM fact — the exact thing `GuestRamPlane`
refuses when it takes the VMM's length rather than an `lseek`.

⇒ Step 2 owes either the VMM's own statement of the layout, or a **refusal by name** for
every GPA outside the single run this deployment is known to have. ⊘ It must not assume
identity because the one command line `w224m` `[2026-08-10]` covers happens to have it.

---

## 5.7 ★★★ STEP 2 LANDED — the layout is **STATED**, and both easy instants report it EMPTY

`[measured 2026-08-10, `vh`, real GA106, rev e1e57f6 — boots `w225a`, `w225f`, `w225g`]`

§5.6 owed "the VMM's own statement of the layout, or a refusal by name". Both are now in the
tree, and the mechanism is the one the hypervisor already had: the **topology listener**.
QEMU calls `region_add` for every section of its flat view, carrying the section's
guest-physical base, its length and its offset within its region. That is the statement. What
was missing was any way to tell *which* of those sections is the block we adopted.

### 5.7.1 The join is on the BLOCK, and it needed four new facts on the wire

`KayfabeSection` grew from five unclassified facts to nine (`KAYFABE_SHIM_ABI` 37 → 38):
`fd_backed`, `backing_dev`, `backing_ino`, `file_offset_of_region`. The C reads them from
`memory_region_get_fd(sec->mr)` + `fstat`, and `mr->ram_block->fd_offset` as a field — the
same justification the file already carries for `mr->rom_device`: no public accessor answers
it, and the alternative is an assumption with nothing to catch it.

⊘ **`mr` cannot do this job**, and that is why the fields exist. `mr` is a region object's
address: unique to one process's lifetime, meaningless to anything that has to `mmap` the
bytes, and not comparable against the descriptor the census adopted. The join is on
`(st_dev, st_ino)` — the same discipline `procfd.rs` was forced onto one layer down when
descriptor *numbers* turned out to move between two physical benches.

⊘ **`fd_backed == 0` is UNMEASURED, not "no backing".** A section that reports no descriptor
states nothing, so every address in it is refused rather than attributed to whichever block
the caller asked about.

### 5.7.2 What the hypervisor actually stated — `w225f`

```
GUEST-RAM CROSSING ARMED — memory-backend-memfd, 2147483648 bytes, dev=1 ino=6739
GUEST-RAM LAYOUT AT END OF RUN — dev=1 ino=6739: 4 contiguous run(s) totalling 2147135488 bytes
  Section funnel: 76 reported -> 10 classified RAM -> 8 carried a backing file, 8 later withdrawn
  gpa 0x0000000000000000..0x00000000000a0000 -> file offset 0x0 (655360 bytes)
  gpa 0x00000000000cb000..0x00000000000ce000 -> file offset 0xcb000 (12288 bytes)
  gpa 0x00000000000e8000..0x00000000000f0000 -> file offset 0xe8000 (32768 bytes)
  gpa 0x0000000000100000..0x0000000080000000 -> file offset 0x100000 (2146435072 bytes)
```

★ The four runs total **2 147 135 488** of the descriptor's **2 147 483 648** bytes. The
348 160-byte difference is exactly the legacy/SMRAM holes at `0xa0000`, `0xce000` and
`0xf0000` — i.e. the layout is a *real* PC memory map and not a single identity range, on the
very command line §5.6 said was uninteresting. **8 reported sections coalesced to 4 runs**,
which is the shape step 3 wants: one descriptor per contiguous run.

⊘ Every run is identity **here**, and the report says so with the word "OBSERVATION" attached.
Nothing in `layout.rs` branches on `is_identity`; `resolve` answers from the stated offset in
all cases, and `tests/stated_layout.rs` asserts the `-m 8G` split — where identity is wrong by
one PCI hole, silently — resolves correctly.

### 5.7.3 ★★★★ THE FINDING: the layout has NO instant at which it is both live and complete

This cost four boots and it is the part worth carrying forward.

The first armed boot, `w225a`, reported **0 runs**. The mechanism was correct; the *instant*
was not. Then the fix moved the report to the exit notifier — and it reported **0 runs
again**, for an unrelated reason. Both zeros are real, and they have nothing in common:

| instant | what the live table says | why |
|---|---|---|
| memory-plane attach | `0 reported -> 0 RAM -> 0 backed` | the listener is registered on the device's **bus-master address space**, whose flat view is empty until the guest enables bus mastering — long after attach |
| exit notifier | `76 reported -> 10 RAM -> 8 backed, 8 later withdrawn` | teardown replays `region_del` over every range, so the live table is empty **again** |

★ This is `a_correct_capture_can_answer_the_wrong_question`, twice in one rung: a working
instrument, correct numbers, and a question about a **lifetime** answered by sampling an
instant. ⊘ And the first attempt at the second instant was *worse* than the first, because it
looked like progress — the report moved and the number did not.

⇒ Three consequences, all in the tree:

1. **`resolve` reads the LIVE table; the report reads an `ever` table** that is added to and
   never withdrawn. ⊘ They must not become one: answering from `ever` would serve ranges the
   hypervisor had stopped backing — a stale mapping with a plausible offset, which is the
   whole class this module refuses. `a_withdrawn_run_leaves_the_evidence_and_leaves_the_resolver`
   is the test that holds them apart.
2. **The report names its instant** (`AT MEMORY PLANE ATTACH` / `AT END OF RUN`) and says in
   the line itself that an empty reading is a statement about *when* it was taken.
3. **A zero is diagnosed, never just printed.** The three-stage funnel plus the withdrawn
   count turns "0 runs" into one of five named sentences — nothing reported yet *and expected
   at this instant*; nothing reported all run (**ordering**); nothing classified RAM
   (**classification**); nothing carried a backing (**the shim's `fd_backed`**); runs stated
   for other files (**the join**), which then prints every `(dev, ino)` that did state one.

★★ Stage (5) is the one that earned its keep. `w225c`/`w225d` reported `76 -> 10 -> 8` with
zero runs for our block and named it a **JOIN fault** — which was wrong in an instructive way:
the join was fine, the rows had simply been withdrawn before the report ran. The funnel was
one counter short, and the missing counter was `forgotten`. ⇒ **A funnel that only counts
arrivals cannot see removals**, and an empty table at the end of a run is far more likely to
be a teardown than a mismatch.

### 5.7.4 The control, and what step 2 still does NOT show

`w225g` — same binary, same script, `KAYFABE_GUEST_RAM` **unset** — prints **zero** layout
lines and the same `doorbells: 2 arrived, 2 served, 0 REFUSED`. The instrument is
observationally neutral, so the armed run stays comparable to its own control.

⊘ **No guest byte has been mapped or pinned.** `GuestRamPlane::honour` still has no
production caller and no `GuestRamGrant` is constructed anywhere. Step 2 delivers a resolver
and its evidence; step 3 is `OS_DESCRIPTOR` + `map_dma` **fixed at the guest VA**, one
descriptor per contiguous run at the GPA-ordered layout above, asserting `placed_as_asked`
per run.

⚠ One scope limit worth stating rather than discovering: the listener sits on the device's
**bus-master** address space, not on system memory. For a machine with no vIOMMU those hold
the same RAM, and every run above is real. On a machine with a vIOMMU they are **not** the
same space, and this map would then describe what the device may DMA to rather than the
guest's physical map. That is the correct space for a DMA descriptor and possibly the wrong
one for a CPU mapping — undecided, and not decided by these boots.
