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

### 4.5 ★ Re-measured on a SECOND physical box

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
