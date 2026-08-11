# §5.13 — ADOPTING THE GUEST'S **USERD** (`w233`, rung `R32`)

`guest_ring_adoption.md` (§5.11) made a host channel name the guest's **queue**. This one
asks the other half of the same question and answers it on hardware: may the channel also
name the guest's **cursors** — the 512-byte USERD block holding `GP_GET` and `GP_PUT`?

**MEASURED**, RTX 3060 GA106 / driver 580.159.04, bench `vh2`, 2026-08-11,
source revision `927378452c9507bb3503a1a8221829a0cb74bc2d`, host Xid clean:
`traces/real_ga106/rmladder_r32_guest_userd_real_ga106.txt`.

⊘ Nothing in this rung schedules a channel, rings a doorbell or writes `GP_PUT`. No guest
was booted. **`cup2` is irrelevant to it in both directions**, and saying so is part of the
result.

---

## 1. ★★★ Lead with what was refuted — and the first thing refuted is the instrument

The rung was commissioned as: *pass `hUserdMemory[0]` and observe whether the alloc returns
`NV_OK`*, with

| commissioned arm | commissioned reading |
|---|---|
| `NV_OK` | the internal CPU touch is fine; the client CPU map is a pure choice |
| failure | *"it reports through `NV_ASSERT_OK` at `kernel_fifo_gm107.c:787`"* |

**[SOURCED, `ogkm-580.159.04` — the version the bench actually runs] It cannot report
there, and both arms print `NV_OK`.**

- `kfifoSetupUserD_GM107` is `void`
  (`src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:795-808`);
- it wraps `memmgrMemSet` in `NV_ASSERT_OK`, whose definition carries the comment
  `/* no other action */` (`src/nvidia/inc/libraries/utils/nvassert.h:467-473`) — it prints
  and continues;
- its **one** call site discards the void return and falls straight into `return NV_OK`
  (`src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2342-2358`).

⇒ the alloc status answers the **ingest** question and is **silent** on the **residual** —
which is the one thing the brief itself marked `[INFERRED, not measured]`. A rung built on
it would have inferred from a silent instrument, which is that brief's own first trap.

★ **So `R32` measures the bytes.** It poisons the block through pages this process owns,
lets RM build the channel, and reads them back.

### 1.1 ⊘ And the fallback arm is deleted on source, before a hardware run is spent on it

The brief offers `NVOS32_DESCRIPTOR_TYPE_OS_FILE_HANDLE` as *"the named escape"*, on the
strength of `os_desc_mem.c:137-141` setting `MEMDESC_FLAGS_ALLOW_EXT_SYSMEM_USER_CPU_MAPPING`
only for that type. That flag reading is **correct**. The escape is **not available**:

- the escape this port uses — `NV_ESC_RM_ALLOC_MEMORY` with class
  `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` — **hard-codes**
  `descriptorType = NVOS32_DESCRIPTOR_TYPE_VIRTUAL_ADDRESS`
  (`src/nvidia/arch/nvalloc/unix/src/escape.c:274`, `RmAllocOsDescriptor`);
- `osCreateMemFromOsDescriptor` refuses `VIRTUAL_ADDRESS` outright
  (`…/osmemdesc.c:133-136`), because the kernel half has already **rewritten** the type to
  `NVOS32_DESCRIPTOR_TYPE_OS_PAGE_ARRAY` after `os_lock_user_pages`
  (`…/escape.c:164-167`).
  ⇒ ★ **our descriptors are `OS_PAGE_ARRAY`**, not the `VIRTUAL_ADDRESS` that
  `HostRmConnection::alloc_os_descriptor`'s own doc comment names. The name in the ioctl is
  a request the driver overwrites.
- `OS_FILE_HANDLE` is not a generic file handle at all — it is a **dma-buf import**:
  `nv_dma_import_from_fd(dma_dev, fd, …)` (`…/osmemdesc.c`,
  `osCreateOsDescriptorFromFileHandle`). A `memfd` is not a dma-buf exporter, and guest RAM
  reaches this process as a `memfd`. Its only in-tree user is nvidia-modeset
  (`src/nvidia-modeset/src/nvkms-surface.c:537`), a kernel client.

⇒ **the `0x56` on a guest-backed object's CPU map has no escape this process can take.**
Whether `udmabuf(7)` could manufacture a dma-buf over a guest `memfd` is a genuinely
different question with a `/dev/udmabuf` permission cost that cuts against
*rootless-by-construction*; it is named here so nobody re-derives the dead end, and it is
not this rung.

### 1.2 What the brief got right, verified in `ogkm-580` rather than taken on trust

- **USERD is ingested by PHYSICAL address**, `memdescGetPhysAddr(…, AT_GPU, userdOffset)`
  (`…/fifo/arch/volta/kernel_channel_gv100.c:206-208`). ⇒ unlike the ring, USERD needs no
  VA and no address space, and `R32` deliberately DMA-maps nothing.
- **`bClientAllocatedUserD` is genuinely set on our path** — `kchannelCreateUserdMemDescBc_GV100`
  sets it `NV_TRUE` whenever `phUserdMemory[0] != 0` (`…/kernel_channel_gv100.c:130`), and the
  channel-side `kchannelMap` then refuses (`…/fifo/kernel_channel.c:1291` in 580; the brief's
  `arch/turing/kernel_channel_tu102.c:188` is the **610** tree, which this bench does not run).
- ★ **A citation the brief did not have, and the strongest pre-run evidence there was:** RM
  *explicitly anticipates* this object being an `OS_DESCRIPTOR` —
  `if (dynamicCast(pUserdMemoryRef->pResource, OsDescMemory) != NULL) refAddDependant(pUserdMemoryRef, RES_GET_REF(pKernelChannel));`
  (`…/kernel_channel_gv100.c:249-252`), making the channel depend on the descriptor's
  lifetime. A driver that could not take one would have no reason to write that line.

---

## 2. What was measured

| arm | question | answer |
|---|---|---|
| **establish** | do the two blocks read back the dictated poison *before* RM is told anything? | **yes** — `0xa5d00000+i` at `0x0`, `0x5b0b0000+i` at `0x8000` |
| **A — ingest** | will RM build a channel over a caller-supplied `OS_DESCRIPTOR` USERD? | **`NV_OK`**, token `0x4` |
| **residual** | does RM's internal `memmgrMemSet` land on those bytes? | **ZEROED**, all 512 |
| **C — control** | does the zeroing follow the `userdOffset` we dictate? | **yes** — `0x8000` zeroed, `0x0` left carrying its own poison |
| **no-cpu-map** | how many CPU mappings did building the channel ask RM for? | **exactly 1** — the ring, which is ours on this rung |
| **cursors** | what does `userd_cursors` answer? | `USERD_NOT_OURS`, by name |
| **B — map** | can this process CPU-map the caller's USERD through RM? | **refused `0x56`** |

★ **The establishing read is not ceremony.** A fresh `memfd` reads as **zero**, so
*"the block is zero after the alloc"* is true of a run in which RM did nothing at all —
`R25`'s `OsDescSeed::Never` trap, one object over. Without it every other arm is vacuous,
and the rung refuses to report any of them if it fails.

### 2.1 The negative control, and why it is a proposition rather than a repetition

`R30`'s standard is *"a proposition the target can evaluate"*, with a fail-arm that returns
**the other direction's pattern** rather than zeros. `R32`'s control is the same alloc, the
same object, and **one number changed**: `userdOffset = 0x8000` instead of `0`. Both blocks
are re-poisoned first and re-read, so arm C is about known bytes.

The two propositions **cannot both hold**. Arm A says *the block at `0x0` was zeroed*; arm C
says *the block at `0x8000` was zeroed and `0x0` still carries `0xa5d00000+i`*. A readback
addressing the wrong block, or an effect that is not scoped to the number we named, shows up
as agreement between the two arms — and the rung prints the **pattern it found**
(`is_poison(base)` distinguishes `0xa5d00000+i` from `0x5b0b0000+i` from zero from a partial
write), not a boolean.

⊘ Every address in the control is dictated by the rung: one `memfd` it created, two offsets
(`0x0`, `0x8000`) no other rung names, two per-word patterns chosen so that a mix-up reads as
*the other pattern*.

### 2.2 ★ The host kernel's own word, and the discriminator it closes

`dmesg` readability was asserted **positively and first** — rc `0`, 920 lines, 192 of them
already containing `NVRM` — so *"no new line"* could not be an unreadable instrument. Exactly
**one** new host kernel line appeared across the whole run:

```
[42628.718057] NVRM: memMap_IMPL: CPU mapping not supported for addressSpace: 0x1
```

That is the driver naming **arm B's** refusal itself (`rmapi/mapping_cpu.c:187-188`). ⇒ the
refusal path was **reached**, so `0x56` here is a measurement and not an inference. And the
**absence** of any `kfifoSetupUserD_GM107` assert line is the corroborating half of
`residual = ZEROED`: the memset did not merely appear to work, it also did not print the
failure it would have printed.

---

## 3. ★★★ What this changes — and the brief predicted it backwards

> *"`NV_OK` ⇒ the internal map path is fine, the client CPU map is **pure choice**, and the
> cursor-bridge step collapses into a mapping."*

**The same run returned `NV_OK` from the alloc AND `0x56` from the CPU map.** The bridge does
**not** collapse into a mapping — RM will never hand this process a CPU view of a guest-backed
USERD, and `mapping_cpu.c:170-191` is a policy on the *object*, not on the *use*.

★ **What actually collapses is the SECOND CURSOR.** Because `hUserdMemory[0]` may be the
guest's own block, a shadow channel need not keep two `GP_PUT`s in step: there is one, in
guest RAM, and the party that writes it is whoever already holds those pages — on the
production path the isolate's own `GuestRamPlane` view, which needs no RM mapping at all.
That is UVM's shape exactly: it allocates and runs a channel with **no CPU mapping of USERD**
and drives the cursors another way (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:6089-6098`,
`:6235-6236`).

⊘ This is a place where **diverging from the C is correct**. The C could CPU-map anything it
liked because it emulated the framebuffer and redirected the guest's view; that reason does
not exist for us, and inheriting *"map USERD, then write `GP_PUT` through it"* would inherit a
step whose justification was the C's architecture rather than the driver's behaviour.

### 3.1 ⚠⚠ AND A HAZARD THIS RUNG DISCOVERED — invisible to the commissioned instrument

`residual = ZEROED` is not only good news. If the object handed in is the guest's **live**
USERD, **RM wipes the guest's own cursors at channel-alloc time** — 512 bytes, silently, with
the alloc still returning `NV_OK`.

`guest_ring_adoption.md` §3 puts the host channel's birth at the **first doorbell**, which is
precisely the moment *after* the guest has written `GP_PUT`. ⇒ adopting the guest's USERD
requires one of:

1. a birth ordered **before** the guest's first cursor write (which §3 measured is *not*
   forced by the ring — `R31` arm C showed `gpFifoOffset` is not resolved at alloc time — so
   this is now a constraint imposed by **USERD**, not by the queue); or
2. a **shadow** USERD block that is not the guest's live one, with the cursor copied in —
   i.e. the bridge stays, and the thing that is cheap is that it is a copy of one word rather
   than a mapping; or
3. re-writing the guest's `GP_PUT` into the block immediately after the alloc, which is
   legitimate only if nothing consumed the ring in between.

⊘ None of these is decided here. What is decided is that *"hand RM the guest's USERD and the
cursors take care of themselves"* is **false**, and the commissioned decision rule would have
scored this `NV_OK` as *"fine"*.

---

## 4. The code, and the census that caught a real conflation

- `GuestUserd` / `UserdSource` — `hUserdMemory[0]` and `userdOffset[0]` become the caller's,
  exactly as `GuestRing` made `gpFifoOffset` / `gpFifoEntries` the caller's. Every existing
  caller passes `UserdSource::Ours` and its behaviour is byte-identical.
- `USERD_NOT_OURS` — `userd_cursors` and `userd_store_u32` refuse by name, and `submit_entry`
  refuses **early**: it writes the two `GP_ENTRY` dwords **before** it touches the cursor, so
  a late refusal would leave the ring mutated and the cursor unmoved — a half-submission
  whose other half the guest owns.
- ⊘ **No production path changed.** `RmBackend::alloc_channel`, the doorbell path, the
  executor and the CE path are untouched.

★★ **A green census caught the mistake, and it was watched failing.** The first cut expressed
USERD provenance with `RingOwner`. Clippy was clean, the whole non-GPU suite passed, and
**one row** of `crates/kayfabe-isolate-host/tests/guest_ring_census.rs` went from **5 to 8** —
a ruling about the GPFIFO silently quantifying over a different object. The available mistake
was to bump the number to 8. ⇒ `UserdOwner` is its own type, and USERD has its own census
(`the_userd_provenance_is_per_channel_and_stays_that_way`, 3 rows), whose sharpest row is
`userd_offset_0: userd_offset` pinned at **one** site: a re-spelled `0` there is correct on
every channel this file allocates and wrong **only** on the guest's, with no error anywhere —
the C's `M5.47` failure shape one object over.

---

## 5. Re-running it

```sh
# on the bench, as root, no guest and no QEMU. Assert the INNER status, not the wrapper's.
cd /workspace/kayfabe_w233
KAYFABE_GIT_SHA=$(git -C <the source tree> rev-parse HEAD) \
  cargo build --release -p kayfabe-isolate-host --bin kayfabe-rm-ladder; echo "BUILD_RC=$?"
# ⊘ the bench tree has no `.git`, so the stamp is HAND-PASSED — check the binary's CONTENT:
strings target/release/kayfabe-rm-ladder | grep -c 'R32 guest userd'   # must be >= 1

XID_OUTDIR=/workspace/bench scripts/bench/host_xid_watch.sh r32_guest_userd -- \
  ./target/release/kayfabe-rm-ladder --guest-userd

# the regression set (each allocates real RM objects; three make a real CE retire)
for f in --osdesc-probe --dictated-ring --guest-ram-pin --executor-vas --guest-ring-channel ''; do
  ./target/release/kayfabe-rm-ladder $f; done
```

---

## 6. Evidence, by file

| file | what it is |
|---|---|
| `traces/real_ga106/rmladder_r32_guest_userd_real_ga106.txt` | the run, the dmesg delta, the four verdicts, and the regression appendix |
| `crates/kayfabe-isolate-host/src/rm.rs` | `GuestUserd`, `UserdSource`, `UserdOwner`, `USERD_NOT_OURS`, `GuestUserdEvidence`, `UserdBlock`, `prove_guest_userd` |
| `crates/kayfabe-isolate-host/src/bin/rmladder.rs` | `guest_userd_probe`, `--guest-userd` |
| `crates/kayfabe-isolate-host/tests/guest_ring_census.rs` | `USERD_SURFACE` + `the_userd_provenance_is_per_channel_and_stays_that_way` |

### 6.1 ⚠ `cargo test --workspace` is RED on `origin/master`, and it is not this rung's

`-p kayfabe-tests --test unranked_locks` fails because `Mutex<u64>` in
`crates/kayfabe-qemu-raw/src/shim.rs` is unclassified. Bisected:

```
git show 11599d9:crates/kayfabe-qemu-raw/src/shim.rs | grep -c 'Mutex<u64>'   # 0
git show 6fcedac:crates/kayfabe-qemu-raw/src/shim.rs | grep -c 'Mutex<u64>'   # 1
```

⇒ it arrived at `6fcedac` (`w232`) and `origin/master` `b6c5442` ships it red — the **third**
instance of the shape that test's own docstring records (*"FOUND BY THIS GATE … and it had
ALREADY SHIPPED"*). 2451 other tests pass. It is deliberately **not** fixed here: classifying
it asserts whether a blocking call may run beneath that lock, which is the `w232` author's
knowledge. Inventing that answer is exactly how a green test comes to hold a wall in place.
